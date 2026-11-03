"""复算 GPU 补供帧新日志，保留同代际窗口和冷启动失败证据。"""

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
BASE = ROOT / 'dev-docs/validation/2026-10-30-frame-credit-window-and-pacing-fresh-trace-validation.py'
SKILL = ROOT / '.agents/skills/analyze-runtime-logs/scripts'
LOG = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(
    '/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793612198686-1.jsonl')
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


def distribution(values):
    values = sorted(v for v in values if isinstance(v, (int, float)) and math.isfinite(v))

    def q(ratio):
        if not values:
            return None
        position = (len(values) - 1) * ratio
        a, b = math.floor(position), math.ceil(position)
        return values[a] + (values[b] - values[a]) * (position - a)

    return dict(n=len(values), p50=q(.5), p95=q(.95), p99=q(.99),
                min=min(values) if values else None, max=max(values) if values else None)


trace_hash = sha(LOG)
with tempfile.TemporaryDirectory(prefix='xbxrc-sr-gpu-fresh-') as folder:
    base_path = Path(folder) / 'base.json'
    code, _ = run(BASE, str(base_path))
    assert code == 0
    base = json.loads(base_path.read_text())
code, text = run(SKILL / 'summarize_runtime_trace.py', '--json', '--sample-rows', '0', '--top-suspicious', '0')
assert code == 0
summary = json.loads(text)
receive_code, receive_text = run(SKILL / 'trace_receive_feedback_report.py', '--fail-on-gate',
                                '--require-media-recovered', '--require-display-stable')
receive = json.loads(receive_text)
mid_code, mid_text = run(SKILL / 'trace_midsegment_report.py')

rows = [json.loads(line) for line in LOG.read_text().splitlines() if line.strip()]
by_seq = {row['seq']: row for row in rows}
assert len(by_seq) == len(rows) == base['rows'] == summary['rows']
assert base['traceSha256'] == trace_hash
telemetry = [r for r in rows if r['event'] == 'renderTelemetryObserved']
stats = [r for r in rows if r['event'] == 'browserWebRtcStatsObserved']
consumed = [r for r in rows if r['event'] == 'browserObservationConsumed'
            and r['payload'].get('consumer') == 'supervision']
resources = [r for r in rows if r['event'] in ['browserMediaResourcesObserved', 'browserMediaResourcesSnapshot']]
event_counts = collections.Counter(r['event'] for r in rows)
coverage = ['receivedCoverageMs', 'decodeCoverageMs', 'callbackCoverageMs', 'rendererCoverageMs', 'videoCoverageMs']

phases, checked_windows = [], 0
for phase in base['visibilityRendererPhases']:
    item = select(phase, ['rendererGeneration', 'kind', 'visible', 'startMs', 'endMs',
                         'liveDelta', 'network', 'windows', 'callbackSampleTotals', 'maxCallbackIntervalMs'])
    item['expectedDisplayLeadMs'] = phase['identity']['expectedDisplayLeadMs']
    for length, window in item['windows'].items():
        length = int(length)
        label = 'short' if length == 3000 else 'long'
        selected = [r for r in consumed if r['payload'].get('rendererGeneration') == phase['rendererGeneration']
                    and phase['startMs'] + length <= r['payload']['sampledAtMs'] <= phase['endMs']
                    and all(r['payload']['facts']['windows'][label].get(k, 0) >= length - 1 for k in coverage)]
        assert len(selected) == window['n']
        for field, expected in window['fields'].items():
            actual = distribution([r['payload']['facts']['windows'][label].get(field) for r in selected])['p50']
            assert actual is None and expected['p50'] is None or math.isclose(actual, expected['p50'])
        checked_windows += len(selected)
        window.pop('rows', None)
    live = item['liveDelta']
    if live:
        a, b = [by_seq[seq]['payload']['mediaDiagnostics'] for seq in live['seq']]
        assert a['rendererGeneration'] == b['rendererGeneration'] == phase['rendererGeneration']
        assert a['frameIdentity']['videoGeneration'] == b['frameIdentity']['videoGeneration']
        assert b['frameIdentity']['localSequence'] - a['frameIdentity']['localSequence'] == live['candidate']
        clocks = {}
        for source in ['videoFrameCallback', 'animationFrame']:
            ta, tb = [d['frameIdentity']['callbackTiming'][source] for d in [a, b]]
            delta = {k: tb[k] - ta[k] for k in ['count', 'intervalCount', 'intervalTotalMs', 'clockLagCount', 'clockLagTotalMs']}
            assert delta['count'] == live['observed'][source]
            clocks[source] = dict(delta, observedFps=delta['count'] / live['durationSeconds'],
                                 meanIntervalMs=delta['intervalTotalMs'] / delta['intervalCount'] if delta['intervalCount'] else None,
                                 meanEntryClockLagMs=delta['clockLagTotalMs'] / delta['clockLagCount'] if delta['clockLagCount'] else None)
        pa, pb = [d['frameIdentity']['videoPixels'] for d in [a, b]]
        item['pixelDelta'] = {k: pb[k] - pa[k] for k in ['readCount', 'baselineCount', 'pairCount', 'changedPairCount', 'deliveredFrameCount']}
        item['callbackClocks'] = clocks
        if live['submitted'] is not None:
            assert b['submittedFramesTotal'] - a['submittedFramesTotal'] == live['submitted']
            item['submittedFps'] = live['submitted'] / live['durationSeconds']
            item['candidateDeliveryRatio'] = live['submitted'] / live['candidate']
    phases.append(item)

