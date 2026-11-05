"""复算最新帧调度实机日志，并用生产FramePacer检查时间槽冲突。"""

import collections
import hashlib
import json
import math
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BASE = ROOT / 'dev-docs/validation/2026-10-30-frame-credit-window-and-pacing-fresh-trace-validation.py'
SKILL = ROOT / '.agents/skills/analyze-runtime-logs/scripts'
LOG = Path(sys.argv[1])
OUTPUT = Path(sys.argv[2]) if len(sys.argv) > 2 else Path(__file__).with_suffix('.metrics.json')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def select(data, keys):
    return {key: data[key] for key in keys if key in data}


def distribution(values):
    values = sorted(x for x in values if isinstance(x, (float, int)) and math.isfinite(x))

    def quantile(p):
        if not values:
            return None
        position = (len(values) - 1) * p
        lower, upper = math.floor(position), math.ceil(position)
        return values[lower] + (values[upper] - values[lower]) * (position - lower)

    return dict(n=len(values), p50=quantile(.5), p95=quantile(.95), p99=quantile(.99),
                min=values[0] if values else None, max=values[-1] if values else None)


def source_fingerprint():
    # 与build.rs使用相同输入范围、路径排序及字节编码。
    inputs = ['src', 'src-tauri/src', 'src-tauri/build.rs', 'src-tauri/tauri.conf.json',
              'src-tauri/Cargo.toml', 'crates', 'Cargo.toml', 'app-edition.json',
              '.cargo/config.toml', 'Cargo.lock', 'rust-toolchain.toml', 'package.json',
              'pnpm-lock.yaml', 'vite.config.ts', 'tsconfig.json', 'tsconfig.node.json',
              'tsconfig.app.json']
    paths = sorted(set(path for path in subprocess.check_output(
        ['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard', '--', *inputs],
        cwd=ROOT).decode().split('\0') if path))
    fingerprint = hashlib.sha256()
    for relative in paths:
        name = relative.encode()
        fingerprint.update(len(name).to_bytes(8, 'little'))
        fingerprint.update(name)
        path = ROOT / relative
        if path.exists():
            data = path.read_bytes()
            fingerprint.update(b'\1')
            fingerprint.update(len(data).to_bytes(8, 'little'))
            fingerprint.update(data)
        else:
            fingerprint.update(b'\0')
    return fingerprint.hexdigest()


def script_report(script, *args):
    result = subprocess.run([sys.executable, '-B', str(script), str(LOG), *args],
                            cwd=ROOT, capture_output=True, text=True)
    if result.returncode not in (0, 2):
        raise RuntimeError(result.stderr or result.stdout)
    return result.returncode, result.stdout


trace_hash = sha(LOG)
if len(sys.argv) > 3:
    base = json.loads(Path(sys.argv[3]).read_text())
    assert base['freshAnalysisScriptSha256'] == sha(BASE)
else:
    with tempfile.TemporaryDirectory(prefix='xbxrc-latest-scheduling-analysis-') as directory:
        cached = Path(directory) / 'base.json'
        code, _ = script_report(BASE, str(cached))
        assert code == 0
        base = json.loads(cached.read_text())
assert base['traceSha256'] == trace_hash

raw = LOG.read_bytes()
assert raw.endswith(b'\n')
rows = [json.loads(line) for line in raw.decode().splitlines() if line.strip()]
by_seq = {row['seq']: row for row in rows}
assert len(by_seq) == len(rows) == base['rows']
assert [row['seq'] for row in rows] == list(range(rows[0]['seq'], rows[-1]['seq'] + 1))
required = {'schemaVersion', 'seq', 'traceRunId', 'tsMs', 'traceMode', 'traceProfile',
            'dimension', 'importance', 'category', 'domain', 'event', 'sessionId', 'payload'}
