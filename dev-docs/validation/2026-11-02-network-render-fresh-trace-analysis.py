"""复算最新 browser-direct 日志，补齐像素首读、准备分段与诊断采集边界。"""

import collections
import hashlib
import json
import math
import subprocess
import sys
import tempfile
from datetime import datetime
from pathlib import Path
from zoneinfo import ZoneInfo

ROOT = Path(__file__).resolve().parents[2]
SKILL = ROOT / '.agents/skills/analyze-runtime-logs/scripts'
BASE = ROOT / 'dev-docs/validation/2026-10-30-browser-frame-counter-pairing-fresh-trace-validation.py'
LOG = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(
    '/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793353382323-1.jsonl')
OUTPUT = Path(sys.argv[2]) if len(sys.argv) > 2 else Path(__file__).with_suffix('.metrics.json')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run_report(path, *args):
    result = subprocess.run([sys.executable, '-B', str(path), str(LOG), *args],
                            cwd=ROOT, capture_output=True, text=True)
    if result.returncode not in (0, 2):
        raise RuntimeError(result.stderr or result.stdout)
    return result.returncode, result.stdout


def select(value, keys):
    return {key: value[key] for key in keys if key in value}


def quantile(values, ratio):
    values = sorted(values)
    if not values:
        return None
    position = (len(values) - 1) * ratio
    low, high = math.floor(position), math.ceil(position)
    return values[low] + (values[high] - values[low]) * (position - low)