pixels = telemetry[-1]['payload']['mediaDiagnostics']['frameIdentity']['videoPixels']
first_pixels = next((r for r in telemetry if r['payload']['mediaDiagnostics']['frameIdentity']['videoPixels']['readCount']), None)
timing = pixels['readTiming']
assert pixels['readCount'] == pixels['completedReadCount'] + pixels['cancelledReadCount'] + pixels['pixelProbeGpuReads']
assert timing['coldReads']['count'] + timing['warmReads']['count'] == pixels['completedReadCount']
assert math.isclose(timing['coldReads']['totalMs'] + timing['warmReads']['totalMs'], pixels['readDurationTotalMs'])
assert math.isclose(sum(timing[k]['totalMs'] for k in ['initialization', 'upload', 'readback']), pixels['readDurationTotalMs'])
first_read = timing.get('firstRead')
if first_read:
    assert math.isclose(first_read['initializationMs'] + first_read['uploadMs'] + first_read['readbackMs'], first_read['durationMs'])

first_diag = telemetry[0]['payload']['mediaDiagnostics']
commit = next(r for r in resources if r['payload']['reason'] == 'rendererCommitted'
              and r['payload']['resources']['rendererObservation']['generation'] == first_diag['rendererGeneration'])
observation = commit['payload']['resources']['rendererObservation']
seconds = (first_diag['observedAtMs'] - commit['payload']['observedAtMs']) / 1000
collection_boundary = dict(seq=[commit['seq'], telemetry[0]['seq']], durationSeconds=seconds,
    collectionStartedAtMs=first_diag['frameIdentity']['collectionStartedAtMs'],
    submitted=first_diag['submittedFramesTotal'] - observation['resources']['realtimeDrawCount'],
    candidates=first_diag['frameIdentity']['localSequence'] - observation['latestVideoSequence'])
collection_boundary['submittedFps'] = collection_boundary['submitted'] / seconds

lifecycle = []
for r in resources:
    if r['payload']['reason'] != 'rendererCommitted':
        continue
    o = r['payload']['resources']['rendererObservation']
    life, p = o['lifecycle'], o['resources'].get('preparation', {})
    durations = dict(requestToCommitMs=life['committedAtMs'] - life['requestedAtMs'])
    if p:
        durations.update(setupMs=p['finishedAtMs'] - p['startedAtMs'],
                         setupToCandidateMs=p['firstCandidateAtMs'] - p['finishedAtMs'],
                         firstDrawMs=p['firstDrawSubmittedAtMs'] - p['firstDrawStartedAtMs'])
    assert all(v >= 0 for v in durations.values())
    lifecycle.append(dict(seq=r['seq'], generation=o['generation'], kind=o['kind'],
                          durations=durations, preparation=p, lifecycle=life))

render_paths = ['src/player/infra/render/video-frame-source.ts', 'src/player/infra/render/video-frame-pixel-observation.ts',
                'src/player/infra/render/SuperResolutionWebGL2Renderer.ts', 'src/player/infra/render/frame-pacer.ts']
build_commit = base['build']['gitCommitShort']
render_sources = []
for relative in render_paths:
    built = subprocess.check_output(['git', 'show', f'{build_commit}:{relative}'], cwd=ROOT)
    current = (ROOT / relative).read_bytes()
    render_sources.append(dict(path=relative, buildCommitSha256=hashlib.sha256(built).hexdigest(),
                               currentSha256=hashlib.sha256(current).hexdigest(), matchesBuildCommit=built == current))

ages = [r['payload']['rendererSubmitAgeMs'] for r in telemetry if r['payload'].get('rendererKind') != 'video'
        and isinstance(r['payload'].get('rendererSubmitAgeMs'), (int, float))]
