"""复算新像素成本日志，兼容先原生video、后SR的阶段序列。"""

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
BASE = ROOT / 'dev-docs/validation/2026-10-30-frame-credit-window-and-pacing-fresh-trace-validation.py'
LOG = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(
    '/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793588378449-1.jsonl')
OUTPUT = Path(sys.argv[2]) if len(sys.argv) > 2 else Path(__file__).with_suffix('.metrics.json')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(path, *args):
    result = subprocess.run([sys.executable, '-B', str(path), str(LOG), *args],
                            cwd=ROOT, capture_output=True, text=True)
    if result.returncode not in (0, 2):
        raise RuntimeError(result.stderr or result.stdout)
    return result.returncode, result.stdout


def select(data, keys):
    return {key: data[key] for key in keys if key in data}


def quantile(values, q):
    values = sorted(values)
    if not values:
        return None
    position = (len(values) - 1) * q
    a, b = math.floor(position), math.ceil(position)
    return values[a] + (values[b] - values[a]) * (position - a)


code, output = run(SKILL / 'summarize_runtime_trace.py', '--json', '--sample-rows', '0', '--top-suspicious', '0')
assert code == 0
summary = json.loads(output)
with tempfile.TemporaryDirectory(prefix='xbxrc-pixel-cost-fresh-') as folder:
    base_path = Path(folder) / 'base.json'
    code, _ = run(BASE, str(base_path))
    assert code == 0
    base = json.loads(base_path.read_text())

rows = [json.loads(line) for line in LOG.read_text().splitlines() if line.strip()]
by_seq = {row['seq']: row for row in rows}
assert len(by_seq) == len(rows) == base['rows'] == summary['rows']
assert base['traceSha256'] == sha(LOG)
stats = [row for row in rows if row['event'] == 'browserWebRtcStatsObserved']
telemetry = [row for row in rows if row['event'] == 'renderTelemetryObserved']
consumed = [row for row in rows if row['event'] == 'browserObservationConsumed'
            and row['payload'].get('consumer') == 'supervision']
resources = [row for row in rows if row['event'] in ['browserMediaResourcesObserved', 'browserMediaResourcesSnapshot']]
commits = [row for row in resources if row['payload'].get('reason') == 'rendererCommitted']
closed = next(row for row in resources if row['payload'].get('reason') == 'clientClosed')
coverage = ['receivedCoverageMs', 'decodeCoverageMs', 'callbackCoverageMs', 'rendererCoverageMs', 'videoCoverageMs']

phases, checked_windows = [], 0
for phase in base['visibilityRendererPhases']:
    item = select(phase, ['rendererGeneration', 'kind', 'visible', 'startMs', 'endMs', 'liveDelta', 'network', 'windows'])
    for length, window in item['windows'].items():
        length = int(length)
        label = 'short' if length == 3000 else 'long'
        selected = [row for row in consumed if row['payload'].get('rendererGeneration') == phase['rendererGeneration']
                    and phase['startMs'] + length <= row['payload']['sampledAtMs'] <= phase['endMs']
                    and all(row['payload']['facts']['windows'][label].get(field, 0) >= length - 1 for field in coverage)]
        assert len(selected) == window['n']
        for field, distribution in window['fields'].items():
            values = [row['payload']['facts']['windows'][label][field] for row in selected]
            actual = quantile(values, .5)
            expected = distribution['p50']
            assert actual is None and expected is None or math.isclose(actual, expected)
        checked_windows += len(selected)
        window.pop('rows', None)
    live = item['liveDelta']
    if live:
        a, b = [by_seq[seq]['payload']['mediaDiagnostics'] for seq in live['seq']]
        assert a['rendererGeneration'] == b['rendererGeneration'] == phase['rendererGeneration']
        assert b['frameIdentity']['localSequence'] - a['frameIdentity']['localSequence'] == live['candidate']
        if live['submitted'] is not None:
            assert b['submittedFramesTotal'] - a['submittedFramesTotal'] == live['submitted']
            item['candidateDeliveryRatio'] = live['submitted'] / live['candidate']
            item['submittedFps'] = live['submitted'] / live['durationSeconds']
        clocks = {}
        for source in ['videoFrameCallback', 'animationFrame']:
            ta, tb = [d['frameIdentity']['callbackTiming'][source] for d in [a, b]]
            delta = {key: tb[key] - ta[key] for key in ['count', 'intervalCount', 'intervalTotalMs', 'clockLagCount', 'clockLagTotalMs']}
            assert delta['count'] == live['observed'][source]
            clocks[source] = dict(delta, observedFps=delta['count'] / live['durationSeconds'], accepted=live['accepted'][source],
                                 meanIntervalMs=delta['intervalTotalMs'] / delta['intervalCount'] if delta['intervalCount'] else None)
        ca, cb = [d['frameIdentity']['counterCadence'] for d in [a, b]]
        cadence = {key: cb[key] - ca[key] for key in ['updateCount', 'intervalCount', 'intervalTotalMs', 'advanceTotal']}
        cadence['meanUpdateIntervalMs'] = cadence['intervalTotalMs'] / cadence['intervalCount'] if cadence['intervalCount'] else None
        item.update(callbackClocks=clocks, counterCadence=cadence)
    phases.append(item)