assert all(required <= row.keys() and row['schemaVersion'] == 3 for row in rows)
events = collections.Counter(row['event'] for row in rows)
telemetry = [row for row in rows if row['event'] == 'renderTelemetryObserved']
stats = [row for row in rows if row['event'] == 'browserWebRtcStatsObserved']
resources = [row for row in rows if row['event'] in ['browserMediaResourcesObserved', 'browserMediaResourcesSnapshot']]
sampling = [row for row in rows if row['event'] == 'gamepadSamplingObserved']
delivery = [row for row in rows if row['event'] == 'gamepadInputDeliveryObserved']
build = next(row['payload']['buildFingerprint'] for row in rows if row['event'] == 'runtimeBuildInfo')
current_fingerprint = source_fingerprint()
plan = next(row['payload'] for row in rows if row['event'] == 'compiledPlan')

phases = []
for phase in base['visibilityRendererPhases']:
    item = select(phase, ['rendererGeneration', 'kind', 'visible', 'startMs', 'endMs',
                         'callbackSampleTotals', 'maxCallbackIntervalMs'])
    item['network'] = select(phase['network'], ['receivedFps', 'decodedFps', 'durationSeconds',
                                               'counterChanges', 'weightedDelayMs'])
    item['windows'] = {length: {key: value for key, value in window.items() if key != 'rows'}
                       for length, window in phase['windows'].items()}
    item['liveDelta'] = phase['liveDelta']
    live = item['liveDelta']
    if live:
        first, last = [by_seq[seq]['payload']['mediaDiagnostics'] for seq in live['seq']]
        assert first['rendererGeneration'] == last['rendererGeneration'] == phase['rendererGeneration']
        assert first['frameIdentity']['videoGeneration'] == last['frameIdentity']['videoGeneration']
        assert last['frameIdentity']['localSequence'] - first['frameIdentity']['localSequence'] == live['candidate']
        item['liveObservedAtMs'] = [first['observedAtMs'], last['observedAtMs']]
        item['candidateFps'] = live['candidate'] / live['durationSeconds']
        if live['submitted'] is not None:
            assert last['submittedFramesTotal'] - first['submittedFramesTotal'] == live['submitted']
            item['submitFps'] = live['submitted'] / live['durationSeconds']
            item['candidateDeliveryRatio'] = live['submitted'] / live['candidate'] if live['candidate'] else None
        a, b = [data['frameIdentity']['videoPixels'] for data in [first, last]]
        item['pixelDelta'] = {key: b.get(key, 0) - a.get(key, 0) for key in [
            'readCount', 'completedReadCount', 'pairCount', 'changedPairCount', 'deliveredFrameCount']}
    phases.append(item)

draw_costs = []
accounting = []
for cost in base['rendererDrawCosts']:
    item = select(cost, ['rendererGeneration', 'rendererKind', 'seq', 'durationSeconds', 'count', 'costMsAvg'])
    item['lastResources'] = select(cost['last'], ['realtimeDrawCount', 'submitFailureCount', 'latestOverwriteCount',
                                                 'duplicateDrawCount', 'rateLimitDropCount', 'frameRequests',
                                                 'animationCallbacks', 'gpuTiming', 'timing', 'preparation',
                                                 'outputWidth', 'outputHeight'])
    draw_costs.append(item)
    row = by_seq[cost['seq'][-1]]
    p = row['payload']
    r = next(r for r in p['resources']['renderers'] if r['generation'] == cost['rendererGeneration'])
    # 资源中的源回调累计与同一单调时刻的即时身份交叉校验，包含renderer的初始缓存候选。
    matches = [t for t in telemetry if t['payload']['mediaDiagnostics']['rendererGeneration'] == r['generation']
               and t['payload']['mediaDiagnostics']['observedAtMs'] == p['observedAtMs']]
    assert matches
    local_sequence = matches[-1]['payload']['mediaDiagnostics']['frameIdentity']['localSequence']
    assert local_sequence == p['resources']['nativeCallbackCount']
    assert p['resources']['polledFrameCount'] == 0
    candidates = local_sequence - r['preparation']['firstCandidateSequence'] + 1
    assert candidates == r['realtimeDrawCount'] + r['rateLimitDropCount']
    assert r['duplicateDrawCount'] == r['submitFailureCount'] == 0
    accounting.append(dict(seq=row['seq'], matchingIdentitySeq=matches[-1]['seq'],
                           observedAtMs=p['observedAtMs'], rendererGeneration=r['generation'],
                           firstCandidateSequence=r['preparation']['firstCandidateSequence'],
                           sourceLocalSequence=local_sequence, sourceNativeCallbacksTotal=p['resources']['nativeCallbackCount'],
                           lastDrawSequence=r['lastDrawSequence'], candidatesIncludingInitialCache=candidates,
                           submitted=r['realtimeDrawCount'], rateLimitDropped=r['rateLimitDropCount'],
                           rateLimitDropRatio=r['rateLimitDropCount'] / candidates, accountingGate='PASS'))

