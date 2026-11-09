import argparse
import hashlib
import json
import math
import struct
import subprocess
from collections import Counter
from datetime import datetime
from pathlib import Path
from zoneinfo import ZoneInfo

EXPECTED_TRACE_SHA256 = 'e3c88d94856c299dbbc39a14221000f04354b7864fd7ac3f3a242a16232aa1d7'


def quantile(values, fraction):
    values = sorted(v for v in values if isinstance(v, (int, float)))
    return values[max(0, math.ceil(len(values) * fraction) - 1)] if values else None


def wall(ts):
    return datetime.fromtimestamp(ts / 1000, ZoneInfo('Asia/Shanghai')).isoformat(timespec='milliseconds')


def source_fingerprint(root):
    # 与 src-tauri/build.rs 保持相同输入及长度编码。
    inputs = ['src', 'src-tauri/src', 'src-tauri/build.rs', 'src-tauri/tauri.conf.json',
              'src-tauri/Cargo.toml', 'crates', 'Cargo.toml', 'app-edition.json',
              '.cargo/config.toml', 'Cargo.lock', 'rust-toolchain.toml', 'package.json',
              'pnpm-lock.yaml', 'vite.config.ts', 'tsconfig.json', 'tsconfig.node.json', 'tsconfig.app.json']
    names = sorted(set(subprocess.check_output(
        ['git', '-C', str(root), 'ls-files', '-z', '--cached', '--others', '--exclude-standard', '--', *inputs]
    ).decode().rstrip('\0').split('\0')))
    h = hashlib.sha256()
    for name in names:
        encoded = name.encode()
        h.update(struct.pack('<Q', len(encoded)))
        h.update(encoded)
        try:
            data = (root / name).read_bytes()
        except FileNotFoundError:
            h.update(b'\0')
            continue
        h.update(b'\1')
        h.update(struct.pack('<Q', len(data)))
        h.update(data)
    return {'sha256': h.hexdigest(), 'inputFiles': len(names)}


