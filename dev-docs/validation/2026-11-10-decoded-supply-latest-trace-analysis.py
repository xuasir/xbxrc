"""按会话、renderer 代际和实际效果复算 Browser 帧供给；保留物理上屏证据边界。"""

import argparse
import hashlib
import json
import math
import subprocess
from collections import Counter
from datetime import datetime
from pathlib import Path
from zoneinfo import ZoneInfo


def wall(ms):
    return datetime.fromtimestamp(ms / 1000, ZoneInfo('Asia/Shanghai')).isoformat(timespec='milliseconds')


def quantile(values, q):
    values = sorted(v for v in values if isinstance(v, (int, float)))
    return values[max(0, math.ceil(len(values) * q) - 1)] if values else None


def summarize(values):
    values = [v for v in values if isinstance(v, (int, float))]
    return {'samples': len(values), 'min': min(values, default=None), 'p95': quantile(values, .95), 'max': max(values, default=None)}


def source_fingerprint(root):
    # 与 src-tauri/build.rs 同一输入集合、排序和字节编码。
    inputs = ['src', 'src-tauri/src', 'src-tauri/build.rs', 'src-tauri/tauri.conf.json',
              'src-tauri/Cargo.toml', 'crates', 'Cargo.toml', 'app-edition.json',
              '.cargo/config.toml', 'Cargo.lock', 'rust-toolchain.toml', 'package.json',
              'pnpm-lock.yaml', 'vite.config.ts', 'tsconfig.json', 'tsconfig.node.json', 'tsconfig.app.json']
    paths = sorted(set(p for p in subprocess.check_output(
        ['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard', '--', *inputs], cwd=root
    ).decode().split('\0') if p))
    digest = hashlib.sha256()
    for relative in paths:
        name = relative.encode()
        digest.update(len(name).to_bytes(8, 'little'))
        digest.update(name)
        try:
            data = (root / relative).read_bytes()
        except FileNotFoundError:
            digest.update(bytes([0]))
            continue
        digest.update(bytes([1]))
        digest.update(len(data).to_bytes(8, 'little'))
        digest.update(data)
    return {'sha256': digest.hexdigest(), 'files': len(paths)}


def raw_window(stats, start, end):
    selected = [r for r in stats if (r['payload'].get('delta') or {}).get('elapsedMs', 0) > 0
                and start <= r['payload']['sampledAtMs'] - r['payload']['delta']['elapsedMs']
                and r['payload']['sampledAtMs'] <= end]
    elapsed = sum(r['payload']['delta']['elapsedMs'] for r in selected)
    result = {'samples': len(selected), 'coverageMs': elapsed}
    keys = ['framesReceivedDelta', 'framesDecodedDelta', 'framesDroppedDelta', 'bytesReceivedDelta',
            'packetsLostDelta', 'packetsReceivedDelta', 'nackCountDelta', 'pliCountDelta', 'firCountDelta',
            'keyFramesDecodedDelta', 'retransmittedPacketsReceivedDelta', 'freezeCountDelta',
            'totalFreezesDurationDelta', 'pauseCountDelta']
    for key in keys:
        values = [r['payload']['delta'].get(key) for r in selected]
        result[key] = sum(v or 0 for v in values) if any(v is not None for v in values) else None
    if elapsed and result['framesDecodedDelta']:
        result['receivedFps'] = result['framesReceivedDelta'] * 1000 / elapsed
        result['decodedFps'] = result['framesDecodedDelta'] * 1000 / elapsed
        result['bitrateMbps'] = result['bytesReceivedDelta'] * 8 / elapsed / 1000
        result['decodeMsAvg'] = sum(r['payload']['delta'].get('totalDecodeTimeDelta', 0) for r in selected) * 1000 / result['framesDecodedDelta']
        emitted = sum(r['payload']['delta'].get('jitterBufferEmittedCountDelta', 0) for r in selected)
        result['jitterBufferMsAvg'] = sum(r['payload']['delta'].get('jitterBufferDelayDelta', 0) for r in selected) * 1000 / emitted if emitted else None
    result['rttMs'] = summarize([(r['payload'].get('selectedCandidatePair') or {}).get('currentRoundTripTime', 0) * 1000 for r in selected])
    result['jitterMs'] = summarize([r['payload']['inboundVideo'].get('jitter', 0) * 1000 for r in selected])
    result['receiveEmptyWindows'] = [{'seq': r['seq'], 'atMs': r['payload']['sampledAtMs'], 'elapsedMs': r['payload']['delta']['elapsedMs']}
                                     for r in selected if r['payload']['delta'].get('framesReceivedDelta') == 0]
    return result


