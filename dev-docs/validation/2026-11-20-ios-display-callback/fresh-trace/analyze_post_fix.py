#!/usr/bin/env python3
"""Compare display callback timings and foreground streaming supply from raw trace."""
import collections
import csv
import hashlib
import json
import math
from pathlib import Path

SOURCE = Path('/Users/guo.xu/Downloads/runtime-trace-ios-1795187951399-1.jsonl')
OUTPUT = Path(__file__).resolve().parent
ROWS = [json.loads(line) for line in SOURCE.open()]
FIRST_TS = min(row['tsMs'] for row in ROWS)


def distribution(values):
    values = sorted(v for v in values if isinstance(v, (float, int)) and not isinstance(v, bool) and math.isfinite(v))
    if not values:
        return {'n': 0}
    def percentile(q):
        return values[max(0, math.ceil(len(values) * q) - 1)]
    return {'n': len(values), 'min': values[0], 'mean': sum(values) / len(values),
            'p50': percentile(.50), 'p95': percentile(.95), 'p99': percentile(.99), 'max': values[-1]}


def write(name, data):
    (OUTPUT / name).write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n')


active = True
active_start = ROWS[0]['payload']['sourceMonotonicMs']
foreground = []
for row in ROWS:
    if row['event'] != 'scenePhaseChanged':
        continue
    p = row['payload']
    now = p['sourceMonotonicMs']
    if active and p['to'] != 'active':
        foreground.append((active_start, now))
        active = False
    elif not active and p['to'] == 'active':
        active_start = now
        active = True
if active:
    foreground.append((active_start, ROWS[-1]['payload']['sourceMonotonicMs']))


def fully_foreground(start, end):
    return start is not None and end is not None and any(a <= start <= end <= b for a, b in foreground)


launches = [row for row in ROWS if row['event'] == 'streamLaunchStarted']
summary = {'input': {'file': str(SOURCE), 'sha256': hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
                     'rows': len(ROWS), 'firstTsMs': FIRST_TS, 'lastTsMs': ROWS[-1]['tsMs']},
           'foregroundIntervalsMonotonicMs': foreground, 'attempts': []}
windows = []
anchors = []
timeline_events = {'streamLaunchStarted', 'sessionReady', 'peerConnected', 'firstVideoFrame', 'displayFirstPresent',
                   'steadyMediaObserved', 'streamingStateChanged', 'scenePhaseChanged', 'sessionSuspended',
                   'sessionResumeValidated', 'sessionTerminal', 'terminalSelected', 'peerDisconnected',
                   'peerClosed', 'remoteSessionClosed', 'accessReleased', 'audioPlayoutGlitchDetected'}
for row in ROWS:
    if row['event'] in timeline_events:
        anchors.append(row)

