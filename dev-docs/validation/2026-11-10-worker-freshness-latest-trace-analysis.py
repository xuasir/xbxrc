"""复算修复后的尺寸协商、Worker 分层供给与本地新鲜度；输出安全摘要。"""

import argparse
import importlib.util
import json
from collections import Counter, defaultdict
from pathlib import Path


spec = importlib.util.spec_from_file_location(
    'supply_analysis', Path(__file__).with_name('2026-11-10-decoded-supply-latest-trace-analysis.py'))
supply = importlib.util.module_from_spec(spec)
spec.loader.exec_module(supply)


def supply_points(rows):
    points = []
    for row in rows:
        p = row['payload']
        if row['event'] in ('browserMediaResourcesObserved', 'browserMediaResourcesSnapshot'):
            renderers = p.get('resources', {}).get('renderers', [])
            for renderer in renderers:
                if renderer.get('decodedFrameSupply'):
                    points.append({'seq': row['seq'], 'event': row['event'],
                                   'atMs': p['observedAtMs'], 'renderer': renderer,
                                   'supply': renderer['decodedFrameSupply']})
        elif row['event'] == 'renderTelemetryObserved' and p.get('rendererDecodedFrameSupply'):
            points.append({'seq': row['seq'], 'event': row['event'], 'atMs': p['sampledAtMs'],
                           'renderer': {'realtimeDrawCount': p['rendererSubmittedFramesTotal'],
                                        'timing': p['rendererTiming']},
                           'supply': p['rendererDecodedFrameSupply']})
    return points


def point_summary(point):
    s, d = point['supply'], point['renderer']
    keys = ['workerReadFrames', 'workerSentFrames', 'receivedMessages', 'deliveredFrames',
            'workerOverwriteCount', 'staleFrameDropCount', 'workerAckCount', 'invalidClockCount',
            'workerObservationAgeMs']
    return {'seq': point['seq'], 'event': point['event'], 'atMs': point['atMs'],
            'submittedFrames': d['realtimeDrawCount'], **{k: s.get(k) for k in keys},
            'supplyTiming': s['timing'], 'rendererTiming': d['timing']}


def analyze_worker(rows):
    points = supply_points(rows)
    live = [p for p in points if p['event'] != 'renderTelemetryObserved']
    final = live[-1]
    s, d = final['supply'], final['renderer']
    changes = []
    clock_changes = []
    for a, b in zip(points, points[1:]):
        increments = {k: b['supply'][k] - a['supply'][k]
                      for k in ['workerOverwriteCount', 'staleFrameDropCount']}
        if any(increments.values()):
            changes.append({'from': point_summary(a), 'to': point_summary(b), 'increments': increments})
        if b['supply']['invalidClockCount'] > a['supply']['invalidClockCount']:
            clock_changes.append({'fromSeq': a['seq'], 'toSeq': b['seq'], 'atMs': b['atMs'],
                                  'count': b['supply']['invalidClockCount']})
    retained = {}
    for owner, keys in [('supply', list(s['timing'])), ('renderer', list(d['timing']))]:
        for key in keys:
            windows = [p for p in points if p[owner]['timing'][key]['count'] >= 256]
            if windows:
                distributions = [p[owner]['timing'][key] for p in windows]
                retained[f'{owner}.{key}'] = {
                    'snapshots': len(windows),
                    'p95RangeMs': [min(t['p95Ms'] for t in distributions), max(t['p95Ms'] for t in distributions)],
                    'p99RangeMs': [min(t['p99Ms'] for t in distributions), max(t['p99Ms'] for t in distributions)],
                    'retainedMaxMs': max(t['maxMs'] for t in distributions),
                }
    resources = [r for r in rows if r['event'] in ('browserMediaResourcesObserved', 'browserMediaResourcesSnapshot')]
    return {
        'snapshots': len(points), 'liveSnapshots': len(live), 'final': point_summary(final),
        'layeredDelta': {
            'readToSent': s['workerReadFrames'] - s['workerSentFrames'],
            'sentToMainReceived': s['workerSentFrames'] - s['receivedMessages'],
            'mainReceivedToDelivered': s['receivedMessages'] - s['deliveredFrames'],
            'deliveredToSubmitted': s['deliveredFrames'] - d['realtimeDrawCount'],
        },
        'lossChanges': changes, 'invalidClockChanges': clock_changes,
        'invalidClockRatio': s['invalidClockCount'] / s['receivedMessages'],
        'ackDeliveryValidRecordRatioToAckCount': s['timing']['ackDelivery']['count'] / s['workerAckCount'],
        'retainedWindowRanges': retained,
        'liveBalanceFailures': [point_summary(p) for p in live
                               if p['supply']['deliveredFrames'] != p['renderer']['realtimeDrawCount']],
        'actualSrStates': dict(Counter(str(r['payload']['resources'].get('actualSuperResolutionActive'))
                                      for r in resources if r['payload']['resources'].get('actualPipeline'))),
    }


