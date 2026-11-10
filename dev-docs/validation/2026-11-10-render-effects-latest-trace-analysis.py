import argparse
import hashlib
import json
import math
from collections import Counter
from datetime import datetime
from pathlib import Path
from zoneinfo import ZoneInfo


def wall(ms):
    return datetime.fromtimestamp(ms / 1000, ZoneInfo('Asia/Shanghai')).isoformat(timespec='milliseconds')


def quantile(values, q):
    values = sorted(v for v in values if isinstance(v, (int, float)))
    return values[max(0, math.ceil(len(values) * q) - 1)] if values else None


def analyze(path):
    rows = [json.loads(line) for line in path.read_text().splitlines() if line.strip()]
    stats = [r for r in rows if r['event'] == 'browserWebRtcStatsObserved']
    telemetry = [r for r in rows if r['event'] == 'renderTelemetryObserved']
    resources = [r for r in rows if r['event'] in ('browserMediaResourcesObserved', 'browserMediaResourcesSnapshot')]
    consumed = [r for r in rows if r['event'] == 'browserObservationConsumed']
    raw_by_id = {r['payload']['sampleId']: r for r in stats}
    assert len(raw_by_id) == len(stats)
    closed = next(r for r in resources if r['payload'].get('reason') == 'clientClosed')
    points = []
    for r in resources:
        for d in r['payload']['resources'].get('renderers', []):
            if not isinstance(d.get('realtimeDrawCount'), (int, float)):
                continue
            points.append({'at': r['payload']['observedAtMs'], 'count': d['realtimeDrawCount'],
                           'generation': d['generation'], 'seq': r['seq'], 'source': 'resources', 'renderer': d})
    for r in telemetry:
        p = r['payload']
        points.append({'at': p['sampledAtMs'], 'count': p['rendererSubmittedFramesTotal'],
                       'generation': p['rendererGeneration'], 'seq': r['seq'], 'source': 'cached'})
        d = p['mediaDiagnostics']
        points.append({'at': d['observedAtMs'], 'count': d['submittedFramesTotal'],
                       'generation': d['rendererGeneration'], 'seq': r['seq'], 'source': 'live'})
    points.sort(key=lambda v: (v['at'], v['count']))
    assert len({p['generation'] for p in points}) == 1
    assert all(a['count'] <= b['count'] for a, b in zip(points, points[1:]))

    boundaries = []
    last_state = None
    for r in resources:
        p = r['payload']; s = p['resources']
        if not s.get('actualPipeline'):
            continue
        state = {k: s.get(k) for k in ('actualPipeline', 'actualSharpness', 'actualProcessing', 'actualSuperResolutionActive',
                                      'actualBrightness', 'actualContrast', 'actualSaturation', 'actualTargetFps')}
        if state != last_state:
            boundaries.append({'at': p['observedAtMs'], 'wall': r['tsMs'], 'seq': r['seq'], 'state': state,
                               'count': s['renderers'][0]['realtimeDrawCount']})
            last_state = state

    def raw_window(start, end):
        selected = [r for r in stats if (r['payload'].get('delta') or {}).get('elapsedMs', 0) > 0
                    and start <= r['payload']['sampledAtMs'] - r['payload']['delta']['elapsedMs']
                    and r['payload']['sampledAtMs'] <= end]
        elapsed = sum(r['payload']['delta']['elapsedMs'] for r in selected)
        out = {'samples': len(selected), 'coverageMs': elapsed}
        keys = ('framesReceivedDelta', 'framesDecodedDelta', 'framesDroppedDelta', 'bytesReceivedDelta', 'packetsLostDelta',
                'packetsReceivedDelta', 'nackCountDelta', 'pliCountDelta', 'firCountDelta', 'keyFramesDecodedDelta',
                'retransmittedPacketsReceivedDelta', 'freezeCountDelta', 'totalFreezesDurationDelta', 'pauseCountDelta')
        for k in keys:
            vs = [r['payload']['delta'].get(k) for r in selected]
            out[k] = sum(v or 0 for v in vs) if any(v is not None for v in vs) else None
        if elapsed:
            out['receivedFps'] = out['framesReceivedDelta'] * 1000 / elapsed
            out['decodedFps'] = out['framesDecodedDelta'] * 1000 / elapsed
            out['bitrateMbps'] = out['bytesReceivedDelta'] * 8 / elapsed / 1000
            out['decodeMsAvg'] = sum(r['payload']['delta'].get('totalDecodeTimeDelta', 0) for r in selected) * 1000 / out['framesDecodedDelta']
        out['rttMsP95'] = quantile([(r['payload'].get('selectedCandidatePair') or {}).get('currentRoundTripTime', 0) * 1000 for r in selected], .95)
        out['jitterMsP95'] = quantile([r['payload']['inboundVideo'].get('jitter', 0) * 1000 for r in selected], .95)
        out['receiveEmptyWindows'] = [{'seq': r['seq'], 'at': r['payload']['sampledAtMs'], 'ms': r['payload']['delta']['elapsedMs'],
                                      'packets': r['payload']['delta'].get('packetsReceivedDelta'),
                                      'decoded': r['payload']['delta'].get('framesDecodedDelta')}
                                     for r in selected if r['payload']['delta'].get('framesReceivedDelta') == 0]
        return out

    segments = []
    for i, start in enumerate(boundaries):
        end = boundaries[i + 1] if i + 1 < len(boundaries) else {'at': closed['payload']['observedAtMs'], 'wall': closed['tsMs'], 'seq': closed['seq']}
        ps = [p for p in points if start['at'] <= p['at'] <= end['at']]
        first = next(p for p in ps if p['seq'] == start['seq'] and p['source'] == 'resources')
        last = ps[-1]
        elapsed = last['at'] - first['at']
        state = start['state']
        sr = state['actualPipeline'] == 'webgl2_sr'
        sharp = state['actualSharpness'] > 0
        label = ('SR同尺寸直通' if sr else '原画') + (' + USM锐化3' if sharp else '')
        seg = {'index': i + 1, 'label': label, 'state': state, 'startSeq': start['seq'], 'endSeq': end['seq'],
               'startMs': start['at'], 'endMs': end['at'], 'startTime': wall(start['wall']), 'endTime': wall(end['wall']),
               'durationMs': end['at'] - start['at'], 'raw': raw_window(start['at'], end['at']),
               'drawWindow': {'startSeq': first['seq'], 'endSeq': last['seq'], 'startMs': first['at'], 'endMs': last['at'],
                              'frames': last['count'] - first['count'], 'coverageMs': elapsed, 'fps': (last['count'] - first['count']) * 1000 / elapsed}}
        own = [r for r in telemetry if start['at'] <= r['payload']['observedAtMs'] <= end['at']]
        pairs = [(a, b) for a, b in zip(telemetry, telemetry[1:]) if start['at'] <= a['payload']['observedAtMs'] < b['payload']['observedAtMs'] <= end['at']]
        duration = sum(b['payload']['observedAtMs'] - a['payload']['observedAtMs'] for a, b in pairs)
        cadence = {'samples': len(pairs), 'coverageMs': duration}
        for k in ('callbackCountSinceLastSample', 'presentedFramesAdvancedSinceLastSample', 'callbackGapCountSinceLastSample',
                  'presentedFramesJumpCountSinceLastSample', 'droppedFramesSinceLastSample'):
            cadence[k] = sum(b['payload'].get(k, 0) for a, b in pairs)
        if duration:
            cadence['callbackFps'] = cadence['callbackCountSinceLastSample'] * 1000 / duration
            cadence['videoPresentProxyFps'] = cadence['presentedFramesAdvancedSinceLastSample'] * 1000 / duration
        cadence['maxCallbackMs'] = max((b['payload']['maxCallbackIntervalMsSinceLastSample'] for a, b in pairs), default=None)
        cadence['trackingSources'] = dict(Counter(r['payload'].get('trackingSource') for r in own))
        cadence['sourceFpsUnavailableReasons'] = dict(Counter(r['payload'].get('sourceFpsUnavailableReason') for r in own))
        cadence['renderCauses'] = dict(Counter(r['payload'].get('renderCause') for r in own))
        cadence['submitAgeMsP95'] = quantile([r['payload'].get('rendererSubmitAgeMs') for r in own if start['at'] <= r['payload']['sampledAtMs']], .95)
        cadence['submitAgeMsMax'] = quantile([r['payload'].get('rendererSubmitAgeMs') for r in own if start['at'] <= r['payload']['sampledAtMs']], 1)
        seg['cadence'] = cadence
        if len(own) >= 2:
            a, b = own[0]['payload']['mediaDiagnostics'], own[-1]['payload']['mediaDiagnostics']
            accepted = {k: b['frameIdentity']['accepted'][k] - a['frameIdentity']['accepted'][k] for k in b['frameIdentity']['accepted']}
            submitted = b['submittedFramesTotal'] - a['submittedFramesTotal']
            assert sum(accepted.values()) == submitted
            seg['liveSupply'] = {'startSeq': own[0]['seq'], 'endSeq': own[-1]['seq'], 'coverageMs': b['observedAtMs'] - a['observedAtMs'], 'accepted': accepted, 'submittedFrames': submitted}
        matched = [r for r in telemetry if start['at'] <= r['payload']['sampledAtMs'] <= end['at']]
        if len(matched) >= 2:
            a, b = matched[0]['payload'], matched[-1]['payload']; elapsed = b['sampledAtMs'] - a['sampledAtMs']
            ia, ib = raw_by_id[a['sampleId']]['payload']['inboundVideo'], raw_by_id[b['sampleId']]['payload']['inboundVideo']
            w = {'startSeq': matched[0]['seq'], 'endSeq': matched[-1]['seq'], 'fromSampleId': a['sampleId'], 'toSampleId': b['sampleId'], 'coverageMs': elapsed,
                 'receivedFrames': ib['framesReceived'] - ia['framesReceived'], 'decodedFrames': ib['framesDecoded'] - ia['framesDecoded'],
                 'submittedFrames': b['rendererSubmittedFramesTotal'] - a['rendererSubmittedFramesTotal'], 'freezeCountDelta': ib['freezeCount'] - ia['freezeCount']}
            w['decodeSubmitDeficitFrames'] = w['decodedFrames'] - w['submittedFrames']
            for k in ('received', 'decoded', 'submitted'):
                w[k + 'Fps'] = w[k + 'Frames'] * 1000 / elapsed
            seg['pairedCachedSupply'] = w
        shorts = {}
        for r in consumed:
            p = r['payload']; s = p.get('facts', {}).get('windows', {}).get('short', {})
            if s.get('rendererCoverageMs', 0) >= 2999 and start['at'] <= p['sampledAtMs'] - 3000 and p['sampledAtMs'] <= end['at']:
                shorts[p['sampleId']] = {'seq': r['seq'], 'sampleId': p['sampleId'], 'at': p['sampledAtMs'], **s}
        ss = list(shorts.values())
        seg['threeSecondSamples'] = len(ss)
        seg['worst3s'] = min(ss, key=lambda s: s['rendererSubmitFpsAvg']) if ss else None
        seg['last3s'] = ss[-1] if ss else None
        tails = [p for p in ps if p['source'] == 'resources' and p['count'] - first['count'] >= 256]
        if tails:
            r = tails[-1]['renderer']; seg['retainedTail'] = {'seq': tails[-1]['seq'], 'timing': r['timing'], 'gpuTiming': r['gpuTiming'],
                                                           'counters': {k: r[k] for k in ('latestOverwriteCount', 'rateLimitDropCount', 'submitFailureCount', 'duplicateDrawCount')}}
        segments.append(seg)
    freeze = [{'seq': r['seq'], 'wall': wall(r['tsMs']), 'at': r['payload']['sampledAtMs'], 'delta': r['payload']['delta']}
              for r in stats if (r['payload'].get('delta') or {}).get('freezeCountDelta', 0) > 0]
    pixels = [r['payload']['mediaDiagnostics']['frameIdentity']['videoPixels'] for r in telemetry]
    return {'source': '/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1794275474604-1.jsonl',
            'snapshot': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest(), 'rows': len(rows),
            'timeRange': [wall(rows[0]['tsMs']), wall(rows[-1]['tsMs'])],
            'build': [r['payload'] for r in rows if r['event'] == 'runtimeBuildInfo'],
            'counts': {'profiles': dict(Counter(r.get('traceProfile') for r in rows)), 'stats': len(stats), 'telemetry': len(telemetry),
                       'commits': sum(r['payload'].get('reason') == 'rendererCommitted' for r in resources), 'actualStateSegments': len(segments),
                       'traceBudgetNotices': sum(r['event'] == 'traceBudgetNotice' for r in rows)},
            'segments': segments, 'allPlaybackRaw': raw_window(boundaries[0]['at'], closed['payload']['observedAtMs']),
            'freezeEvents': freeze, 'finalInboundCounters': {k: stats[-1]['payload']['inboundVideo'].get(k) for k in ('framesReceived', 'framesDecoded', 'framesDropped', 'packetsLost', 'packetsReceived', 'nackCount', 'pliCount', 'firCount', 'freezeCount', 'totalFreezesDuration')},
            'pixels': {'enabledCounts': dict(Counter(p['enabled'] for p in pixels)), 'readCountMax': max(p['readCount'] for p in pixels)},
            'assertions': {'sameGenerationMonotonicSubmission': 'PASS', 'rawSampleIdsUnique': 'PASS', 'acceptedEqualsSubmittedPerMeasuredSegment': 'PASS'},
            'assumptions': ['Segments start at actual renderer resource state transitions, including in-place updates.',
                            'Canvas FPS is cumulative submitted count delta divided by monotonic observation time.',
                            'Three-second windows have full renderer coverage and remain inside one actual state segment.',
                            'Receive/decode/bitrate sum complete raw RTC delta intervals within each segment.',
                            'Paired deficits use identical sample IDs and sampledAtMs for RTC and cached renderer counters.',
                            'GPU completion and physical scanout are unavailable; submit age reports local last-submit age.']}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('trace', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    data = analyze(args.trace)
    args.output.write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n')
    for s in data['segments']:
        print(json.dumps({'segment': s['index'], 'label': s['label'], 'start': s['startTime'], 'end': s['endTime'], 'durationMs': s['durationMs'],
                          'fps': s['drawWindow']['fps'], 'worst3sFps': s['worst3s']['rendererSubmitFpsAvg'] if s['worst3s'] else None,
                          'raw': s['raw'], 'cadence': s['cadence'], 'paired': s.get('pairedCachedSupply')}, ensure_ascii=False))
    print('PLAYBACK', json.dumps(data['allPlaybackRaw'], ensure_ascii=False))