for index, launch in enumerate(launches):
    attempt = launch['payload']['attemptId']
    end_seq = launches[index + 1]['seq'] if index + 1 < len(launches) else len(ROWS) + 1
    run = [row for row in ROWS if launch['seq'] - 1 <= row['seq'] < end_seq]
    groups = collections.defaultdict(list)
    for row in run:
        groups[row['event']].append(row)
    presents = groups['displayPresent']
    submissions = {(r['payload'].get('peerEpoch'), r['payload']['frameId']): r for r in groups['displaySubmit']}
    gpu_times = []
    representative = []
    for row in presents:
        p = row['payload']
        submission = submissions.get((p.get('peerEpoch'), p['frameId']))
        if submission:
            gpu_ms = p['presentedAtMonotonicMs'] - submission['payload']['submittedAtMonotonicMs']
            gpu_times.append(gpu_ms)
            representative.append((p['gpuToMainActorMs'], submission, row, gpu_ms))
    metrics = {name: distribution([r['payload'].get(name) for r in presents]) for name in
               ['gpuToSlotReleaseMs', 'gpuToMainActorMs', 'frameFeedbackWorkMs', 'displayTelemetryWorkMs',
                'mainActorWorkBeforeTraceMs', 'gpuToTraceRecordMs']}
    metrics['submitToGpuCompletionMs'] = distribution(gpu_times)
    metrics['mainActorQueueSharePercent'] = distribution([
        100 * r['payload']['gpuToMainActorMs'] / r['payload']['gpuToTraceRecordMs']
        for r in presents if r['payload']['gpuToTraceRecordMs'] > 0])
    metrics['writerQueueDelayMs'] = distribution([r['payload'].get('writerQueueDelayMs') for r in run])
    for event, names in [('rtcHealthSnapshot', ['rttMs', 'jitterMs', 'receiveBitrateBps', 'freezeCount', 'freezeDurationMs']),
                         ('displayHealthSnapshot', ['frameIntervalMs', 'submitToPresentP50Ms', 'submitToPresentP95Ms',
                                                    'submitToPresentP99Ms', 'submitToDrawablePresentP50Ms',
                                                    'submitToDrawablePresentP95Ms', 'submitToDrawablePresentP99Ms']),
                         ('runtimeSupervisionSnapshot', ['inboundFps', 'decodeFps', 'presentFps', 'videoPresentAgeMs'])]:
        for name in names:
            metrics[event + '.' + name] = distribution([r['payload'].get(name) for r in groups[event]])

    first_present = presents[0]['payload']['presentedAtMonotonicMs'] if presents else None
    rate_items = collections.defaultdict(list)
    displays = {r['payload'].get('sampleId'): r for r in groups['displayHealthSnapshot']}
    for row in groups['rtcHealthSnapshot']:
        p = row['payload']
        display_row = displays.get(p.get('sampleId'))
        width = p.get('sampleWindowMs')
        if not isinstance(width, (int, float)) or width <= 0 or p.get('counterReset'):
            continue
        is_fg = fully_foreground(p.get('windowStartMonotonicMs'), p.get('windowEndMonotonicMs'))
        is_steady = first_present is not None and p['windowStartMonotonicMs'] >= first_present
        w = {'attempt': index + 1, 'seq': row['seq'], 'offsetSeconds': (row['tsMs'] - FIRST_TS) / 1000,
             'sampleId': p['sampleId'], 'sampleWindowMs': width, 'fullyForeground': is_fg, 'afterFirstPresent': is_steady}
        for name in ['framesReceivedDelta', 'framesDecodedDelta', 'packetsReceivedDelta', 'packetsLostDelta']:
            w[name] = p.get(name)
            if isinstance(p.get(name), (int, float)):
                w[name + 'Rate'] = p[name] * 1000 / width
                rate_items['all.' + name].append((p[name], width))
                if is_fg and is_steady:
                    rate_items['foreground.' + name].append((p[name], width))
        if display_row:
            dp = display_row['payload']
            dw = dp.get('sampleWindowMs')
            w['displaySampleWindowMs'] = dw
            if isinstance(dw, (int, float)) and dw > 0 and not dp.get('counterReset'):
                for name in ['submitCountDelta', 'presentCountDelta', 'drawablePresentCountDelta']:
                    w[name] = dp.get(name)
                    if isinstance(dp.get(name), (int, float)):
                        w[name + 'Rate'] = dp[name] * 1000 / dw
                        rate_items['all.' + name].append((dp[name], dw))
                        if is_fg and is_steady and fully_foreground(dp.get('windowStartMonotonicMs'), dp.get('windowEndMonotonicMs')):
                            rate_items['foreground.' + name].append((dp[name], dw))
        windows.append(w)
    for name, samples in rate_items.items():
        metrics[name + 'Rate'] = {**distribution([count * 1000 / width for count, width in samples]),
                                 'weightedMean': sum(count for count, _ in samples) * 1000 / sum(width for _, width in samples),
                                 'totalCount': sum(count for count, _ in samples), 'totalWindowMs': sum(width for _, width in samples)}

    foreground_segments = []
    if first_present is not None:
        for a, b in foreground:
            selected = [r for r in presents if a <= r['payload']['presentedAtMonotonicMs'] <= b]
            if len(selected) >= 2:
                begin = selected[0]['payload']['presentedAtMonotonicMs']
                end = selected[-1]['payload']['presentedAtMonotonicMs']
                foreground_segments.append({'firstSeq': selected[0]['seq'], 'lastSeq': selected[-1]['seq'],
                                            'frames': len(selected), 'durationMs': end - begin,
                                            'completedFps': (len(selected) - 1) * 1000 / (end - begin),
                                            'gpuToMainActorMs': distribution([r['payload']['gpuToMainActorMs'] for r in selected])})
    states = collections.Counter(r['payload'].get('state') for r in groups['streamingStateChanged']
                                 if r['payload'].get('attemptId') == attempt)
    drops = collections.Counter()
    for r in groups['videoFrameDropped']:
        drops[r['payload']['reason']] += r['payload'].get('count', 1)
    config = groups['peerRuntimeConfigured'][0]['payload']
    report = {'attempt': index + 1, 'attemptId': attempt, 'generation': launch['payload']['generation'],
              'launchSeq': launch['seq'], 'startedAtMs': launch['tsMs'], 'endAtMs': run[-1]['tsMs'],
              'metrics': metrics, 'foregroundSegments': foreground_segments,
              'acceptedEvents': len(groups['videoFrameAcceptedToMailbox']), 'submittedEvents': len(groups['displaySubmit']),
              'completedEvents': len(presents), 'drops': drops, 'stateCounts': states,
              'lastRtc': groups['rtcHealthSnapshot'][-1]['payload'],
              'lastDisplay': groups['displayHealthSnapshot'][-1]['payload'] if groups['displayHealthSnapshot'] else None,
              'lastSupervision': groups['runtimeSupervisionSnapshot'][-1]['payload'] if groups['runtimeSupervisionSnapshot'] else None,
              'terminal': [r for r in groups['sessionTerminal']],
              'configuration': {k: config.get(k) for k in ['deviceModel', 'osVersion', 'targetVideoWidth', 'targetVideoHeight',
                                                         'maxVideoBitrateKbps', 'preventResolutionDrops', 'preferIPv6']},
              'networkPressureCounts': collections.Counter(r['payload'].get('networkPressure') for r in groups['runtimeSupervisionSnapshot']),
              'qualityStateCounts': collections.Counter(r['payload'].get('qualityState') for r in groups['runtimeSupervisionSnapshot']),
              'connectionRecoveryCounts': collections.Counter(str(r['payload'].get('connectionRecoveryActive')) for r in groups['runtimeSupervisionSnapshot']),
              'freshCounts': collections.Counter(str(r['payload'].get('fresh')) for r in groups['runtimeSupervisionSnapshot']),
              'receiverReasons': collections.Counter(r['payload'].get('reason') for r in groups['receiverTrackDiscovered']),
              'channelEvents': {str(k): v for k, v in collections.Counter((r['payload'].get('label'), r['payload'].get('event')) for r in groups['dataChannelStateChanged']).items()}}
    if representative:
        ordered = sorted(representative, key=lambda item: item[0])
        for q in [.5, .95, .99, 1]:
            _, submit, present, gpu = ordered[max(0, math.ceil(q * len(ordered)) - 1)]
            anchors.append({'quantile': q, 'attempt': index + 1, 'gpuMs': gpu, 'submission': submit, 'completion': present})
    summary['attempts'].append(report)