def analyze_session(rows, index):
    sid = rows[0]['sessionId']
    stats = [r for r in rows if r['event'] == 'browserWebRtcStatsObserved']
    telemetry = [r for r in rows if r['event'] == 'renderTelemetryObserved']
    resources = [r for r in rows if r['event'] in ('browserMediaResourcesObserved', 'browserMediaResourcesSnapshot')]
    consumed = [r for r in rows if r['event'] == 'browserObservationConsumed']
    by_id = {r['payload']['sampleId']: r for r in stats}
    closed = next(r for r in resources if r['payload'].get('reason') == 'clientClosed')
    points = []
    boundaries = []
    last_state = None
    reader_samples = []
    for r in resources:
        p = r['payload']
        s = p['resources']
        for d in s.get('renderers', []):
            if not isinstance(d.get('realtimeDrawCount'), (int, float)):
                continue
            point = {'at': p['observedAtMs'], 'count': d['realtimeDrawCount'], 'generation': d['generation'],
                     'seq': r['seq'], 'source': 'resources', 'renderer': d}
            points.append(point)
            if d.get('renderFrameSource') == 'worker':
                reader_samples.append({'seq': r['seq'], 'atMs': p['observedAtMs'],
                                       'readerFrames': d.get('decodedFramesReceived'), 'submittedFrames': d['realtimeDrawCount']})
            if s.get('actualPipeline'):
                state = {k: v for k, v in s.items() if k.startswith('actual')}
                identity = (d['generation'], state)
                if identity != last_state:
                    boundaries.append({'at': p['observedAtMs'], 'wall': r['tsMs'], 'seq': r['seq'],
                                       'generation': d['generation'], 'state': state, 'point': point})
                    last_state = identity
    for r in telemetry:
        p = r['payload']
        points.append({'at': p['sampledAtMs'], 'count': p['rendererSubmittedFramesTotal'],
                       'generation': p['rendererGeneration'], 'seq': r['seq'], 'source': 'cached'})
        d = p['mediaDiagnostics']
        points.append({'at': d['observedAtMs'], 'count': d['submittedFramesTotal'],
                       'generation': d['rendererGeneration'], 'seq': r['seq'], 'source': 'live'})
    points.sort(key=lambda p: (p['at'], p['count']))
    segments = []
    for i, boundary in enumerate(boundaries):
        end = boundaries[i + 1] if i + 1 < len(boundaries) else {
            'at': closed['payload']['observedAtMs'], 'wall': closed['tsMs'], 'seq': closed['seq']}
        start_ms, end_ms = boundary['at'], end['at']
        generation = boundary['generation']
        selected_points = [p for p in points if start_ms <= p['at'] <= end_ms and p['generation'] == generation]
        first, last = boundary['point'], selected_points[-1]
        own = [r for r in telemetry if start_ms <= r['payload']['sampledAtMs'] <= end_ms
               and r['payload']['rendererGeneration'] == generation]
        duration = last['at'] - first['at']
        state = boundary['state']
        label = 'SR开关开启（同尺寸跳过重建）' if state['actualPipeline'] == 'webgl2_sr' else '原画'
        if state.get('actualSharpness', 0) > 0:
            label += f" + {state['actualProcessing'].upper()} {state['actualSharpness']}"
        segment = {'index': i + 1, 'label': label, 'generation': generation, 'state': state,
                   'startSeq': boundary['seq'], 'endSeq': end['seq'], 'startMs': start_ms, 'endMs': end_ms,
                   'startTime': wall(boundary['wall']), 'endTime': wall(end['wall']), 'durationMs': end_ms - start_ms,
                   'raw': raw_window(stats, start_ms, end_ms),
                   'submission': {'startSeq': first['seq'], 'endSeq': last['seq'], 'coverageMs': duration,
                                  'frames': last['count'] - first['count'], 'fps': (last['count'] - first['count']) * 1000 / duration}}
        if len(own) >= 2:
            a, b = own[0]['payload'], own[-1]['payload']
            elapsed = b['sampledAtMs'] - a['sampledAtMs']
            ra, rb = by_id[a['sampleId']]['payload']['inboundVideo'], by_id[b['sampleId']]['payload']['inboundVideo']
            paired = {'startSeq': own[0]['seq'], 'endSeq': own[-1]['seq'], 'fromSampleId': a['sampleId'], 'toSampleId': b['sampleId'],
                      'coverageMs': elapsed, 'receivedFrames': rb['framesReceived'] - ra['framesReceived'],
                      'decodedFrames': rb['framesDecoded'] - ra['framesDecoded'],
                      'submittedFrames': b['rendererSubmittedFramesTotal'] - a['rendererSubmittedFramesTotal']}
            paired['decodeSubmitDeficitFrames'] = paired['decodedFrames'] - paired['submittedFrames']
            for k in ('received', 'decoded', 'submitted'):
                paired[k + 'Fps'] = paired[k + 'Frames'] * 1000 / elapsed
            paired['counterGapLevels'] = summarize([by_id[r['payload']['sampleId']]['payload']['inboundVideo']['framesDecoded']
                                                    - r['payload']['rendererSubmittedFramesTotal'] for r in own])
            segment['pairedSupply'] = paired
        short_by_id = {}
        for r in consumed:
            p = r['payload']
            s = p.get('facts', {}).get('windows', {}).get('short', {})
            if (s.get('rendererCoverageMs', 0) >= 2999 and s.get('decodeCoverageMs', 0) >= 2999
                    and start_ms <= p['sampledAtMs'] - 3000 and p['sampledAtMs'] <= end_ms
                    and p.get('rendererGeneration') == generation):
                short_by_id[p['sampleId']] = {'seq': r['seq'], 'sampleId': p['sampleId'], 'atMs': p['sampledAtMs'], **s}
        shorts = list(short_by_id.values())
        segment['steadySupply'] = {'full3sSamples': len(shorts),
                                   'submitFps': summarize([s['rendererSubmitFpsAvg'] for s in shorts]),
                                   'decodeSubmitFpsDelta': summarize([s['decodeFpsAvg'] - s['rendererSubmitFpsAvg'] for s in shorts]),
                                   'worst3s': min(shorts, key=lambda s: s['rendererSubmitFpsAvg']) if shorts else None}
        intervals = [(a, b) for a, b in zip(telemetry, telemetry[1:])
                     if start_ms <= a['payload']['observedAtMs'] < b['payload']['observedAtMs'] <= end_ms
                     and a['payload']['rendererGeneration'] == b['payload']['rendererGeneration'] == generation]
        cadence_ms = sum(b['payload']['observedAtMs'] - a['payload']['observedAtMs'] for a, b in intervals)
        cadence = {'coverageMs': cadence_ms}
        for key in ('callbackCountSinceLastSample', 'presentedFramesAdvancedSinceLastSample',
                    'callbackGapCountSinceLastSample', 'presentedFramesJumpCountSinceLastSample'):
            cadence[key] = sum(b['payload'].get(key, 0) for a, b in intervals)
        if cadence_ms:
            cadence['callbackFps'] = cadence['callbackCountSinceLastSample'] * 1000 / cadence_ms
            cadence['videoPresentProxyFps'] = cadence['presentedFramesAdvancedSinceLastSample'] * 1000 / cadence_ms
        cadence['maxCallbackIntervalMs'] = max((b['payload']['maxCallbackIntervalMsSinceLastSample'] for a, b in intervals), default=None)
        segment['rvfcDiagnostic'] = cadence
        segment['localSubmitAgeMs'] = summarize([r['payload'].get('rendererSubmitAgeMs') for r in own])
        # 256 项环形窗口必须在当前效果段内填满，才统计该段的分位数。
        timing_points = [{'seq': r['seq'], 'count': r['payload']['rendererSubmittedFramesTotal'], 'timing': r['payload']['rendererTiming']}
                         for r in own]
        timing_points += [{'seq': p['seq'], 'count': p['count'], 'timing': p['renderer']['timing']}
                          for p in selected_points if p['source'] == 'resources']
        qualified = [p for p in timing_points if p['count'] - first['count'] >= 256]
        segment['retainedTimingWindows'] = {'qualifiedSnapshots': len(qualified), 'capacity': 256}
        for key in ('duration', 'upload', 'submit', 'interval'):
            segment['retainedTimingWindows'][key] = {
                'p95AcrossWindowsMs': summarize([p['timing'][key].get('p95Ms') for p in qualified]),
                'maxAcrossWindowsMs': max((p['timing'][key].get('maxMs', 0) for p in qualified), default=None)}
        resource_points = [p for p in selected_points if p['source'] == 'resources']
        if resource_points:
            final_d = resource_points[-1]['renderer']
            segment['finalCounters'] = {k: final_d.get(k) for k in ['decodedFramesReceived', 'realtimeDrawCount',
                                        'latestOverwriteCount', 'rateLimitDropCount', 'submitFailureCount', 'duplicateDrawCount']}
            n = final_d['realtimeDrawCount'] - first['count']
            segment['cpuCostDelta'] = {'frames': n}
            for key in ('drawDurationTotalMs', 'uploadDurationTotalMs', 'submitDurationTotalMs'):
                difference = final_d[key] - first['renderer'][key]
                segment['cpuCostDelta'][key] = difference
                segment['cpuCostDelta'][key.replace('TotalMs', 'MsAvg')] = difference / n if n else None
        segment['rendererSources'] = dict(Counter(r['payload'].get('rendererFrameSource') for r in own))
        segment['renderCauses'] = dict(Counter(r['payload'].get('renderCause') for r in own))
        segments.append(segment)
    final = stats[-1]['payload']['inboundVideo']
    timeline = [{'seq': r['seq'], 'atMs': r['payload']['observedAtMs'],
                 'kind': r['payload']['kind'], 'state': r['payload'].get('connectionState')}
                for r in rows if r['event'] == 'browserWebRtcTimelineObserved']
    connected = next(v for v in timeline if v['kind'] == 'connectionStateChanged' and v['state'] == 'connected')
    commit = boundaries[0]
    reader_differences = [v for v in reader_samples if v['readerFrames'] != v['submittedFrames']]
    first_tel, last_tel = telemetry[0]['payload'], telemetry[-1]['payload']
    first_rtc = by_id[first_tel['sampleId']]['payload']['inboundVideo']
    last_rtc = by_id[last_tel['sampleId']]['payload']['inboundVideo']
    measured_supply = {'fromSampleId': first_tel['sampleId'], 'toSampleId': last_tel['sampleId'],
                       'coverageMs': last_tel['sampledAtMs'] - first_tel['sampledAtMs'],
                       'decodedFrames': last_rtc['framesDecoded'] - first_rtc['framesDecoded'],
                       'submittedFrames': last_tel['rendererSubmittedFramesTotal'] - first_tel['rendererSubmittedFramesTotal']}
    measured_supply['decodeSubmitDeficitFrames'] = measured_supply['decodedFrames'] - measured_supply['submittedFrames']
    for key in ('decoded', 'submitted'):
        measured_supply[key + 'Fps'] = measured_supply[key + 'Frames'] * 1000 / measured_supply['coverageMs']
    monotonic_failures = []
    for generation in {p['generation'] for p in points}:
        same_generation = [p for p in points if p['generation'] == generation]
        monotonic_failures += [{'fromSeq': a['seq'], 'toSeq': b['seq']} for a, b in zip(same_generation, same_generation[1:]) if b['count'] < a['count']]
    return {'index': index, 'sessionId': sid, 'startTime': wall(rows[0]['tsMs']), 'endTime': wall(rows[-1]['tsMs']),
            'stats': len(stats), 'telemetry': len(telemetry), 'segments': segments,
            'allPlaybackRaw': raw_window(stats, boundaries[0]['at'], closed['payload']['observedAtMs']),
            'measuredSupply': measured_supply,
            'traceProducerMax': {key: max(r['payload']['trace'].get(key, 0) for r in telemetry)
                for key in ['dropped', 'droppedEssential', 'queueFull', 'ipcFailed', 'ipcTimedOut', 'rowOversized', 'ackMissing']},
            'readerSupply': {'snapshots': len(reader_samples), 'counterDifferences': reader_differences, 'last': reader_samples[-1]},
            'startup': {'connectedSeq': connected['seq'], 'connectedAtMs': connected['atMs'], 'committedSeq': commit['seq'],
                        'committedAtMs': commit['at'], 'connectedToCommitMs': commit['at'] - connected['atMs']},
            'finalInboundCounters': {k: final.get(k) for k in ['framesReceived', 'framesDecoded', 'framesDropped', 'packetsLost',
                'packetsReceived', 'nackCount', 'pliCount', 'firCount', 'freezeCount', 'totalFreezesDuration', 'pauseCount']},
            'cleanEndingResources': {k: closed['payload']['resources'].get(k) for k in ['currentRenderers', 'pendingRenderers',
                'decodedFrameReaders', 'retainedDecodedFrames', 'frameRequests', 'nativeFrameCallbacks', 'animationCallbacks',
                'videoElements', 'audioElements', 'pixelProbeGpuReads', 'readyTimers']},
            'assertions': {'rawSampleIdsUnique': len(by_id) == len(stats), 'monotonicSubmission': not monotonic_failures,
                           'monotonicFailures': monotonic_failures, 'readerEqualsSubmitted': not reader_differences}}