def build_matrix(rows, trace_path):
    TRACE = trace_path
    stats = [r for r in rows if r['event'] == 'browserWebRtcStatsObserved']
    telemetry = [r for r in rows if r['event'] == 'renderTelemetryObserved']
    consumed = [r for r in rows if r['event'] == 'browserObservationConsumed']
    resources = [r for r in rows if r['event'] in ['browserMediaResourcesObserved', 'browserMediaResourcesSnapshot']]
    commits = [r for r in resources if r['payload'].get('reason') == 'rendererCommitted']
    policies = [r for r in rows if r['event'] == 'renderPolicyApplied']
    raw_by_id = {r['payload']['sampleId']: r for r in stats}
    assert len(raw_by_id) == len(stats)
    assert len(commits) == 8


    def wall(ts):
        return datetime.fromtimestamp(ts / 1000, ZoneInfo('Asia/Shanghai')).strftime('%H:%M:%S.%f')[:-3]


    def quantile(values, fraction):
        values = sorted(v for v in values if isinstance(v, (int, float)))
        return values[max(0, math.ceil(len(values) * fraction) - 1)] if values else None


    def round_metrics(obj):
        if isinstance(obj, float):
            return round(obj, 4)
        if isinstance(obj, dict):
            return {k: round_metrics(v) for k, v in obj.items()}
        if isinstance(obj, list):
            return [round_metrics(v) for v in obj]
        return obj


    def raw_window(start, end):
        selected = []
        for r in stats:
            p = r['payload']
            d = p.get('delta') or {}
            t = p.get('sampledAtMs', 0)
            elapsed = d.get('elapsedMs', 0)
            if elapsed > 0 and t - elapsed >= start and t <= end:
                selected.append(r)
        duration = sum(r['payload']['delta']['elapsedMs'] for r in selected)
        result = {'sampleCount': len(selected), 'coverageMs': duration, 'requestedDurationMs': end - start}
        for key in ['bytesReceivedDelta', 'framesReceivedDelta', 'framesDecodedDelta', 'framesDroppedDelta', 'freezeCountDelta',
                    'totalFreezesDurationDelta', 'pauseCountDelta', 'pliCountDelta', 'firCountDelta',
                    'nackCountDelta', 'packetsLostDelta', 'packetsReceivedDelta', 'retransmittedPacketsReceivedDelta']:
            values = [r['payload']['delta'].get(key) for r in selected]
            if any(v is not None for v in values):
                result[key] = sum(v or 0 for v in values)
        if duration:
            for key, field in [('receivedFps', 'framesReceivedDelta'), ('decodedFps', 'framesDecodedDelta')]:
                result[key] = result.get(field, 0) * 1000 / duration
            result['bitrateKbpsAvg'] = result.get('bytesReceivedDelta', 0) * 8 / duration
        decode_frames = result.get('framesDecodedDelta', 0)
        result['decodeCostMsAvg'] = sum(r['payload']['delta'].get('totalDecodeTimeDelta', 0) for r in selected) * 1000 / decode_frames if decode_frames else None
        rtt = [(r['payload'].get('selectedCandidatePair') or {}).get('currentRoundTripTime') for r in selected]
        jitter = [(r['payload'].get('inboundVideo') or {}).get('jitter') for r in selected]
        result['rttMsP95'] = None if quantile(rtt, 0.95) is None else quantile(rtt, 0.95) * 1000
        result['jitterMsP95'] = None if quantile(jitter, 0.95) is None else quantile(jitter, 0.95) * 1000
        return result


    draw_points = {}
    for r in resources:
        p = r['payload']
        for renderer in p.get('resources', {}).get('renderers', []):
            count = renderer.get('realtimeDrawCount')
            if count is not None:
                draw_points.setdefault(renderer['generation'], []).append({
                    'at': p['observedAtMs'], 'count': count, 'seq': r['seq'], 'source': 'resources',
                    'timing': renderer.get('timing'), 'gpu': renderer.get('gpuTiming'),
                    'counters': {k: renderer.get(k) for k in ['latestOverwriteCount', 'rateLimitDropCount', 'submitFailureCount', 'duplicateDrawCount']},
                })
    for r in telemetry:
        p = r['payload']
        if p.get('rendererSubmittedFramesTotal') is not None:
            draw_points.setdefault(p['rendererGeneration'], []).append({
                'at': p['sampledAtMs'], 'count': p['rendererSubmittedFramesTotal'], 'seq': r['seq'], 'source': 'cachedStats',
                'timing': p.get('rendererTiming'), 'gpu': p.get('rendererGpuTiming'),
                'counters': {k: p.get('renderer' + k[0].upper() + k[1:]) for k in ['latestOverwriteCount', 'rateLimitDropCount', 'submitFailureCount', 'duplicateDrawCount']},
            })
        d = p.get('mediaDiagnostics') or {}
        if d.get('submittedFramesTotal') is not None:
            draw_points.setdefault(d['rendererGeneration'], []).append({
                'at': d['observedAtMs'], 'count': d['submittedFramesTotal'], 'seq': r['seq'], 'source': 'diagnostics',
            })


    stop = next(r for r in rows if r['event'] == 'inputUiInteractionObserved' and r['payload'].get('action') == 'powerOffExit')
    episodes = []
    for i, commit in enumerate(commits):
        p = commit['payload']
        observation = p['resources']['rendererObservation']
        generation, kind = observation['generation'], observation['kind']
        start = p['observedAtMs']
        end = commits[i + 1]['payload']['observedAtMs'] if i + 1 < len(commits) else stop['payload']['observedAtMs']
        policy = [r for r in policies if r['payload']['observedAtMs'] <= start][-1]
        episode = {'generation': generation, 'kind': kind, 'startMs': start, 'endMs': end,
                   'startTime': wall(commit['tsMs']), 'endTime': wall(commits[i + 1]['tsMs'] if i + 1 < len(commits) else stop['tsMs']),
                   'commitSeq': commit['seq'], 'policySeq': policy['seq'], 'sharpnessAtCommit': policy['payload'].get('sharpness'),
                   'srAtCommit': policy['payload'].get('sr'), 'durationMs': end - start}
        same_gen = [r for r in telemetry if r['payload'].get('rendererGeneration') == generation]
        internal_windows = []
        for prev, curr in zip(telemetry, telemetry[1:]):
            a, b = prev['payload']['observedAtMs'], curr['payload']['observedAtMs']
            if start <= a and b <= end and curr['payload'].get('rendererGeneration') == generation:
                q = curr['payload']
                internal_windows.append({'startMs': a, 'endMs': b, 'seq': curr['seq'],
                                         'callbacks': q.get('callbackCountSinceLastSample', 0),
                                         'presented': q.get('presentedFramesAdvancedSinceLastSample', 0),
                                         'gaps': q.get('callbackGapCountSinceLastSample', 0),
                                         'jumps': q.get('presentedFramesJumpCountSinceLastSample', 0),
                                         'droppedLike': q.get('droppedFramesSinceLastSample', 0),
                                         'maxCallbackMs': q.get('maxCallbackIntervalMsSinceLastSample', 0)})
        total_ms = sum(w['endMs'] - w['startMs'] for w in internal_windows)
        episode['fullTelemetryWindows'] = {
            'count': len(internal_windows), 'coverageMs': total_ms,
            'callbackFps': sum(w['callbacks'] for w in internal_windows) * 1000 / total_ms if total_ms else None,
            'videoPresentProxyFps': sum(w['presented'] for w in internal_windows) * 1000 / total_ms if total_ms else None,
            'gaps': sum(w['gaps'] for w in internal_windows), 'jumps': sum(w['jumps'] for w in internal_windows),
            'droppedLike': sum(w['droppedLike'] for w in internal_windows),
            'maxCallbackMs': max((w['maxCallbackMs'] for w in internal_windows), default=None),
            'min1sCallbackFps': min((w['callbacks'] * 1000 / (w['endMs'] - w['startMs']) for w in internal_windows), default=None),
            'min1sVideoPresentProxyFps': min((w['presented'] * 1000 / (w['endMs'] - w['startMs']) for w in internal_windows), default=None),
        }
        good_short = []
        for r in consumed:
            q = r['payload']
            s = (q.get('facts', {}).get('windows') or {}).get('short') or {}
            if q.get('rendererGeneration') == generation and q.get('sampledAtMs', 0) - 3000 >= start and q.get('sampledAtMs', 0) <= end and s.get('rendererCoverageMs', 0) >= 2999:
                good_short.append({'seq': r['seq'], 'sampledAtMs': q['sampledAtMs'], **{k: s.get(k) for k in ['rendererSubmitFpsAvg', 'videoPresentFpsAvg', 'callbackFpsAvg', 'inboundFpsAvg', 'decodeFpsAvg', 'renderDrawDurationMsAvg']}})
        episode['minMeasured3sRendererFps'] = min((s['rendererSubmitFpsAvg'] for s in good_short if s['rendererSubmitFpsAvg'] is not None), default=None)
        episode['lastComplete3s'] = good_short[-1] if good_short else None
        episode['rawStats'] = raw_window(start, end)
        episode['trackingSources'] = dict(Counter(r['payload'].get('trackingSource') for r in same_gen))
        episode['sourceFpsUnavailableReasons'] = dict(Counter(r['payload'].get('sourceFpsUnavailableReason') for r in same_gen))
        if kind != 'video':
            points = sorted(draw_points[generation], key=lambda d: (d['at'], d['count']))
            # 同时刻对齐后，用累计值检查代次内的连续性。
            assert all(a['count'] <= b['count'] for a, b in zip(points, points[1:])), (generation, points)
            first = next(d for d in points if d['seq'] == commit['seq'])
            last = max(points, key=lambda d: (d['at'], d['count']))
            episode['drawStart'] = {k: first[k] for k in ['at', 'count', 'seq']}
            episode['drawEnd'] = {k: last[k] for k in ['at', 'count', 'seq']}
            episode['postCommitDrawFps'] = (last['count'] - first['count']) * 1000 / (last['at'] - first['at'])
            episode['drawDeficitFrom60Percent'] = (1 - episode['postCommitDrawFps'] / 60) * 100
            post1s = [d for d in points if d['at'] >= start + 1000]
            steady_start = post1s[0]
            episode['post1sDrawWindow'] = {'start': {k: steady_start[k] for k in ['at', 'count', 'seq']},
                                           'end': {k: last[k] for k in ['at', 'count', 'seq']},
                                           'coverageMs': last['at'] - steady_start['at']}
            episode['post1sDrawFps'] = (last['count'] - steady_start['count']) * 1000 / (last['at'] - steady_start['at'])
            end_timing = next((d for d in reversed(points) if d.get('timing')), {})
            episode['lastTimingSeq'] = end_timing.get('seq')
            episode['lastTiming'] = end_timing.get('timing')
            episode['lastGpuTiming'] = end_timing.get('gpu')
            episode['lastRendererDropCounters'] = end_timing.get('counters')
            episode['preparation'] = observation.get('resources', {}).get('preparation')
            matched = [r for r in same_gen if r['payload'].get('rendererSubmittedFramesTotal') is not None and r['payload']['sampleId'] in raw_by_id]
            a, b = matched[0]['payload'], matched[-1]['payload']
            ia, ib = raw_by_id[a['sampleId']]['payload']['inboundVideo'], raw_by_id[b['sampleId']]['payload']['inboundVideo']
            elapsed = b['sampledAtMs'] - a['sampledAtMs']
            episode['sameCachedSampleComparison'] = {'fromSampleId': a['sampleId'], 'toSampleId': b['sampleId'], 'coverageMs': elapsed,
                'receivedFps': (ib['framesReceived'] - ia['framesReceived']) * 1000 / elapsed,
                'decodedFps': (ib['framesDecoded'] - ia['framesDecoded']) * 1000 / elapsed,
                'rendererDrawFps': (b['rendererSubmittedFramesTotal'] - a['rendererSubmittedFramesTotal']) * 1000 / elapsed}
        episodes.append(episode)

    full_ms = sum(e['fullTelemetryWindows']['coverageMs'] for e in episodes if e['kind'] == 'video')
    video_fps = sum(e['fullTelemetryWindows']['videoPresentProxyFps'] * e['fullTelemetryWindows']['coverageMs'] for e in episodes if e['kind'] == 'video') / full_ms
    last_raw = stats[-1]['payload']
    result = {'trace': str(TRACE), 'sha256': hashlib.sha256(TRACE.read_bytes()).hexdigest(), 'rows': len(rows),
              'timeRange': [wall(rows[0]['tsMs']), wall(rows[-1]['tsMs'])],
              'profileCounts': dict(Counter(r.get('traceProfile') for r in rows)),
              'rawStatsCount': len(stats), 'renderTelemetryCount': len(telemetry), 'resourceEvidenceCount': len(resources),
              'episodes': episodes, 'videoWeightedProxyFps': video_fps, 'videoWindowCoverageMs': full_ms,
              'allPlaybackRawStats': raw_window(commits[0]['payload']['observedAtMs'], stop['payload']['observedAtMs']),
              'finalInboundCounters': {k: last_raw.get('inboundVideo', {}).get(k) for k in ['framesReceived', 'framesDecoded', 'framesDropped', 'freezeCount', 'totalFreezesDuration', 'pauseCount', 'packetsLost', 'nackCount', 'pliCount', 'firCount']},
              'assumptions': ['SR FPS uses cumulative same-renderer post-commit draw counters and monotonic timestamps.',
                              'Video FPS uses complete 1Hz presented-frame advancement windows entirely inside the committed pipeline interval.',
                              'Raw network/decode averages use complete 250ms delta intervals inside the renderer interval.',
                              '3s renderer minima use full original projection windows inside the committed renderer interval.',
                              'Browser physical scanout and GPU query timings are unavailable.']}
    result = round_metrics(result)
    return result