summary['writerQueueDelayMsAll'] = distribution([r['payload'].get('writerQueueDelayMs') for r in ROWS])
summary['eventCounts'] = collections.Counter(r['event'] for r in ROWS)
write('health-summary.json', summary)
write('evidence-anchors.json', anchors)
if windows:
    fields = sorted({key for w in windows for key in w})
    with (OUTPUT / 'steady-windows.csv').open('w', newline='') as file:
        writer = csv.DictWriter(file, fieldnames=fields)
        writer.writeheader()
        writer.writerows(windows)
write('analysis-input.json', {'source': summary['input'], 'scriptSha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                            'quantileMethod': 'nearest rank',
                            'fpsMethod': 'sum of counter deltas / sum of actual sampleWindowMs; foreground samples fully contained in active scene intervals after first completion'})
for report in summary['attempts']:
    keys = ['gpuToSlotReleaseMs', 'gpuToMainActorMs', 'frameFeedbackWorkMs', 'displayTelemetryWorkMs',
            'mainActorWorkBeforeTraceMs', 'gpuToTraceRecordMs', 'submitToGpuCompletionMs',
            'mainActorQueueSharePercent', 'foreground.framesDecodedDeltaRate', 'foreground.presentCountDeltaRate',
            'foreground.drawablePresentCountDeltaRate', 'rtcHealthSnapshot.rttMs', 'rtcHealthSnapshot.jitterMs',
            'displayHealthSnapshot.submitToDrawablePresentP95Ms']
    print(json.dumps({'attempt': report['attempt'], 'metrics': {k: report['metrics'].get(k) for k in keys},
                      'foregroundSegments': report['foregroundSegments'], 'stateCounts': report['stateCounts'], 'drops': report['drops'],
                      'lastRtc': {k: report['lastRtc'].get(k) for k in ['framesDecoded', 'framesReceived', 'framesDropped', 'packetsReceived',
                                                                     'packetsLost', 'freezeCount', 'freezeDurationMs', 'nackCount', 'pliCount',
                                                                     'firCount', 'selectedCandidatePairProtocol', 'selectedLocalCandidateType', 'selectedRemoteCandidateType']},
                      'lastDisplay': {k: (report['lastDisplay'] or {}).get(k) for k in ['submitCount', 'presentCount', 'drawablePresentCount', 'mailboxOverwriteCount', 'dropCountByReason']},
                      'networkPressureCounts': report['networkPressureCounts'], 'qualityStateCounts': report['qualityStateCounts'],
                      'connectionRecoveryCounts': report['connectionRecoveryCounts']}, ensure_ascii=False))
