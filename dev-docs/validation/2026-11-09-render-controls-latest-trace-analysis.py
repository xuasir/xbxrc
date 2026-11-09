import argparse
import hashlib
import json
import math
from collections import Counter, defaultdict
from datetime import datetime
from pathlib import Path
from zoneinfo import ZoneInfo


def wall(ms):
    return datetime.fromtimestamp(ms / 1000, ZoneInfo('Asia/Shanghai')).isoformat(timespec='milliseconds')


def percentile(values, q):
    values = sorted(v for v in values if isinstance(v, (int, float)))
    return values[max(0, math.ceil(len(values) * q) - 1)] if values else None


def measure(rows, path):
    stats = [r for r in rows if r['event'] == 'browserWebRtcStatsObserved']
    telemetry = [r for r in rows if r['event'] == 'renderTelemetryObserved']
    consumed = [r for r in rows if r['event'] == 'browserObservationConsumed']
    resources = [r for r in rows if r['event'] in ('browserMediaResourcesObserved', 'browserMediaResourcesSnapshot')]
    commits = [r for r in resources if r['payload'].get('reason') == 'rendererCommitted']
    closed = next((r for r in resources if r['payload'].get('reason') == 'clientClosed'), None)
    end_at = closed['payload']['observedAtMs'] if closed else stats[-1]['payload']['sampledAtMs']
    policies = [r for r in rows if r['event'] == 'renderPolicyApplied']
    raw_by_id = {r['payload']['sampleId']: r for r in stats}
    assert len(raw_by_id) == len(stats), 'raw sample identities must be unique'

    def raw_window(start, end):
        selected = []
        for r in stats:
            p = r['payload']; d = p.get('delta') or {}; t = p['sampledAtMs']; elapsed = d.get('elapsedMs', 0)
            if elapsed > 0 and t - elapsed >= start and t <= end:
                selected.append(r)
        duration = sum(r['payload']['delta']['elapsedMs'] for r in selected)
        result = {'sampleCount': len(selected), 'coverageMs': duration}
        for k in ('bytesReceivedDelta', 'framesReceivedDelta', 'framesDecodedDelta', 'framesDroppedDelta',
                  'freezeCountDelta', 'totalFreezesDurationDelta', 'pauseCountDelta', 'pliCountDelta',
                  'firCountDelta', 'nackCountDelta', 'packetsLostDelta', 'packetsReceivedDelta',
                  'retransmittedPacketsReceivedDelta', 'keyFramesDecodedDelta'):
            values = [r['payload']['delta'].get(k) for r in selected]
            result[k] = sum(v or 0 for v in values) if any(v is not None for v in values) else None
        if duration:
            result['receivedFps'] = result['framesReceivedDelta'] * 1000 / duration
            result['decodedFps'] = result['framesDecodedDelta'] * 1000 / duration
            result['bitrateMbps'] = result['bytesReceivedDelta'] * 8 / duration / 1000
        decoded = result['framesDecodedDelta']
        result['decodeCostMsAvg'] = sum(r['payload']['delta'].get('totalDecodeTimeDelta', 0) for r in selected) * 1000 / decoded if decoded else None
        for key, object_key, field in (('rttMsP95', 'selectedCandidatePair', 'currentRoundTripTime'), ('jitterMsP95', 'inboundVideo', 'jitter')):
            value = percentile([(r['payload'].get(object_key) or {}).get(field) for r in selected], .95)
            result[key] = value * 1000 if value is not None else None
        return result

    points = defaultdict(list)
    for r in resources:
        p = r['payload']
        for d in (p.get('resources') or {}).get('renderers', []):
            if isinstance(d.get('realtimeDrawCount'), (int, float)):
                points[d['generation']].append({'at': p['observedAtMs'], 'count': d['realtimeDrawCount'], 'seq': r['seq'], 'source': 'resources',
                    'timing': d.get('timing'), 'gpu': d.get('gpuTiming'),
                    'counters': {k: d.get(k) for k in ('latestOverwriteCount', 'rateLimitDropCount', 'submitFailureCount', 'duplicateDrawCount')}})
    for r in telemetry:
        p = r['payload']; d = p.get('mediaDiagnostics') or {}
        if isinstance(p.get('rendererSubmittedFramesTotal'), (int, float)):
            points[p['rendererGeneration']].append({'at': p['sampledAtMs'], 'count': p['rendererSubmittedFramesTotal'], 'seq': r['seq'], 'source': 'cachedStats', 'timing': p.get('rendererTiming'), 'gpu': p.get('rendererGpuTiming')})
        if isinstance(d.get('submittedFramesTotal'), (int, float)):
            points[d['rendererGeneration']].append({'at': d['observedAtMs'], 'count': d['submittedFramesTotal'], 'seq': r['seq'], 'source': 'liveDiagnostics'})

    episodes = []
    for i, commit in enumerate(commits):
        p = commit['payload']; o = p['resources']['rendererObservation']; start = p['observedAtMs']
        end = commits[i + 1]['payload']['observedAtMs'] if i + 1 < len(commits) else end_at
        policy = [r for r in policies if r['payload']['observedAtMs'] <= start][-1]
        generation = o['generation']; kind = o['kind']
        ep = {'generation': generation, 'kind': kind, 'commitSeq': commit['seq'], 'policySeq': policy['seq'],
              'startMs': start, 'endMs': end, 'startTime': wall(commit['tsMs']),
              'endTime': wall(commits[i + 1]['tsMs'] if i + 1 < len(commits) else closed['tsMs']) if closed else None,
              'durationMs': end - start, 'sharpness': policy['payload'].get('sharpness'), 'sr': policy['payload'].get('sr'),
              'sourceDimensions': [o.get('sourceWidth'), o.get('sourceHeight')], 'targetFps': o.get('targetFps'), 'rawStats': raw_window(start, end)}
        same = [r for r in telemetry if r['payload'].get('rendererGeneration') == generation and start <= r['payload']['sampledAtMs'] <= end]
        intervals = []
        for a, b in zip(telemetry, telemetry[1:]):
            x, y = a['payload'], b['payload']
            if x.get('rendererGeneration') == y.get('rendererGeneration') == generation and start <= x['observedAtMs'] < y['observedAtMs'] <= end:
                intervals.append({'ms': y['observedAtMs'] - x['observedAtMs'], 'callbacks': y.get('callbackCountSinceLastSample', 0),
                    'presented': y.get('presentedFramesAdvancedSinceLastSample', 0), 'gaps': y.get('callbackGapCountSinceLastSample', 0),
                    'jumps': y.get('presentedFramesJumpCountSinceLastSample', 0), 'droppedLike': y.get('droppedFramesSinceLastSample', 0),
                    'maxCallbackMs': y.get('maxCallbackIntervalMsSinceLastSample', 0), 'seq': b['seq']})
        ms = sum(v['ms'] for v in intervals)
        ep['telemetryWindows'] = {'count': len(intervals), 'coverageMs': ms,
            'callbackFps': sum(v['callbacks'] for v in intervals) * 1000 / ms if ms else None,
            'videoPresentProxyFps': sum(v['presented'] for v in intervals) * 1000 / ms if ms else None,
            'callbackGaps': sum(v['gaps'] for v in intervals), 'presentedJumps': sum(v['jumps'] for v in intervals),
            'droppedLike': sum(v['droppedLike'] for v in intervals), 'maxCallbackMs': max((v['maxCallbackMs'] for v in intervals), default=None)}
        ep['trackingSources'] = dict(Counter(r['payload'].get('trackingSource') for r in same))
        ep['sourceFpsUnavailableReasons'] = dict(Counter(r['payload'].get('sourceFpsUnavailableReason') for r in same))
        short = []
        for r in consumed:
            z = r['payload']; s = ((z.get('facts') or {}).get('windows') or {}).get('short') or {}
            if z.get('rendererGeneration') == generation and start <= z.get('sampledAtMs', 0) - 3000 and z.get('sampledAtMs', 0) <= end and s.get('rendererCoverageMs', 0) >= 2999:
                short.append({'seq': r['seq'], 'sampleId': z.get('sampleId'), 'sampledAtMs': z.get('sampledAtMs'),
                    **{k: s.get(k) for k in ('rendererSubmitFpsAvg', 'videoPresentFpsAvg', 'callbackFpsAvg', 'inboundFpsAvg', 'decodeFpsAvg', 'rendererSubmitAgeMsAvg')}})
        ep['complete3sWindows'] = short
        ep['min3sRendererFps'] = min((v['rendererSubmitFpsAvg'] for v in short if v['rendererSubmitFpsAvg'] is not None), default=None)
        if kind != 'video':
            ps = sorted((v for v in points[generation] if start <= v['at'] <= end), key=lambda v: (v['at'], v['count']))
            assert all(a['count'] <= b['count'] for a, b in zip(ps, ps[1:])), f'non-monotonic draw counts in generation {generation}'
            first = next(v for v in ps if v['seq'] == commit['seq'] and v['source'] == 'resources'); last = ps[-1]
            for skip, name in ((0, 'fullDrawWindow'), (1000, 'post1sDrawWindow'), (2000, 'post2sDrawWindow')):
                qualifying = [v for v in ps if v['at'] >= start + skip]
                a = first if skip == 0 else qualifying[0] if qualifying else None
                ep[name] = {'startSeq': a['seq'], 'endSeq': last['seq'], 'startMs': a['at'], 'endMs': last['at'], 'frames': last['count'] - a['count'],
                    'coverageMs': last['at'] - a['at'], 'fps': (last['count'] - a['count']) * 1000 / (last['at'] - a['at'])} if a and last['at'] > a['at'] else None
            timing = next((v for v in reversed(ps) if v.get('timing')), None)
            ep['lastTiming'] = timing.get('timing') if timing else None
            ep['lastGpuTiming'] = timing.get('gpu') if timing else None
            counters = [v['counters'] for v in ps if v.get('counters')]
            ep['rendererDropCounters'] = counters[-1] if counters else None
            matched = [r for r in same if r['payload'].get('rendererSubmittedFramesTotal') is not None and r['payload']['sampleId'] in raw_by_id]
            if len(matched) >= 2:
                a, b = matched[0]['payload'], matched[-1]['payload']; ia = raw_by_id[a['sampleId']]['payload']['inboundVideo']; ib = raw_by_id[b['sampleId']]['payload']['inboundVideo']; elapsed = b['sampledAtMs'] - a['sampledAtMs']
                ep['pairedCachedWindow'] = {'fromSampleId': a['sampleId'], 'toSampleId': b['sampleId'], 'startSeq': matched[0]['seq'], 'endSeq': matched[-1]['seq'], 'coverageMs': elapsed,
                    'receivedFrames': ib['framesReceived'] - ia['framesReceived'], 'decodedFrames': ib['framesDecoded'] - ia['framesDecoded'],
                    'submittedFrames': b['rendererSubmittedFramesTotal'] - a['rendererSubmittedFramesTotal'], 'freezeDelta': ib['freezeCount'] - ia['freezeCount']}
                w = ep['pairedCachedWindow']
                for k in ('received', 'decoded', 'submitted'): w[k + 'Fps'] = w[k + 'Frames'] * 1000 / elapsed
                w['decodeSubmitDeficitFrames'] = w['decodedFrames'] - w['submittedFrames']
            ds = [r for r in telemetry if (r['payload'].get('mediaDiagnostics') or {}).get('rendererGeneration') == generation and start <= r['payload']['mediaDiagnostics']['observedAtMs'] <= end]
            if len(ds) >= 2:
                a, b = ds[0]['payload']['mediaDiagnostics'], ds[-1]['payload']['mediaDiagnostics']; x = a['frameIdentity']; y = b['frameIdentity']
                accepted = {k: y['accepted'][k] - x['accepted'][k] for k in y['accepted']}
                submitted = b['submittedFramesTotal'] - a['submittedFramesTotal']
                ep['pairedLiveSupply'] = {'startSeq': ds[0]['seq'], 'endSeq': ds[-1]['seq'], 'coverageMs': b['observedAtMs'] - a['observedAtMs'], 'accepted': accepted, 'submittedFrames': submitted}
                assert submitted == sum(accepted.values()), f'accepted/submit mismatch in generation {generation}'
        episodes.append(ep)

    pixels = [(r['payload'].get('mediaDiagnostics') or {}).get('frameIdentity', {}).get('videoPixels') for r in telemetry]
    pixels = [v for v in pixels if v]
    video = [e['telemetryWindows'] for e in episodes if e['kind'] == 'video' and e['telemetryWindows']['coverageMs'] > 0]
    video_ms = sum(v['coverageMs'] for v in video)
    build = [r['payload'] for r in rows if r['event'] == 'runtimeBuildInfo']
    result = {'trace': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest(), 'rows': len(rows), 'timeRange': [wall(rows[0]['tsMs']), wall(rows[-1]['tsMs'])],
        'build': build, 'traceProfiles': dict(Counter(r.get('traceProfile') for r in rows)), 'counts': {'rawStats': len(stats), 'telemetry': len(telemetry), 'resources': len(resources), 'rendererCommits': len(commits), 'traceBudgetNotices': sum(r['event'] == 'traceBudgetNotice' for r in rows)},
        'episodes': episodes, 'allPlaybackRawStats': raw_window(commits[0]['payload']['observedAtMs'], end_at),
        'videoWeightedProxyFps': sum(v['videoPresentProxyFps'] * v['coverageMs'] for v in video) / video_ms if video_ms else None, 'videoCoverageMs': video_ms,
        'finalInboundCounters': {k: stats[-1]['payload']['inboundVideo'].get(k) for k in ('framesReceived', 'framesDecoded', 'framesDropped', 'freezeCount', 'totalFreezesDuration', 'pauseCount', 'packetsLost', 'nackCount', 'pliCount', 'firCount', 'retransmittedPacketsReceived')},
        'pixels': {'samples': len(pixels), 'enabledCounts': dict(Counter(v.get('enabled') for v in pixels)), 'maxCounters': {k: max((v.get(k, 0) for v in pixels), default=None) for k in ('pixelProbeCanvases', 'pixelProbeGpuReads', 'pixelProbePendingPairs', 'pixelProbePreparations', 'pixelProbeTimers', 'readCount')}},
        'localActions': [{ 'seq': r['seq'], 'tsMs': r['tsMs'], **{k: v for k, v in r['payload'].items() if not isinstance(v, (dict, list))}} for r in rows if r['event'] in ('renderLocalActionPlanned', 'renderLocalActionEffectEvaluated')],
        'endingEvidence': [{'seq': r['seq'], 'event': r['event'], 'payload': r['payload']} for r in rows if r['event'] in ('executionCleanupObserved', 'traceProducerDrainObserved', 'traceFlushObserved')],
        'noSupplyIntervals': [{'seq': r['seq'], 'sampledAtMs': r['payload']['sampledAtMs'], 'delta': {k: r['payload']['delta'].get(k) for k in ('elapsedMs', 'packetsReceivedDelta', 'framesReceivedDelta', 'framesDecodedDelta')}} for r in stats if (r['payload'].get('delta') or {}).get('framesDecodedDelta') == 0],
        'assumptions': ['Canvas FPS comes from monotonic same-generation cumulative submitted counts; startup and post-1s/post-2s windows are separate.', 'Video FPS is presented-frame advancement in full telemetry publication intervals.', 'Raw media averages sum complete RTC delta intervals inside each committed generation.', 'Paired frame deficits use matching cached sample IDs and cumulative RTC counters.', 'Full three-second minima require matching renderer identity and >=2999ms coverage.', 'Browser physical scanout and unavailable GPU timing remain capability-limited.']}
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('trace', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--expected-sha')
    parser.add_argument('--browser-audit', type=Path)
    parser.add_argument('--production-audit', type=Path)
    args = parser.parse_args()
    if args.expected_sha:
        assert hashlib.sha256(args.trace.read_bytes()).hexdigest() == args.expected_sha, 'trace snapshot SHA256 mismatch'
    rows = [json.loads(line) for line in args.trace.read_text().splitlines() if line.strip()]
    data = measure(rows, args.trace)
    if args.browser_audit:
        b = json.loads(args.browser_audit.read_text())
        s = b['browserRuntimeStability']
        data['browserAudit'] = {'behaviorGate': b['browserWebRtcBehaviorGate'], 'stabilityGate': s['gate'], 'failures': b['failures'], 'stabilityFailures': s['failures'], 'evidence': s['evidence']}
    if args.production_audit:
        p = json.loads(args.production_audit.read_text())
        data['productionAudit'] = {k: p[k] for k in ('productionContractVersion', 'gate', 'acceptanceGate', 'sections', 'steadyDurationMs', 'healthRecords', 'devIntervals', 'input', 'traceLoss', 'ending', 'capabilities')}
    args.output.write_text(json.dumps(data, indent=2, ensure_ascii=False) + '\n')
    print(json.dumps({'sha256': data['sha256'], 'episodes': len(data['episodes']), 'pairedSupplyAssertions': 'PASS', 'output': str(args.output)}, ensure_ascii=False))


if __name__ == '__main__':
    main()