entry_pairs = []
seen = set()
for phase, cost in zip([p for p in phases if p['kind'] == 'webgl2_sr'], draw_costs):
    assert phase['rendererGeneration'] == cost['rendererGeneration']
    anchor = cost['lastResources']['preparation']['firstDrawStartedAtMs']
    for row in telemetry:
        d = row['payload']['mediaDiagnostics']
        if d['rendererGeneration'] != phase['rendererGeneration']:
            continue
        for e in d['frameIdentity']['evidence']:
            key = (d['rendererGeneration'], e['trackingSource'], e.get('callbackEnteredAtMs'))
            t, gap = e.get('callbackEnteredAtMs'), e.get('callbackIntervalMs')
            if key in seen or not e['newFrame'] or e['trackingSource'] != 'videoFrameCallback' or gap is None:
                continue
            seen.add(key)
            if t - gap <= anchor or t >= phase['endMs']:
                continue
            entry_pairs.append(dict(seq=row['seq'], rendererGeneration=d['rendererGeneration'],
                                    localSequence=e['localSequence'], anchorMs=anchor,
                                    previousCallbackEnteredAtMs=t - gap, callbackEnteredAtMs=t,
                                    callbackIntervalMs=gap, nativeDelta=e.get('nativeDelta'),
                                    rtpTimestampState=e.get('rtpTimestampState')))

# 转译生产类，用部分入口时序与明确标记的模拟节拍检验相位敏感性。
node_code = r"""
const fs = require('node:fs')
const path = require('node:path')
const vm = require('node:vm')
const ts = require('typescript')
const input = JSON.parse(fs.readFileSync(0, 'utf8'))
const moduleObject = { exports: {} }
const code = ts.transpileModule(fs.readFileSync(path.join(input.root, 'src/player/infra/render/frame-pacer.ts'), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText
vm.runInNewContext(code, { exports: moduleObject.exports, module: moduleObject })
const { FramePacer } = moduleObject.exports
const entryPairs = input.entryPairs.map(pair => {
  const pacer = new FramePacer()
  const times = [pair.anchorMs, pair.previousCallbackEnteredAtMs, pair.callbackEnteredAtMs]
  const admitted = times.map(t => pacer.admit(t))
  const slots = times.map(t => Math.round((t - pair.anchorMs + 1) * 60 / 1000))
  return { ...pair, slots, admitted, sameSlotCollision: slots[1] === slots[2] }
})
function replay(name, sourceFps, phaseShiftMs, jitterMs) {
  const period = 1000 / sourceFps
  const times = Array.from({ length: sourceFps * 10 }, (_, i) => Math.round(i * period
    + (i >= sourceFps ? phaseShiftMs + (i % 2 ? jitterMs : -jitterMs) : 0)))
  const pacer = new FramePacer()
  const accepted = times.filter(t => pacer.admit(t))
  return { name, synthetic: true, sourceFps, targetFps: 60, phaseShiftMs, jitterMs,
    offered: times.length, submitted: accepted.length, rateLimitDropped: times.length - accepted.length,
    measuredOfferedFps: (times.length - 1) * 1000 / (times.at(-1) - times[0]),
    maxSubmissionGapMs: Math.max(...accepted.slice(1).map((t, i) => t - accepted[i])),
    finalCandidateAccepted: accepted.at(-1) === times.at(-1) }
}
process.stdout.write(JSON.stringify({ productionSource: 'src/player/infra/render/frame-pacer.ts', entryPairs,
  syntheticCases: [replay('stable60', 60, 0, 0), replay('phaseShiftWith1msJitter60', 60, 1000 / 60 / 2 - 1, 1),
    replay('underBudget53With4msJitter', 53, 7, 4)] }))
"""
replay_process = subprocess.run(['node', '-e', node_code], cwd=ROOT, capture_output=True, text=True,
                                input=json.dumps(dict(root=str(ROOT), entryPairs=entry_pairs)))