lifecycle = []
for row in commits:
    observation = row['payload']['resources']['rendererObservation']
    life = observation['lifecycle']
    preparation = observation['resources'].get('preparation', {})
    durations = {'requestToCommitMs': life['committedAtMs'] - life['requestedAtMs']}
    if preparation:
        durations.update(setupMs=preparation['finishedAtMs'] - preparation['startedAtMs'],
                         setupToFirstCandidateMs=preparation['firstCandidateAtMs'] - preparation['finishedAtMs'],
                         candidateToFirstDrawMs=preparation['firstDrawStartedAtMs'] - preparation['firstCandidateAtMs'],
                         firstDrawMs=preparation['firstDrawSubmittedAtMs'] - preparation['firstDrawStartedAtMs'])
    assert all(value >= 0 for value in durations.values())
    lifecycle.append(dict(seq=row['seq'], generation=observation['generation'], kind=observation['kind'],
                          lifecycle=life, preparation=preparation, durations=durations,
                          firstDrawCosts=select(observation['resources'], ['realtimeDrawCount', 'drawDurationTotalMs', 'uploadDurationTotalMs', 'submitDurationTotalMs'])))

pixel_rows = [row for row in telemetry if row['payload']['mediaDiagnostics']['frameIdentity']['videoPixels']['readCount'] > 0]
pixels = telemetry[-1]['payload']['mediaDiagnostics']['frameIdentity']['videoPixels']
read_timing = pixels.get('readTiming', {})
if read_timing:
    assert read_timing['coldReads']['count'] + read_timing['warmReads']['count'] == pixels['readCount']
    assert math.isclose(read_timing['coldReads']['totalMs'] + read_timing['warmReads']['totalMs'], pixels['readDurationTotalMs'])
    if 'firstRead' in read_timing:
        first = read_timing['firstRead']
        assert first['readFinishedAtMs'] - first['readStartedAtMs'] == first['durationMs']
pixel_phase = next((p['rendererGeneration'] for p in phases if 'firstRead' in read_timing
                    and p['startMs'] <= read_timing['firstRead']['readStartedAtMs'] < p['endMs']), None)
ages = [row['payload']['rendererSubmitAgeMs'] for row in telemetry if row['payload'].get('rendererKind') != 'video'
        and isinstance(row['payload'].get('rendererSubmitAgeMs'), (int, float))]
age_summary = dict(n=len(ages), p50=quantile(ages, .5), p95=quantile(ages, .95), p99=quantile(ages, .99), max=max(ages) if ages else None)

receive_code, receive_text = run(SKILL / 'trace_receive_feedback_report.py', '--fail-on-gate', '--require-media-recovered', '--require-display-stable')
receive = json.loads(receive_text)
mid_code, mid_text = run(SKILL / 'trace_midsegment_report.py')
recovery_events = ['pictureRecoveryTransition', 'pictureRecoveryBlockerObserved', 'videoIngressTermination',
                   'firstFrameLatencyObserved', 'h264InspectionObserved', 'h264InspectionRejected', 'referenceChainStateChanged']
event_counts = collections.Counter(row['event'] for row in rows)
evidence = {}
for row in telemetry:
    for item in row['payload']['mediaDiagnostics']['frameIdentity'].get('evidence', []):
        evidence[(item['atMs'], item['trackingSource'])] = item