def main():
    parser = argparse.ArgumentParser(description='复算 2026-11-09 SR/锐化新日志的同窗供给与网络冻结。')
    parser.add_argument('trace', type=Path)
    parser.add_argument('--browser-audit', type=Path, required=True)
    parser.add_argument('--production-audit', type=Path, required=True)
    parser.add_argument('--repo', type=Path, default=Path.cwd())
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    data = args.trace.read_bytes()
    assert hashlib.sha256(data).hexdigest() == EXPECTED_TRACE_SHA256, '该日期化脚本绑定报告中的不可变日志快照。'
    rows = [json.loads(line) for line in data.splitlines() if line.strip()]
    matrix = build_matrix(rows, args.trace)
    assert hashlib.sha256(data).hexdigest() == matrix['sha256']
    assert len(rows) == matrix['rows']
    raw = [r for r in rows if r['event'] == 'browserWebRtcStatsObserved']
    raw_by_id = {r['payload']['sampleId']: r for r in raw}
    telemetry = [r for r in rows if r['event'] == 'renderTelemetryObserved']
    resource_rows = [r for r in rows if r['event'] in ['browserMediaResourcesObserved', 'browserMediaResourcesSnapshot']]
    policies = [r for r in rows if r['event'] == 'renderPolicyApplied']
    episodes = matrix['episodes']
    commits = [r for r in resource_rows if r['payload'].get('reason') == 'rendererCommitted']
    assert [r['payload']['resources']['rendererObservation']['generation'] for r in commits] == [e['generation'] for e in episodes]

    def raw_window(start, end):
        selected = [r for r in raw if (r['payload'].get('delta') or {}).get('elapsedMs', 0) > 0
                    and r['payload']['sampledAtMs'] - r['payload']['delta']['elapsedMs'] >= start
                    and r['payload']['sampledAtMs'] <= end]
        duration = sum(r['payload']['delta']['elapsedMs'] for r in selected)
        counters = {k: sum((r['payload']['delta'].get(k) or 0) for r in selected) for k in
                    ['bytesReceivedDelta', 'framesReceivedDelta', 'framesDecodedDelta', 'packetsReceivedDelta', 'packetsLostDelta',
                     'nackCountDelta', 'pliCountDelta', 'firCountDelta', 'keyFramesDecodedDelta',
                     'retransmittedPacketsReceivedDelta', 'freezeCountDelta', 'totalFreezesDurationDelta',
                     'framesDroppedDelta', 'totalDecodeTimeDelta']}
        return {'sampleCount': len(selected), 'coverageMs': duration, 'requestedDurationMs': end - start,
                'receivedFps': counters['framesReceivedDelta'] * 1000 / duration if duration else None,
                'decodedFps': counters['framesDecodedDelta'] * 1000 / duration if duration else None,
                'bitrateKbpsAvg': counters['bytesReceivedDelta'] * 8 / duration if duration else None,
                'decodeCostMsAvg': counters['totalDecodeTimeDelta'] * 1000 / counters['framesDecodedDelta'] if counters['framesDecodedDelta'] else None,
                'collectionDurationMsP95': quantile([r['payload'].get('collectionDurationMs') for r in selected], .95),
                **counters}

    points = []
    for r in resource_rows:
        p = r['payload']
        for renderer in p.get('resources', {}).get('renderers', []):
            if renderer.get('realtimeDrawCount') is not None:
                points.append({'at': p['observedAtMs'], 'drawn': renderer['realtimeDrawCount'],
                               'generation': renderer['generation'], 'seq': r['seq'], 'source': 'resources'})
    for r in telemetry:
        p = r['payload']
        if p.get('rendererSubmittedFramesTotal') is not None:
            points.append({'at': p['sampledAtMs'], 'drawn': p['rendererSubmittedFramesTotal'],
                           'generation': p['rendererGeneration'], 'seq': r['seq'], 'source': 'cachedStats'})
        d = p.get('mediaDiagnostics') or {}
        if d.get('submittedFramesTotal') is not None:
            points.append({'at': d['observedAtMs'], 'drawn': d['submittedFramesTotal'],
                           'generation': d['rendererGeneration'], 'seq': r['seq'], 'source': 'liveDiagnostics'})

    def measured_draw_window(start, end, generation):
        ps = sorted((p for p in points if p['generation'] == generation and start <= p['at'] <= end), key=lambda p: (p['at'], p['drawn']))
        assert len(ps) >= 2
        assert all(a['drawn'] <= b['drawn'] for a, b in zip(ps, ps[1:]))
        a, b = ps[0], ps[-1]
        return {'start': a, 'end': b, 'coverageMs': b['at'] - a['at'], 'drawnDelta': b['drawn'] - a['drawn'],
                'rendererDrawFps': (b['drawn'] - a['drawn']) * 1000 / (b['at'] - a['at'])}

    def matched_cached_window(start, end, generation):
        selected = sorted((r for r in telemetry if r['payload'].get('rendererGeneration') == generation
                           and r['payload'].get('rendererSubmittedFramesTotal') is not None
                           and start <= r['payload']['sampledAtMs'] <= end), key=lambda r: r['payload']['sampledAtMs'])
        assert len(selected) >= 2
        a, b = selected[0], selected[-1]
        x, y = a['payload'], b['payload']
        ix, iy = raw_by_id[x['sampleId']]['payload']['inboundVideo'], raw_by_id[y['sampleId']]['payload']['inboundVideo']
        duration = y['sampledAtMs'] - x['sampledAtMs']
        received, decoded = iy['framesReceived'] - ix['framesReceived'], iy['framesDecoded'] - ix['framesDecoded']
        drawn = y['rendererSubmittedFramesTotal'] - x['rendererSubmittedFramesTotal']
        return {'startSeq': a['seq'], 'endSeq': b['seq'], 'fromSampleId': x['sampleId'], 'toSampleId': y['sampleId'],
                'startMs': x['sampledAtMs'], 'endMs': y['sampledAtMs'], 'coverageMs': duration,
                'receivedDelta': received, 'decodedDelta': decoded, 'drawnDelta': drawn, 'decodeDrawDeficitFrames': decoded - drawn,
                'receivedFps': received * 1000 / duration, 'decodedFps': decoded * 1000 / duration,
                'rendererDrawFps': drawn * 1000 / duration,
                'freezeCountDelta': iy['freezeCount'] - ix['freezeCount'],
                'freezesDurationDelta': iy['totalFreezesDuration'] - ix['totalFreezesDuration'],
                'drawnOverDecoded': drawn / decoded if decoded else None}

    short_minima = []
    for e in episodes:
        candidates = []
        for r in rows:
            p = r.get('payload') or {}
            s = (p.get('facts', {}).get('windows') or {}).get('short') or {}
            if r['event'] == 'browserObservationConsumed' and p.get('rendererGeneration') == e['generation'] \
                    and p.get('sampledAtMs', 0) - 3000 >= e['startMs'] and p['sampledAtMs'] <= e['endMs'] \
                    and s.get('rendererCoverageMs', 0) >= 2999 and s.get('rendererSubmitFpsAvg') is not None:
                candidates.append((r, s))
        if candidates:
            r, s = min(candidates, key=lambda pair: pair[1]['rendererSubmitFpsAvg'])
            short_minima.append({'generation': e['generation'], 'seq': r['seq'], 'sampledAtMs': r['payload']['sampledAtMs'],
                                 **{k: s.get(k) for k in ['durationMs', 'sampleCount', 'rendererCoverageMs', 'inboundFpsAvg',
                                                         'decodeFpsAvg', 'videoPresentFpsAvg', 'callbackFpsAvg', 'rendererSubmitFpsAvg',
                                                         'renderDrawDurationMsAvg', 'renderUploadDurationMsAvg', 'presentAgeMsAvg']}})

    toggle = next(r for r in policies if r['seq'] == 2352)
    toggle_at = toggle['payload']['observedAtMs']
    sr = next(e for e in episodes if e['generation'] == 5)
    split = []
    for sharpness, start, end in [(3, sr['startMs'], toggle_at), (0, toggle_at, sr['endMs'])]:
        split.append({'sharpness': sharpness, 'requestedStartMs': start, 'requestedEndMs': end,
                      'drawWindow': measured_draw_window(start, end, 5),
                      'matchedCachedWindow': matched_cached_window(start, end, 5), 'rawStats': raw_window(start, end)})

    frame_identity_deltas = []
    for e in episodes:
        if e['kind'] == 'video':
            continue
        selected = [r for r in telemetry if (r['payload'].get('mediaDiagnostics') or {}).get('rendererGeneration') == e['generation']]
        a, b = selected[0], selected[-1]
        x, y = a['payload']['mediaDiagnostics'], b['payload']['mediaDiagnostics']
        fx, fy = x['frameIdentity'], y['frameIdentity']
        delta = {'generation': e['generation'], 'startSeq': a['seq'], 'endSeq': b['seq'], 'startMs': x['observedAtMs'],
                 'endMs': y['observedAtMs'], 'coverageMs': y['observedAtMs'] - x['observedAtMs'],
                 'drawnDelta': y['submittedFramesTotal'] - x['submittedFramesTotal'],
                 'rvfcObservedDelta': fy['observed']['videoFrameCallback'] - fx['observed']['videoFrameCallback'],
                 'rvfcAcceptedDelta': fy['accepted']['videoFrameCallback'] - fx['accepted']['videoFrameCallback'],
                 'rafObservedDelta': fy['observed']['animationFrame'] - fx['observed']['animationFrame'],
                 'rafAcceptedDelta': fy['accepted']['animationFrame'] - fx['accepted']['animationFrame']}
        assert delta['drawnDelta'] == delta['rvfcAcceptedDelta'] + delta['rafAcceptedDelta']
        frame_identity_deltas.append(delta)

    diagnostics = [r['payload']['mediaDiagnostics'] for r in telemetry]
    pixels = [d['frameIdentity']['videoPixels'] for d in diagnostics]
    pixel_keys = ['pixelProbeCanvases', 'pixelProbePreparations', 'pixelProbeGpuReads', 'pixelProbePendingPairs',
                  'pixelProbeTimers', 'readCount', 'completedReadCount', 'gpuErrorChecks']
    assert all(p['enabled'] is False and all(p[k] == 0 for k in pixel_keys) for p in pixels)
    assert all(p['preparationTiming']['count'] == 0 for p in pixels)
    assert all((r['payload'].get('resources') or {}).get(k, 0) == 0 for r in resource_rows for k in pixel_keys[:5])

    x, y = diagnostics[0]['frameIdentity'], diagnostics[-1]['frameIdentity']
    total_identity = {'coverageMs': telemetry[-1]['payload']['mediaDiagnostics']['observedAtMs'] - telemetry[0]['payload']['mediaDiagnostics']['observedAtMs'],
                      'observedDelta': {s: y['observed'][s] - x['observed'][s] for s in y['observed']},
                      'acceptedDelta': {s: y['accepted'][s] - x['accepted'][s] for s in y['accepted']},
                      'lastCounterCadence': y['counterCadence'], 'lastCallbackTiming': y['callbackTiming']}

    frozen_rows = [r for r in raw if (r['payload'].get('delta') or {}).get('freezeCountDelta', 0) > 0]
    freeze_anchors = [{'seq': r['seq'], 'time': wall(r['tsMs']), 'sampledAtMs': r['payload']['sampledAtMs'],
                       'freezeCountDelta': r['payload']['delta']['freezeCountDelta'],
                       'durationSec': r['payload']['delta']['totalFreezesDurationDelta']} for r in frozen_rows]
    network_points = []
    for r in raw:
        p, d = r['payload'], r['payload'].get('delta') or {}
        if 325500 <= p['sampledAtMs'] <= 328000 or 332000 <= p['sampledAtMs'] <= 334600:
            network_points.append({'seq': r['seq'], 'time': wall(r['tsMs']), 'sampledAtMs': p['sampledAtMs'],
                                   'elapsedMs': d.get('elapsedMs'), **{k: d.get(k) for k in
                                    ['framesReceivedDelta', 'framesDecodedDelta', 'keyFramesDecodedDelta', 'packetsReceivedDelta',
                                     'packetsLostDelta', 'nackCountDelta', 'pliCountDelta', 'freezeCountDelta', 'totalFreezesDurationDelta']},
                                   'jitterMs': (p.get('inboundVideo') or {}).get('jitter', 0) * 1000})

    ordinary_video = [e for e in episodes if e['generation'] in [2, 4, 8]]
    duration = sum(e['fullTelemetryWindows']['coverageMs'] for e in ordinary_video)
    weighted_fps = sum(e['fullTelemetryWindows']['videoPresentProxyFps'] * e['fullTelemetryWindows']['coverageMs'] for e in ordinary_video) / duration
    build = next(r['payload']['buildFingerprint'] for r in rows if r['event'] == 'runtimeBuildInfo')
    current = source_fingerprint(args.repo)
    current['matchesTraceBuild'] = current['sha256'] == build['sourceFingerprintSha256']
    browser = json.loads(args.browser_audit.read_text())
    production = json.loads(args.production_audit.read_text())
    first_diagnostic_at = telemetry[0]['payload']['mediaDiagnostics']['observedAtMs']
    before_diagnostic = [r for r in raw if r['payload']['sampledAtMs'] < first_diagnostic_at]
    early_diagnostic = [r for r in raw if first_diagnostic_at <= r['payload']['sampledAtMs'] <= first_diagnostic_at + 5000]

    for e in episodes:
        if 'post1sDrawWindow' in e:
            e['lateFirstAvailableDrawWindow'] = e.pop('post1sDrawWindow')
            e['lateFirstAvailableDrawFps'] = e.pop('post1sDrawFps')
            e['lateFirstAvailableDrawWindow']['startOffsetFromCommitMs'] = e['lateFirstAvailableDrawWindow']['start']['at'] - e['startMs']

    safe_input = [{ 'seq': r['seq'], 'time': wall(r['tsMs']), **{k: r['payload'].get(k) for k in
                    ['action', 'source', 'scope', 'outcome', 'operationId', 'observedAtMs', 'durationMs', 'reason']}}
                  for r in rows if r['event'] == 'inputUiInteractionObserved' and r['payload'].get('action') in
                  ['toggleExperience', 'toggleBrowserDiagnostics', 'toggleSuperResolution', 'powerOffExit']]
    closed = next(r for r in resource_rows if r['payload'].get('reason') == 'clientClosed')
    result = {'schemaVersion': 1, 'analysisVersion': 3,
              'analyzedAt': datetime.now().astimezone().isoformat(timespec='milliseconds'),
              'analysisScriptSha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              'traceIdentity': {'file': args.trace.name, 'originalFile': 'runtime-trace-1794190273035-1.jsonl', 'sha256': hashlib.sha256(data).hexdigest(), 'bytes': len(data),
                                'rows': len(rows), 'timeRange': [wall(rows[0]['tsMs']), wall(rows[-1]['tsMs'])],
                                'profileCounts': dict(Counter(r.get('traceProfile') for r in rows)), 'build': build},
              'currentSourceFingerprint': current,
              'counts': {'rawStats': len(raw), 'renderTelemetry': len(telemetry), 'resources': len(resource_rows),
                         'rendererCommits': len(commits), 'traceBudgetNotices': sum(r['event'] == 'traceBudgetNotice' for r in rows)},
              'episodes': episodes, 'srSharpnessSplit': {'toggleSeq': toggle['seq'], 'time': wall(toggle['tsMs']),
                    'observedAtMs': toggle_at, 'unchangedRendererGeneration': 5, 'unchangedSrPlan': toggle['payload']['sr'],
                    'processing': toggle['payload']['processing'], 'segments': split},
              'ordinaryVideo': {'generations': [2, 4, 8], 'coverageMs': duration, 'videoPresentProxyFps': weighted_fps,
                                'freezeCount': sum(e['rawStats']['freezeCountDelta'] for e in ordinary_video),
                                'freezesDurationSec': sum(e['rawStats']['totalFreezesDurationDelta'] for e in ordinary_video)},
              'completeShortWindowMinima': short_minima, 'frameIdentityDeltas': frame_identity_deltas,
              'totalFrameIdentity': total_identity,
              'pixelDiagnostics': {'sampleCount': len(pixels), 'allDisabled': True, 'resourceEvidenceCount': len(resource_rows),
                                   'maxCounters': {k: max(p[k] for p in pixels) for k in pixel_keys}, 'preparationCount': 0},
              'network': {'allPlayback': matrix['allPlaybackRawStats'], 'finalInboundCounters': matrix['finalInboundCounters'],
                          'freezeAnchors': freeze_anchors, 'rawAnomalyPoints': network_points,
                          'srRetryBeforeFourthFreeze': matched_cached_window(330676, 334076, 7)},
              'samplingCoverage': {'firstDiagnosticAtMs': first_diagnostic_at,
                                   'firstDiagnosticDelayFromCommitMs': first_diagnostic_at - episodes[0]['startMs'],
                                   'beforeMedianRawIntervalMs': quantile([(r['payload'].get('delta') or {}).get('elapsedMs') for r in before_diagnostic], .5),
                                   'afterMedianRawIntervalMs': quantile([(r['payload'].get('delta') or {}).get('elapsedMs') for r in early_diagnostic], .5),
                                   'documentVisibilityEvidence': 'missing', 'nativeWindowOcclusionEvidence': 'missing'},
              'gates': {'browserBehavior': browser['browserWebRtcBehaviorGate'], 'browserStability': browser['browserRuntimeStability']['gate'],
                        'productionHealth': production['gate'], 'traceLoss': production['traceLoss'], 'ending': production['ending'],
                        'capabilities': production['capabilities']},
              'inputObservations': safe_input, 'cleanup': {'seq': closed['seq'], 'time': wall(closed['tsMs']), 'resources': closed['payload']['resources']},
              'measurementContract': ['增强 FPS 使用同代际单调累计提交量；video 使用完整代次内累计推进窗口，代表浏览器呈现代理。',
                                      '锐度切换边界两侧采用已存在的累计采样点，保留切换边界缺口，禁止插值。',
                                      '同 sampleId 的 getStats 接收/解码与缓存 renderer 累计值按原始 sampledAtMs 配对。',
                                      '原始网络 delta 使用完全处于目标区间的有效窗，包含首段 1Hz 和后续 250ms。',
                                      '各段实际码率使用同窗 bytesReceivedDelta 加权复算；画面负载差异作为跨段比较边界。',
                                      '完整 3s 最低值使用 renderer coverage >=2999ms 且完全位于代次区间的原始投影。',
                                      '首段缺少文档可见性与原生窗口遮挡证据；首个详细采样之前的 FPS 仅作全段事实。',
                                      'RVFC 合批推进及冻结后的追赶可使短窗推进/接收超过60；canvas累计提交保留其独立口径。',
                                      'Browser 原生 ReferenceChain 与物理 scanout 为 UNSUPPORTED，GPU query 为 unavailable。']}
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps({'output': str(args.output), 'traceSha256': result['traceIdentity']['sha256'],
                      'pixelDiagnosticsAllDisabled': result['pixelDiagnostics']['allDisabled'],
                      'ordinaryVideoFps': weighted_fps,
                      'srSharpnessDrawFps': [s['drawWindow']['rendererDrawFps'] for s in split],
                      'srRetryBeforeFreezeFps': result['network']['srRetryBeforeFourthFreeze']['rendererDrawFps'],
                      'candidateSubmitPairing': 'PASS'}, ensure_ascii=False))


if __name__ == '__main__':
    main()