assert replay_process.returncode == 0, replay_process.stderr
pacer_replay = json.loads(replay_process.stdout)
assert pacer_replay['syntheticCases'][0]['rateLimitDropped'] == 0
assert all(case['rateLimitDropped'] > 0 for case in pacer_replay['syntheticCases'][1:])
pacer_replay['sameSlotEntryPairCount'] = sum(pair['sameSlotCollision'] for pair in pacer_replay['entryPairs'])
assert pacer_replay['sameSlotEntryPairCount'] > 0
pacer_replay['interpretation'] = ('回放使用抽样的native入口时间；admit调用时间与逐帧决策记录仍缺证。'
                                  '资源计数证明实机限帧损失，入口回放和模拟节拍证明生产规则存在相位冲突。')


def sampling_interval_report(start, end):
    intervals = []
    for first, last in zip(sampling, sampling[1:]):
        a, b = first['payload'], last['payload']
        elapsed = b['sampledAtMs'] - a['sampledAtMs']
        if elapsed > 0 and start <= a['sampledAtMs'] and b['sampledAtMs'] <= end:
            intervals.append(dict(seq=last['seq'], elapsedMs=elapsed, polls=b['pollCount'] - a['pollCount'],
                                  published=b['publishedCount'] - a['publishedCount'],
                                  pollingHz=(b['pollCount'] - a['pollCount']) * 1000 / elapsed,
                                  gap=select(b['gap'], ['p50Ms', 'p95Ms', 'p99Ms', 'maxMs'])))
    elapsed = sum(interval['elapsedMs'] for interval in intervals)
    return dict(boundsMs=[start, end], durationSeconds=elapsed / 1000, intervalCount=len(intervals),
                aggregatePollingHz=sum(x['polls'] for x in intervals) * 1000 / elapsed if elapsed else None,
                aggregatePublishedHz=sum(x['published'] for x in intervals) * 1000 / elapsed if elapsed else None,
                intervalPollingHz=distribution([x['pollingHz'] for x in intervals]),
                snapshotGapP95Ms=distribution([x['gap'].get('p95Ms') for x in intervals]),
                snapshotGapP99Ms=distribution([x['gap'].get('p99Ms') for x in intervals]),
                maximumRecentGapMs=max((x['gap'].get('maxMs', 0) for x in intervals), default=0),
                lowPollingIntervals=[x for x in intervals if x['pollingHz'] < 100])


sampling_peaks = []
previous_peak = 0
for row in sampling:
    p = row['payload']
    peak = p['gap'].get('maxObservedMs', 0)
    if peak > previous_peak:
        sampling_peaks.append(dict(seq=row['seq'], sampledAtMs=p['sampledAtMs'], peakMs=peak))
        previous_peak = peak
all_sampling = sampling_interval_report(stats[0]['payload']['sampledAtMs'], stats[-1]['payload']['sampledAtMs'])
stable_sampling = sampling_interval_report(telemetry[0]['payload']['sampledAtMs'] + 3000,
                                         stats[-1]['payload']['sampledAtMs'])
input_report = dict(samples=len(delivery), gamepadFramesSent=events['gamepadFrameSent'], latest=delivery[-1]['payload'],
                    currentValueDistributions={field: distribution([row['payload'].get(field) for row in delivery])
                                               for field in ['bufferedBytes', 'oldestBufferedAgeMs', 'pendingInputs', 'ledgerEntries', 'softLimitBytes']},
                    maximumObservedLatencyMs={field: max(row['payload'][field].get('maxObservedMs', 0) for row in delivery)
                                              for field in ['queueWait', 'observationToSend', 'physicalSampleAge', 'drainAge']},
                    percentileSemantics='latency快照分位数覆盖最近256项；maxObservedMs覆盖InputService生命周期')