paired = [item['pairedCounters'] for item in evidence.values() if item.get('pairedCounters')]
previous_path = ROOT / 'dev-docs/validation/2026-11-02-network-render-fresh-trace-analysis.metrics.json'
previous = json.loads(previous_path.read_text())
report = dict(trace=str(LOG), traceSha256=sha(LOG), rows=len(rows),
    traceBounds={**base['traceBounds'], 'wallClock': [datetime.fromtimestamp(row['tsMs'] / 1000, ZoneInfo('Asia/Shanghai')).isoformat() for row in [rows[0], rows[-1]]]},
    build=base['build'], sourceMatchesCurrent=base['sourceMatchesCurrent'], sourceInputCount=base['sourceInputCount'],
    network=select(base['network'], ['bounds', 'durationSeconds', 'counterChanges', 'receivedFps', 'decodedFps', 'bitrateMbps',
        'positiveLossRatio', 'rttMs', 'jitterMs', 'sampleDelayMs', 'weightedDelayMs', 'connectionStates', 'worstDelaySamples']),
    inboundIdentityCounts=dict(ssrc=len({r['payload']['inboundVideo']['ssrc'] for r in stats}), statsId=len({r['payload']['inboundVideo']['id'] for r in stats})),
    phases=phases, lifecycle=lifecycle, pixels=dict(firstObservedSeq=pixel_rows[0]['seq'] if pixel_rows else None,
        firstReadRendererGeneration=pixel_phase, latest=pixels),
    drawCosts=[select(p, ['rendererGeneration', 'seq', 'count', 'realtimeFps', 'costMsAvg']) for p in base['rendererDrawCosts']],
    combinedSteadyDrawCosts=base['combinedDrawCosts'], submitAgeMs=age_summary,
    freezeTimeline=base['freezeTimeline'], networkBurstAnchors=base['networkBurstAnchors'],
    telemetrySegments=[select(segment, ['key', 'sampleTotals', 'trackingSourceCounts', 'renderCauseCounts']) for segment in base['segments']],
    pairedCounts=dict(samples=len(paired), playbackQualityValid=sum(p.get('playbackQualityFrames') is not None for p in paired),
                      webkitValid=sum(p.get('webkitFrames') is not None for p in paired)),
    latestIdentity=select(telemetry[-1]['payload']['mediaDiagnostics']['frameIdentity'], ['observed', 'accepted', 'counterCadence', 'sampledCount']),
    recovery=dict(strictExitCode=receive_code, receiveReport=select(receive, ['receiveFeedbackGate', 'receiveFeedbackGateFailures', 'rates', 'keyframeChain']),
        structuredEvents={event: event_counts[event] for event in recovery_events}, audit=summary['recoveryAudit']),
    gates=base['gates'], nativeMidsegment=dict(exitCode=mid_code, report=mid_text), traceQueue=base['traceQueue'],
    schema=summary['counts'], events=dict(event_counts),
    actions=[dict(seq=row['seq'], tsMs=row['tsMs'], event=row['event'], payload=row['payload']) for row in rows
             if row['event'] in ['renderLocalActionPlanned', 'renderLocalActionEffectEvaluated']],
    closedResources=dict(seq=closed['seq'], observedAtMs=closed['payload']['observedAtMs'], resources=closed['payload']['resources']),
    comparison=dict(reference=str(previous_path.relative_to(ROOT)), sha256=sha(previous_path), network=previous['network'],
                    scope='两份独立会话，按阶段/同窗比较；运行顺序、网络与短时SR窗口分别保留。'),
    verification=dict(status='PASS', uniqueSequences=len(by_seq), fullyCoveredWindowSamples=checked_windows,
        liveDeltas='PASS', pixelTimingTotals='PASS', lifecycleOrdering='PASS', traceHashStable='PASS'),
    scriptsSha256={str(path.relative_to(ROOT)): sha(path) for path in [Path(__file__), BASE, SKILL / 'summarize_runtime_trace.py',
        SKILL / 'trace_receive_feedback_report.py', SKILL / 'trace_midsegment_report.py']},
    limitations=['browser接口的video推进与SR成功本地处理各自结算；真实scanout/GPU/submit-to-present继续待验。',
        '有效像素配对缺少时保持drawable推进缺证；readback同步耗时的GPU/拷贝细分继续待验。',
        '本次SR为1440p等尺寸passthrough，长期及实际放大效果路径继续待验。',
        '内部ReferenceChain/媒体锚点/显示闭环与逐NACK结果依赖额外浏览器内部事实。'])
assert sha(LOG) == report['traceSha256']
OUTPUT.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
print(json.dumps(dict(output=str(OUTPUT), verification=report['verification'], sourceMatchesCurrent=report['sourceMatchesCurrent'],
    networkFps=report['network']['decodedFps'], pixels=pixels,
    sr=[dict(generation=p['rendererGeneration'], live=p['liveDelta'], shortFps=p['windows']['3000']['fields']['rendererSubmitFpsAvg']['p50'])
        for p in phases if p['kind'] != 'video']), ensure_ascii=False, indent=2))