# 沿用已验收的同代际、完整覆盖窗口算法，独立核对其原始行和中位数。
with tempfile.TemporaryDirectory(prefix='xbxrc-network-render-fresh-') as folder:
    intermediate = Path(folder) / 'counter.json'
    subprocess.run([sys.executable, '-B', str(BASE), str(LOG), str(intermediate)],
                   cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
    base = json.loads(intermediate.read_text())

rows = [json.loads(line) for line in LOG.read_text().splitlines() if line.strip()]
assert base['traceSha256'] == sha(LOG)
assert base['pairedFreshAnalysisScriptSha256'] == sha(BASE)
by_seq = {row['seq']: row for row in rows}
assert len(by_seq) == len(rows)
stats = [row for row in rows if row['event'] == 'browserWebRtcStatsObserved']
telemetry = [row for row in rows if row['event'] == 'renderTelemetryObserved']
consumed = [row for row in rows if row['event'] == 'browserObservationConsumed'
            and row['payload'].get('consumer') == 'supervision']
resources = [row for row in rows if row['event'] in
             ['browserMediaResourcesObserved', 'browserMediaResourcesSnapshot']]
commits = [row for row in resources if row['payload'].get('reason') == 'rendererCommitted']
closed = next(row for row in resources if row['payload'].get('reason') == 'clientClosed')

phases = []
verified_windows = 0
coverage = ['receivedCoverageMs', 'decodeCoverageMs', 'callbackCoverageMs',
            'rendererCoverageMs', 'videoCoverageMs']
for phase in base['visibilityRendererPhases']:
    current = select(phase, ['rendererGeneration', 'kind', 'visible', 'startMs', 'endMs',
                            'liveDelta', 'network', 'windows', 'identity'])
    current['identity'].pop('evidence', None)
    for length, window in current['windows'].items():
        length = int(length)
        label = 'short' if length == 3000 else 'long'
        selected = [row for row in consumed
                    if row['payload'].get('rendererGeneration') == phase['rendererGeneration']
                    and phase['startMs'] + length <= row['payload']['sampledAtMs'] <= phase['endMs']
                    and all(row['payload']['facts']['windows'][label].get(key, 0) >= length - 1
                            for key in coverage)]
        assert len(selected) == window['n']
        for field, summary in window['fields'].items():
            values = [row['payload']['facts']['windows'][label][field] for row in selected]
            median = quantile(values, .5)
            assert median is None and summary['p50'] is None or math.isclose(median, summary['p50'])
        verified_windows += len(selected)
        # 保存窗口摘要及 seq 边界；逐秒原始供给从日志按 seq 复算。
        window.pop('rows', None)
    live = phase['liveDelta']
    if live:
        first, last = [by_seq[seq]['payload']['mediaDiagnostics'] for seq in live['seq']]
        assert first['rendererGeneration'] == last['rendererGeneration'] == phase['rendererGeneration']
        assert last['frameIdentity']['localSequence'] - first['frameIdentity']['localSequence'] == live['candidate']
        if live['submitted'] is not None:
            assert last['submittedFramesTotal'] - first['submittedFramesTotal'] == live['submitted']
        cadence = {key: last['frameIdentity']['counterCadence'][key]
                   - first['frameIdentity']['counterCadence'][key]
                   for key in ['updateCount', 'intervalCount', 'intervalTotalMs', 'advanceTotal']}
        cadence['meanUpdateIntervalMs'] = cadence['intervalTotalMs'] / cadence['intervalCount'] if cadence['intervalCount'] else None
        cadence['includesPreCollectionFirstInterval'] = first['frameIdentity']['counterCadence']['updateCount'] == 0
        current['counterCadenceDelta'] = cadence
    phases.append(current)

lifecycle = []
for row in commits:
    observation = row['payload']['resources']['rendererObservation']
    life = observation['lifecycle']
    preparation = observation.get('resources', {}).get('preparation', {})
    durations = {'requestToCommitMs': life['committedAtMs'] - life['requestedAtMs'],
                 'attachCallMs': life['attachCallReturnedAtMs'] - life['attachStartedAtMs'],
                 'attachAsyncWaitMs': life['attachCompletedAtMs'] - life['attachCallReturnedAtMs']}
    if preparation:
        durations.update(setupMs=preparation['finishedAtMs'] - preparation['startedAtMs'],
                         setupToFirstCandidateMs=preparation['firstCandidateAtMs'] - preparation['finishedAtMs'],
                         candidateToFirstDrawMs=preparation['firstDrawStartedAtMs'] - preparation['firstCandidateAtMs'],
                         firstDrawMs=preparation['firstDrawSubmittedAtMs'] - preparation['firstDrawStartedAtMs'])
    if 'previousReleasedAtMs' in life:
        durations['previousReleaseMs'] = life['previousReleasedAtMs'] - life['commitStartedAtMs']
    assert all(duration >= 0 for duration in durations.values())
    lifecycle.append(dict(seq=row['seq'], rendererGeneration=observation['generation'],
                          kind=observation['kind'], lifecycle=life, preparation=preparation,
                          durations=durations, firstDrawCosts=select(observation.get('resources', {}),
                          ['realtimeDrawCount', 'drawDurationTotalMs', 'uploadDurationTotalMs', 'submitDurationTotalMs'])))

first_diag = next(row for row in telemetry if row['payload'].get('mediaDiagnostics', {})
                  .get('frameIdentity', {}).get('collectionStartedAtMs') is not None)
first_live = first_diag['payload']['mediaDiagnostics']
first_commit = next(row for row in commits
                    if row['payload']['resources']['rendererObservation']['generation'] == first_live['rendererGeneration'])
first_resource = first_commit['payload']['resources']['rendererObservation']
seconds = (first_live['observedAtMs'] - first_commit['payload']['observedAtMs']) / 1000
pre_count = first_live['submittedFramesTotal'] - first_resource['resources']['realtimeDrawCount']
pre_candidate = first_live['frameIdentity']['localSequence'] - first_resource['latestVideoSequence']
pre_stats = [row for row in stats if first_commit['payload']['observedAtMs'] <= row['payload']['sampledAtMs'] <= first_live['observedAtMs']]
net_first, net_last = [row['payload'] for row in [pre_stats[0], pre_stats[-1]]]
net_seconds = (net_last['sampledAtMs'] - net_first['sampledAtMs']) / 1000
collection_boundary = dict(seq=[first_commit['seq'], first_diag['seq']],
    startMs=first_commit['payload']['observedAtMs'], collectionStartedAtMs=first_live['frameIdentity']['collectionStartedAtMs'],
    diagnosticObservedAtMs=first_live['observedAtMs'], durationSeconds=seconds,
    candidates=pre_candidate, submitted=pre_count, candidateFps=pre_candidate / seconds,
    submitFps=pre_count / seconds, rendererTargetFps=first_diag['payload']['rendererTargetFps'],
    networkSeq=[pre_stats[0]['seq'], pre_stats[-1]['seq']], networkDurationSeconds=net_seconds,
    decodedFps=(net_last['inboundVideo']['framesDecoded'] - net_first['inboundVideo']['framesDecoded']) / net_seconds,
    firstTelemetry=select(first_diag['payload'], ['callbackCountSinceLastSample', 'presentedFramesAdvancedSinceLastSample',
                           'presentedFramesJumpCountSinceLastSample', 'maxCallbackIntervalMsSinceLastSample']))

pixel_rows = [row for row in telemetry if row['payload'].get('mediaDiagnostics', {})
              .get('frameIdentity', {}).get('videoPixels', {}).get('readCount', 0) > 0]
pixels = telemetry[-1]['payload']['mediaDiagnostics']['frameIdentity']['videoPixels']
pixel_result = dict(firstObservedSeq=pixel_rows[0]['seq'] if pixel_rows else None,
                    firstObservedAtMs=pixel_rows[0]['payload']['observedAtMs'] if pixel_rows else None,
                    latest=pixels, observableAdvancement='MISSING' if pixels['pairCount'] == 0 else 'AVAILABLE')

summary_code, summary_text = run_report(SKILL / 'summarize_runtime_trace.py', '--json', '--sample-rows', '0', '--top-suspicious', '0')
assert summary_code == 0
summary = json.loads(summary_text)
receive_code, receive_text = run_report(SKILL / 'trace_receive_feedback_report.py', '--fail-on-gate',
                                       '--require-media-recovered', '--require-display-stable')
receive = json.loads(receive_text)
mid_code, mid_text = run_report(SKILL / 'trace_midsegment_report.py')
native = [row for row in rows if row['event'] == 'statsSnapshot']
missing_recovery = [key for key in ['cleanAnchorCommitted', 'displayStable'] if receive['keyframeChain'][key] == 0]
recovery = dict(strictExitCode=receive_code, strictEvidence='MISSING' if missing_recovery else 'AVAILABLE',
    missingRequiredStages=missing_recovery,
    receiveReport=select(receive, ['receiveFeedbackGate', 'receiveFeedbackGateFailures', 'summary', 'rates', 'keyframeChain']),
    structuredEvents={event: sum(row['event'] == event for row in rows) for event in
        ['pictureRecoveryTransition', 'pictureRecoveryBlockerObserved', 'videoIngressTermination',
         'firstFrameLatencyObserved', 'h264InspectionObserved', 'h264InspectionRejected', 'referenceChainStateChanged']},
    keyframeEffectiveness=summary['recoveryAudit']['keyframeEffectiveness'],
    nackEffectiveness=summary['recoveryAudit']['nackEffectiveness'],
    repairabilityPersistence=summary['recoveryAudit']['repairabilityPersistence'],
    rawCompositeScore=summary['recoveryAudit']['recoveryEffectiveness'],
    interpretation='浏览器内部逐请求结果和恢复闭环缺证；零样本派生的零分保留为缺证。')

draw_costs = [select(cost, ['rendererGeneration', 'rendererKind', 'seq', 'durationSeconds', 'count',
                          'realtimeFps', 'costMsAvg', 'countedCostSamples']) for cost in base['rendererDrawCosts']]
draw_count = sum(cost['count'] for cost in draw_costs)
for field, average in base['combinedDrawCosts']['avgMs'].items():
    total = sum(cost['count'] * cost['costMsAvg'][field] for cost in draw_costs)
    assert math.isclose(total / draw_count, average)

timeline = [dict(seq=row['seq'], tsMs=row['tsMs'], event=row['event'],
                 facts=select(row['payload'], ['observedAtMs', 'kind', 'reason', 'result', 'next', 'previous',
                              'targetFps', 'pipelineType', 'rendererGeneration', 'baselineRendererGeneration',
                              'elapsedSinceConnectedMs', 'connectedElapsedMs'])) for row in rows
            if row['event'] in ['renderPipelineSwitched', 'renderPolicyApplied', 'renderLocalActionPlanned',
                                'renderLocalActionEffectEvaluated', 'firstFrameStageChanged']]
feedback_counters = ['pliCount', 'firCount', 'nackCount', 'keyFramesDecoded', 'retransmittedPacketsReceived']
milestone_rows = [dict(seq=row['seq'], tsMs=row['tsMs'],
    facts=select(row['payload'], ['observedAtMs', 'kind', 'elapsedSinceConnectedMs'])) for row in rows
    if row['event'] == 'browserWebRtcTimelineObserved'
    and row['payload'].get('kind') in ['firstInboundPacket', 'firstDecoded', 'firstKeyframeDecoded', 'firstPresented']]
previous_path = ROOT / 'dev-docs/validation/2026-10-30-browser-frame-counter-pairing-fresh-trace-validation.metrics.json'
previous = json.loads(previous_path.read_text())
comparison = dict(reference=str(previous_path.relative_to(ROOT)), referenceSha256=sha(previous_path),
    traceSha256=previous['traceSha256'], network=select(previous['network'],
    ['durationSeconds', 'positiveLossRatio', 'receivedFps', 'decodedFps']),
    freezeCount=previous['network']['counterChanges']['freezeCount'],
    freezeDurationSeconds=previous['network']['counterChanges']['totalFreezesDuration'],
    steadyUploadMs=previous['same1440pDrawCosts']['costMsAvg']['uploadDurationTotalMs'],
    comparisonScope='同1440p源的独立会话；SR阶段长度与网络状态分别保留。')

data = dict(trace=str(LOG), traceSha256=sha(LOG), rows=len(rows),
    traceBounds={**base['traceBounds'], 'wallClock': [datetime.fromtimestamp(row['tsMs'] / 1000,
                 ZoneInfo('Asia/Shanghai')).isoformat() for row in [rows[0], rows[-1]]]},
    build=base['build'], sourceMatchesCurrent=base['sourceMatchesCurrent'], sourceInputCount=base['sourceInputCount'],
    currentSourceFingerprint=base['currentSourceFingerprint'], counts=summary['counts'],
    events=dict(collections.Counter(row['event'] for row in rows)),
    network={**select(base['network'], ['bounds', 'durationSeconds', 'counterChanges', 'receivedFps', 'decodedFps',
             'bitrateMbps', 'positivePacketsLost', 'positiveLossRatio', 'rttMs', 'jitterMs', 'sampleDelayMs',
             'weightedDelayMs', 'connectionStates']), 'inboundSsrcCount': len(base['inboundIdentity']['ssrcs']),
             'inboundStatsIdCount': len(base['inboundIdentity']['statsIds']),
             'firstSampleFeedback': select(stats[0]['payload']['inboundVideo'], feedback_counters),
             'lastSampleFeedback': select(stats[-1]['payload']['inboundVideo'], feedback_counters)},
    foregroundMedia=base['foregroundMedia'], backgroundMedia=base['backgroundMedia'],
    sourceDimensionTransitions=base['sourceDimensionTransitions'], phases=phases,
    collectionBoundary=collection_boundary, pixels=pixel_result, rendererLifecycle=lifecycle,
    drawCosts=draw_costs, combinedSteadyDrawCosts=base['combinedDrawCosts'],
    srSubmitAgeMs=base['srSubmitAgeOutsideFreezingPhases'], submitAgeTails=base['submitAgeTails'],
    callbackEvidence=base['callbackEvidence'],
    renderTelemetrySegments=[select(segment, ['key', 'sampleTotals', 'trackingSourceCounts', 'renderCauseCounts'])
                             for segment in base['segments']],
    pairedCounters={key: value for key, value in base['pairedCounterEvidence'].items() if key != 'sampleEvidence'},
    latestFrameIdentityCounts=base['latestFrameIdentityCounts'], collectionDurationMs=base['collectionDurationMs'],
    traceQueue=base['traceQueue'], recovery=recovery,
    gates=dict(browserBehavior=base['gates']['browserBehavior'], browserRuntimeStability=base['gates']['browserRuntimeStability'],
               nativeMidsegment=dict(exitCode=mid_code, report=mid_text,
               applicability='浏览器媒体会话；native统计单例没有host present epoch',
               nativePhaseCounts=dict(collections.Counter(row['payload'].get('session_phase') for row in native)),
               hostFrameEpochCounts=dict(collections.Counter(row['payload'].get('host_frame_present_epoch') for row in native)))),
    timeline=timeline, browserFirstFrameMilestones=milestone_rows,
    closedResources=dict(seq=closed['seq'], observedAtMs=closed['payload']['observedAtMs'], resources=closed['payload']['resources']),
    comparison=comparison,
    limitations=base['limitations'] + ['首包/解码milestone的elapsedSinceConnectedMs=0；首样本观测与实际首包分别解释。',
        '首段SR诊断启动前后约30/53fps存在时间关联；诊断与窗口状态的调度影响待对照。',
        'counterCadence首个区间跨越诊断启动；常态更新节拍采用后续同代际区间。',
        '源尺寸固定1440p，本次重建34ms对应同尺寸SR试探。'],
    verification=dict(status='PASS', uniqueSequences=len(by_seq), fullyCoveredWindowSamples=verified_windows,
        liveDeltas='PASS', weightedDrawCosts='PASS', lifecycleOrdering='PASS', traceHashStable='PASS'),
    scriptsSha256={str(path.relative_to(ROOT)): sha(path) for path in [Path(__file__), BASE,
        ROOT / 'dev-docs/validation/2026-10-30-frame-credit-window-and-pacing-fresh-trace-validation.py',
        ROOT / 'dev-docs/validation/2026-10-30-display-deficit-fresh-trace-validation.py',
        SKILL / 'summarize_runtime_trace.py', SKILL / 'trace_receive_feedback_report.py',
        SKILL / 'trace_browser_webrtc_behavior_report.py', SKILL / 'trace_midsegment_report.py']})
assert data['traceSha256'] == base['traceSha256']
assert data['rows'] == base['rows'] == summary['rows']
OUTPUT.write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n')
print(json.dumps(dict(output=str(OUTPUT), sourceMatchesCurrent=data['sourceMatchesCurrent'],
    verification=data['verification'], networkFps=data['network']['decodedFps'],
    srShortWindows=[dict(generation=p['rendererGeneration'], count=p['windows']['3000']['n'],
                        fps=p['windows']['3000']['fields']['rendererSubmitFpsAvg']['p50'])
                    for p in phases if p['kind'] == 'webgl2_sr'],
    pixelPairs=pixels['pairCount'], pixelReadMs=pixels['readDurationTotalMs'],
    lifecycle=[dict(generation=item['rendererGeneration'], durations=item['durations']) for item in lifecycle]),
    ensure_ascii=False, indent=2))