last_pixel = telemetry[-1]['payload']['mediaDiagnostics']['frameIdentity']['videoPixels']
pixel_report = select(last_pixel, ['readCount', 'completedReadCount', 'baselineCount', 'pairCount',
                                   'changedPairCount', 'deliveredFrameCount', 'gpuErrorChecks', 'preparationState',
                                   'disabledReason', 'supplyMode', 'preparationTiming', 'readTiming', 'timing', 'discarded'])
first_read = last_pixel['readTiming']['firstRead']['readStartedAtMs']
pixel_duration = (telemetry[-1]['payload']['mediaDiagnostics']['observedAtMs'] - first_read) / 1000
pixel_report['firstReadToLastObservationSeconds'] = pixel_duration
pixel_report['averageReadsPerSecond'] = last_pixel['readCount'] / pixel_duration
closed = next(row for row in resources if row['payload']['reason'] == 'clientClosed')
zero_resources = {key: value for key, value in closed['payload']['resources'].items() if isinstance(value, (int, float))}
assert all(value == 0 for value in zero_resources.values())
consumers = collections.Counter(row['payload']['consumer'] for row in rows if row['event'] == 'browserObservationConsumed')
strict_code, strict_text = script_report(SKILL / 'trace_browser_webrtc_behavior_report.py', '--require-stability')
strict_behavior = json.loads(strict_text)
receive_code, receive_text = script_report(SKILL / 'trace_receive_feedback_report.py')
receive = json.loads(receive_text)
midsegment_code, midsegment_text = script_report(SKILL / 'trace_midsegment_report.py')
network = {key: value for key, value in base['network'].items() if key != 'pairIds'}
budgets = [dict(seq=r['seq'], **r['payload']) for r in rows if r['event'] == 'renderFpsBudgetChanged']