def analyze(path, source, root, behavior_paths, verification_path):
    rows = [json.loads(s) for s in path.read_text().splitlines() if s.strip()]
    sessions = list(dict.fromkeys(r['sessionId'] for r in rows if r['sessionId']))
    result = {'source': source, 'snapshot': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
              'rows': len(rows), 'bytes': path.stat().st_size, 'timeRange': [wall(rows[0]['tsMs']), wall(rows[-1]['tsMs'])],
              'profileCounts': dict(Counter(r.get('traceProfile') for r in rows)),
              'traceBudgetNoticeCount': sum(r['event'] == 'traceBudgetNotice' for r in rows),
              'build': next(r['payload']['buildFingerprint'] for r in rows if r['event'] == 'runtimeBuildInfo'),
              'sessions': [analyze_session([r for r in rows if r['sessionId'] == sid], i + 1) for i, sid in enumerate(sessions)],
              'anomalies': [], 'browserReports': [],
              'boundaries': ['RTC decode and cached submit deltas pair by sessionId/sampleId/sampledAtMs.',
                             'Worker reader counters and renderer counters pair inside the same live resource snapshot.',
                             'RVFC accepted/decodedFrame identity counters describe presentation diagnostics only.',
                             'Fully covered 3s windows stay in the same actual effect segment and renderer generation.',
                             '256-frame CPU/interval P95 windows are retained snapshots; overlapping windows do not establish a global P95.',
                             'GPU timing is unavailable; screenPresentState is unavailable. Submit age is local last-submit age.',
                             'VideoFrame timestamp remains zero in this WKWebView trace; no decode-frame age or scanout latency can be reconstructed.',
                             'Browser ReferenceChain/cleanAnchorCommitted/DisplayStable and per-request NACK effectiveness are capability-limited.']}
    for r in rows:
        if r['event'] == 'browserWebRtcStatsObserved' and (r['payload'].get('delta') or {}).get('freezeCountDelta', 0) > 0:
            p = r['payload']
            result['anomalies'].append({'event': 'freeze', 'sessionId': r['sessionId'], 'seq': r['seq'], 'time': wall(r['tsMs']),
                'atMs': p['sampledAtMs'], 'delta': p['delta'], 'rttMs': p['selectedCandidatePair'].get('currentRoundTripTime', 0) * 1000,
                'jitterMs': p['inboundVideo'].get('jitter', 0) * 1000})
        if r['event'] == 'renderTelemetryObserved' and r['payload'].get('renderCause') != 'renderStable':
            p = r['payload']
            result['anomalies'].append({'event': 'renderCause', 'sessionId': r['sessionId'], 'seq': r['seq'],
                'time': wall(r['tsMs']), **{k: p.get(k) for k in ['sampleId', 'sampledAtMs', 'renderCause', 'decodeFps',
                    'rendererSubmitFps', 'callbackFps', 'rendererSubmitAgeMs', 'rendererFrameSource']}})
    if root:
        result['currentSource'] = source_fingerprint(root)
        result['currentSource']['matchesRuntime'] = result['currentSource']['sha256'] == result['build']['sourceFingerprintSha256']
    if verification_path:
        result['sourceVerification'] = json.loads(verification_path.read_text())
    for p in behavior_paths:
        report = json.loads(p.read_text())
        stability = report['browserRuntimeStability']
        result['browserReports'].append({'input': str(p), 'behaviorGate': report['browserWebRtcBehaviorGate'],
            'failures': report['failures'], 'stability': {k: stability.get(k) for k in ['gate', 'failures', 'missingEvidence', 'evidence']}})
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('trace', type=Path)
    parser.add_argument('--source', required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--source-root', type=Path)
    parser.add_argument('--source-verification', type=Path)
    parser.add_argument('--browser-report', type=Path, action='append', default=[])
    args = parser.parse_args()
    data = analyze(args.trace, args.source, args.source_root, args.browser_report, args.source_verification)
    args.output.write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n')
    for session in data['sessions']:
        for segment in session['segments']:
            print(json.dumps({'session': session['index'], 'segment': segment['index'], 'label': segment['label'],
                'durationMs': segment['durationMs'], 'submission': segment['submission'], 'paired': segment.get('pairedSupply'),
                'worst3s': segment['steadySupply']['worst3s'], 'rvfc': segment['rvfcDiagnostic'],
                'localSubmitAgeMs': segment['localSubmitAgeMs'], 'timing': segment['retainedTimingWindows']}, ensure_ascii=False))
        print(json.dumps({'session': session['index'], 'reader': session['readerSupply'], 'raw': session['allPlaybackRaw'],
                          'startup': session['startup'], 'assertions': session['assertions']}, ensure_ascii=False))