closed = next(r for r in resources if r['payload']['reason'] == 'clientClosed')
steady = [{**select(p, ['rendererGeneration', 'startMs', 'endMs']),
           'window': {k: v for k, v in p['windows'].items() if k != 'rows'}} for p in base['steadyForegroundWindows']]
baseline_path = ROOT / 'dev-docs/validation/2026-11-02-pixel-cost-fresh-trace-analysis.metrics.json'
previous = json.loads(baseline_path.read_text())

report = dict(analysisDate='2026-11-03', trace=str(LOG), traceSha256=trace_hash, rows=len(rows),
    traceBounds={**base['traceBounds'], 'wallClock': [datetime.fromtimestamp(r['tsMs'] / 1000, ZoneInfo('Asia/Shanghai')).isoformat() for r in [rows[0], rows[-1]]]},
    build=base['build'], sourceMatchesCurrent=base['sourceMatchesCurrent'], sourceInputCount=base['sourceInputCount'],
    renderSources=render_sources, network=base['network'], foregroundMedia=base['foregroundMedia'], phases=phases,
    recoveredVideoWindows=steady, collectionBoundary=collection_boundary, lifecycle=lifecycle,
    pixels=dict(firstObservedSeq=first_pixels['seq'] if first_pixels else None, latest=pixels),
    combinedSteadyDrawCosts=base['combinedDrawCosts'], rendererDrawCosts=base['rendererDrawCosts'],
    submitAgeMs=distribution(ages), resourceSubmitAgeMs=base['srLiveSubmitAgeMs'],
    freezeTimeline=base['freezeTimeline'], networkBurstAnchors=base['networkBurstAnchors'],
    telemetrySegments=[select(s, ['key', 'sampleTotals', 'trackingSourceCounts', 'renderCauseCounts']) for s in base['segments']],
    recovery=dict(strictExitCode=receive_code, receiveReport=receive,
        structuredEvents={e: event_counts[e] for e in ['pictureRecoveryTransition', 'pictureRecoveryBlockerObserved',
            'videoIngressTermination', 'firstFrameLatencyObserved', 'h264InspectionObserved', 'h264InspectionRejected', 'referenceChainStateChanged']},
        audit=summary['recoveryAudit'], interpretation='逐请求、ReferenceChain、NACK有效性及综合评分为缺证；零样本派生零分按MISSING解释。'),
    gates=base['gates'], nativeMidsegment=dict(exitCode=mid_code, report=mid_text), schema=summary['counts'],
    traceQueue=base['traceQueue'], actions=[select(r, ['seq', 'tsMs', 'event', 'payload']) for r in rows
        if r['event'] in ['renderLocalActionPlanned', 'renderLocalActionEffectEvaluated']],
    closedResources=select(closed, ['seq', 'payload']),
    comparison=dict(reference=str(baseline_path.relative_to(ROOT)), sha256=sha(baseline_path),
        network=previous['network'], sr=[select(p, ['rendererGeneration', 'windows', 'submittedFps']) for p in previous['phases'] if p['kind'] != 'video'],
        scope='独立会话按renderer/前台/完整覆盖窗口比较，运行顺序和网络扰动分别结算。'),
    verification=dict(status='PASS', uniqueSequences=len(by_seq), fullyCoveredWindowSamples=checked_windows,
        liveDeltas='PASS', pixelTimingTotals='PASS', lifecycleOrdering='PASS', traceHashStable='PASS'),
    scriptsSha256={str(p.relative_to(ROOT)): sha(p) for p in [Path(__file__), BASE, SKILL / 'summarize_runtime_trace.py']},
    limitations=['SR成功本地提交、浏览器video推进和物理屏幕呈现分别结算；GPU执行/submit-to-present/glass-to-glass缺证。',
        'GPU首读超冷预算停用，warm读取、像素配对及实际补帧效果缺样本。',
        '诊断启用前首读次数0；probe eligibility与页面可见性/暂停状态的连续事实需补齐。',
        'SR为1440p等尺寸passthrough，实际放大和SR完整15s窗口继续待验。',
        '全源码指纹随当前input改动变化；四个渲染模块与采样commit逐文件匹配，运行时JS产物指纹继续待补。'])
assert sha(LOG) == trace_hash
OUTPUT.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
print(json.dumps(dict(output=str(OUTPUT), verification=report['verification'],
    gpuDisabledReason=pixels.get('disabledReason'), gpuDelivered=pixels['deliveredFrameCount'],
    sr=[dict(generation=p['rendererGeneration'], shortFps=p['windows']['3000']['fields']['rendererSubmitFpsAvg']['p50'])
        for p in phases if p['kind'] != 'video'], submitAgeMs=report['submitAgeMs'], lifecycle=lifecycle), ensure_ascii=False, indent=2))