result = dict(
    analysisConsistencyGate='PASS', trace=str(LOG), traceSha256=trace_hash, bytes=len(raw), rows=len(rows),
    traceBounds=dict(firstTsMs=rows[0]['tsMs'], lastTsMs=rows[-1]['tsMs'],
                     durationSeconds=(rows[-1]['tsMs'] - rows[0]['tsMs']) / 1000),
    sourceIdentity=dict(build=build, currentSourceFingerprint=current_fingerprint,
                        sourceMatchesCurrent=current_fingerprint == build['sourceFingerprintSha256'],
                        fingerprintAlgorithm='src-tauri/build.rs source_fingerprint'),
    testContext=dict(targetType=plan['targetType'], input=plan['plan']['input'],
                     render=plan['plan']['render'], sceneAndOcclusion='当前日志缺少逐段场景/窗口遮挡的用户确认；沿日志visible和预算事实分析'),
    schema=dict(version=3, profiles=dict(collections.Counter(row['traceProfile'] for row in rows)),
                importance=dict(collections.Counter(row['importance'] for row in rows)),
                sequenceContiguous=True, writerBudget=rows[0]['payload']['budget'],
                writerThread=rows[0]['payload']['writerThread'], budgetNotices=events['traceBudgetNotice']),
    events=dict(events), network=network, rendererPhases=phases, rendererDrawCosts=draw_costs,
    rendererResourceAccounting=accounting, productionPacerReplay=pacer_replay, renderFpsBudgets=budgets,
    sourceCadence=dict(trackingSources=dict(collections.Counter(row['payload']['trackingSource'] for row in telemetry)),
                       unavailableReasons=dict(collections.Counter(row['payload'].get('sourceFpsUnavailableReason') for row in telemetry)),
                       sourceDimensions=sorted(set((row['payload']['sourceWidth'], row['payload']['sourceHeight']) for row in telemetry)),
                       outputDimensions=sorted(set((row['payload']['outputWidth'], row['payload']['outputHeight']) for row in telemetry)),
                       tier=[dict(seq=r['seq'], **r['payload']) for r in rows if r['event'] == 'superResolutionTierFrozen']),
    pixelProbe=pixel_report, sampling=dict(allMediaInterval=all_sampling, afterFirstTelemetry3s=stable_sampling,
                                          lifetimePeakAnchors=sampling_peaks), inputDelivery=input_report,
    startup=base['startup'], renderActions=base['renderActions'],
    frontendTraceQueue={field: max(row['payload']['trace'][field] for row in telemetry)
                        for field in ['pending', 'inFlight', 'dropped', 'droppedEssential']},
    cleanup=dict(seq=closed['seq'], observedAtMs=closed['payload']['observedAtMs'], zeroResources=zero_resources,
                 stages=[dict(seq=row['seq'], **row['payload']) for row in rows if row['event'] == 'cleanupFinished']),
    observationConsumers=dict(consumers),
    gates=dict(browserBehavior=strict_behavior['browserWebRtcBehaviorGate'],
               browserRuntimeStability=select(strict_behavior['browserRuntimeStability'], ['gate', 'failures', 'missingEvidence', 'evidence']),
               strictBrowser=dict(exitCode=strict_code, failures=strict_behavior['failures']),
               nativeReceiveFeedback=dict(exitCode=receive_code, summary=receive['summary'],
                                          applicability='Browser trace缺少native恢复链与NACK逐包结果；空证据门禁单独标记'),
               nativeMidsegment=dict(exitCode=midsegment_code, text=midsegment_text,
                                     applicability='statsSnapshot/host epoch缺证，使用上述完整3s/15s Browser同代际窗口'),
               deviceSrSupply='候选→提交存在限帧损失；两个SR阶段均不足15s，稳态增强满帧验收待完成'),
    sourceFiles={str(path.relative_to(ROOT)): sha(path) for path in [
        Path(__file__), BASE, ROOT / 'dev-docs/validation/2026-10-30-display-deficit-fresh-trace-validation.py',
        ROOT / 'src-tauri/build.rs', ROOT / 'src/player/infra/render/frame-pacer.ts',
        ROOT / 'src/player/infra/render/SuperResolutionWebGL2Renderer.ts',
        ROOT / 'src/player/infra/render/video-frame-source.ts',
        ROOT / 'src/player/infra/render/video-frame-identity-observation.ts',
        ROOT / 'src/player/infra/render/video-frame-pixel-observation.ts', ROOT / 'src/player/app/input/InputService.ts']},
    limitations=[
        '即时增量使用mediaDiagnostics即时观测时间；顶层周期计数存在采样时差，限帧配平使用同一资源时刻证据。',
        '固定时隙相位机制由生产回放验证；具体84帧的admit调用时间和逐帧决策在当前trace中缺证。',
        '两个SR阶段短于15s，完整3s/15s窗数量分别保留；Browser行为门禁与真实增强帧供给分别验收。',
        '本轮覆盖主机/SR同尺寸passthrough与video；普通锐化、云游戏与真实放大SR需要各自设备样本。',
        'GPU timer扩展unavailable，实际GPU执行与物理submit-to-present仍待实测。',
        '像素探测为dev低频诊断，12组有效配对变化0；逐段画面运动缺少场景标记。',
        'getGamepads调用频率、最新稳定状态心跳年龄和输入到画面响应分别解释；硬件/端到端时延仍待外部测量。',
        '串流前后存在应用级采样长间隔；媒体有效区间无低于100Hz的完整采样窗。',
        'Browser缺少ReferenceChain/RecoveryLedger/cleanAnchorCommitted/DisplayStable与NACK逐包结果，聚合反馈次数作为请求事实。',
    ])
assert sha(LOG) == trace_hash
OUTPUT.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
print(json.dumps(dict(output=str(OUTPUT), analysisConsistencyGate=result['analysisConsistencyGate'],
                      sourceMatchesCurrent=result['sourceIdentity']['sourceMatchesCurrent'], rows=len(rows),
                      resourceAccounting=accounting, pacerSameSlotPairs=pacer_replay['sameSlotEntryPairCount'],
                      syntheticPacerCases=pacer_replay['syntheticCases'],
                      pollingHz=all_sampling['aggregatePollingHz'],
                      strictBrowserRuntimeGate=result['gates']['browserRuntimeStability']['gate']), ensure_ascii=False))