def analyze_resolution(rows):
    resolved = next(r for r in rows if r['event'] == 'sessionVideoConstraintsResolved')
    execution = next(r for r in rows if r['event'] == 'sessionExecutionStarted')
    stages = []
    for row in rows:
        p = row['payload']
        if row['event'] == 'browserWebRtcSdpObserved' and p['stage'] in ('localOfferAfterPatch', 'remoteAnswer'):
            stages.append({'seq': row['seq'], 'stage': p['stage'],
                           'videoImageConstraints': p.get('videoImageConstraints'),
                           'h264': [{k: v.get(k) for k in ['payloadType', 'profileLevelId', 'packetizationMode', 'fmtp']}
                                    for v in p.get('h264Payloads', [])]})
    stats = [r for r in rows if r['event'] == 'browserWebRtcStatsObserved'
             and r['payload'].get('inboundVideo', {}).get('frameWidth', 0) > 0]
    return {'resolved': {'seq': resolved['seq'], **resolved['payload']},
            'execution': {'seq': execution['seq'], **{k: execution['payload']['runtime'][k]
                                                     for k in ['targetVideoWidth', 'targetVideoHeight']}},
            'sdp': stages, 'validDimensionSamples': len(stats),
            'actualDimensions': dict(Counter(f"{r['payload']['inboundVideo']['frameWidth']}x{r['payload']['inboundVideo']['frameHeight']}" for r in stats)),
            'firstActualSeq': stats[0]['seq'], 'lastActualSeq': stats[-1]['seq']}


def shared_sample_evidence(rows):
    consumers = defaultdict(set)
    consumer_samples = defaultdict(list)
    for row in rows:
        if row['event'] != 'browserObservationConsumed':
            continue
        p = row['payload']
        identity = (row.get('traceRunId'), row.get('sessionId'), p.get('clientGeneration'),
                    p.get('peerGeneration'), p.get('sampleId'))
        consumers[identity].add(p['consumer'])
        consumer_samples[p['consumer']].append(p['sampleId'])
    return {'criterion': 'panel and supervision consume the same run/session/client/peer/sample identity',
            'sharedSamples': sum({'panel', 'supervision'}.issubset(group) for group in consumers.values()),
            'consumerSamples': dict(consumer_samples)}


def anomaly_rtc_window(rows, worker):
    # 本次新增覆盖与陈旧丢弃在两个快照之间；保留同窗 250 ms RTC delta。
    windows = []
    for change in worker['lossChanges']:
        a, b = change['from'], change['to']
        if not change['increments']['staleFrameDropCount']:
            continue
        stats = [r for r in rows if r['event'] == 'browserWebRtcStatsObserved'
                 and a['atMs'] < r['payload']['sampledAtMs'] <= b['atMs']]
        windows.append({'fromSeq': a['seq'], 'toSeq': b['seq'], 'snapshotIntervalMs': b['atMs'] - a['atMs'],
                        'rtc': [{'seq': r['seq'], 'sampleId': r['payload']['sampleId'],
                                 'atMs': r['payload']['sampledAtMs'], 'delta': r['payload']['delta'],
                                 'rttMs': r['payload'].get('selectedCandidatePair', {}).get('currentRoundTripTime', 0) * 1000,
                                 'jitterMs': r['payload']['inboundVideo'].get('jitter', 0) * 1000} for r in stats]})
    return windows


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('trace', type=Path)
    parser.add_argument('--source', required=True)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--source-root', type=Path)
    parser.add_argument('--browser-report', type=Path, action='append', default=[])
    args = parser.parse_args()
    rows = [json.loads(line) for line in args.trace.read_text().splitlines() if line.strip()]
    result = supply.analyze(args.trace, args.source, args.source_root, args.browser_report, None)
    result['sequence'] = {'first': rows[0]['seq'], 'last': rows[-1]['seq'],
                          'continuous': all(b['seq'] == a['seq'] + 1 for a, b in zip(rows, rows[1:]))}
    result['boundaries'][6] = 'Worker read-finished to submit is measured locally; VideoFrame timestamp is zero, and capture-to-display / physical scanout remain unavailable.'
    result['resolution'] = analyze_resolution(rows)
    result['worker'] = analyze_worker(rows)
    result['staleDeliveryWindow'] = anomaly_rtc_window(rows, result['worker'])
    result['sharedSampleEvidence'] = shared_sample_evidence(rows)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps({'sequence': result['sequence'], 'dimensions': result['resolution']['actualDimensions'],
                      'workerLayeredDelta': result['worker']['layeredDelta'],
                      'lossWindows': [{'fromSeq': v['from']['seq'], 'toSeq': v['to']['seq'], 'delta': v['increments']}
                                      for v in result['worker']['lossChanges']],
                      'retainedTiming': result['worker']['retainedWindowRanges'],
                      'source': result.get('currentSource')}, ensure_ascii=False))
