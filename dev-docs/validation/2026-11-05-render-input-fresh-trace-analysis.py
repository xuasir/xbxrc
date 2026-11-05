"""复算渲染/输入新日志，复用同代际窗口并保留指标与观测边界。"""

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


def select(data, keys):
    return {key: data[key] for key in keys if key in data}


def source_fingerprint():
    # 完整复用build.rs身份算法，包含锁文件和新增源码。
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
    with tempfile.TemporaryDirectory(prefix='xbxrc-render-input-analysis-') as directory:
        output = Path(directory) / 'base.json'
        code, _ = script_report(BASE, str(output))
        assert code == 0
        base = json.loads(output.read_text())
assert base['traceSha256'] == trace_hash

rows = [json.loads(line) for line in LOG.read_text().splitlines() if line.strip()]
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

phases = []
pixel_fields = ['readCount', 'completedReadCount', 'baselineCount', 'pairCount',
                'changedPairCount', 'deliveredFrameCount', 'skippedHealthyReads', 'gpuErrorChecks']
for phase in base['visibilityRendererPhases']:
    item = select(phase, ['rendererGeneration', 'kind', 'visible', 'startMs', 'endMs',
                         'network', 'callbackSampleTotals', 'maxCallbackIntervalMs'])
    item['windows'] = {length: {key: value for key, value in window.items() if key != 'rows'}
                       for length, window in phase['windows'].items()}
    item['liveDelta'] = phase['liveDelta']
    live = item['liveDelta']
    if live:
        first, last = [by_seq[seq]['payload']['mediaDiagnostics'] for seq in live['seq']]
        assert first['rendererGeneration'] == last['rendererGeneration'] == phase['rendererGeneration']
        assert first['frameIdentity']['videoGeneration'] == last['frameIdentity']['videoGeneration']
        assert last['frameIdentity']['localSequence'] - first['frameIdentity']['localSequence'] == live['candidate']
        item['candidateFps'] = live['candidate'] / live['durationSeconds']
        if live['submitted'] is not None:
            assert last['submittedFramesTotal'] - first['submittedFramesTotal'] == live['submitted']
            item['submitFps'] = live['submitted'] / live['durationSeconds']
            item['candidateDeliveryRatio'] = live['submitted'] / live['candidate'] if live['candidate'] else None
        a, b = [data['frameIdentity']['videoPixels'] for data in [first, last]]
        item['pixelDelta'] = {key: b.get(key, 0) - a.get(key, 0) for key in pixel_fields}
        item['pixelLast'] = select(b, ['preparationState', 'disabledReason', 'supplyMode', 'timing'])
    phases.append(item)

draw_costs = []
for cost in base['rendererDrawCosts']:
    item = select(cost, ['rendererGeneration', 'rendererKind', 'seq', 'durationSeconds', 'count', 'costMsAvg'])
    item['lastResources'] = select(cost['last'], ['realtimeDrawCount', 'submitFailureCount', 'latestOverwriteCount',
                                                  'duplicateDrawCount', 'frameRequests', 'animationCallbacks',
                                                  'gpuTiming', 'timing', 'preparation', 'outputWidth', 'outputHeight'])
    draw_costs.append(item)


def sampling_interval_report(start, end):
    intervals = []
    for first, last in zip(sampling, sampling[1:]):
        a, b = first['payload'], last['payload']
        elapsed = b['sampledAtMs'] - a['sampledAtMs']
        if elapsed > 0 and start <= a['sampledAtMs'] and b['sampledAtMs'] <= end:
            intervals.append(dict(seq=last['seq'], startMs=a['sampledAtMs'], endMs=b['sampledAtMs'],
                                  elapsedMs=elapsed, polls=b['pollCount'] - a['pollCount'],
                                  published=b['publishedCount'] - a['publishedCount'],
                                  pollingHz=(b['pollCount'] - a['pollCount']) * 1000 / elapsed,
                                  gap=select(b['gap'], ['p50Ms', 'p95Ms', 'p99Ms', 'maxMs']),
                                  read=select(b['read'], ['p50Ms', 'p95Ms', 'p99Ms']),
                                  dispatch=select(b['dispatch'], ['p50Ms', 'p95Ms', 'p99Ms'])))
    elapsed = sum(interval['elapsedMs'] for interval in intervals)
    return dict(startMs=start, endMs=end, durationSeconds=elapsed / 1000, intervalCount=len(intervals),
                polls=sum(x['polls'] for x in intervals), publications=sum(x['published'] for x in intervals),
                aggregatePollingHz=sum(x['polls'] for x in intervals) * 1000 / elapsed if elapsed else None,
                aggregatePublishedHz=sum(x['published'] for x in intervals) * 1000 / elapsed if elapsed else None,
                intervalPollingHz=distribution([x['pollingHz'] for x in intervals]),
                snapshotGapP95Ms=distribution([x['gap'].get('p95Ms') for x in intervals]),
                snapshotGapP99Ms=distribution([x['gap'].get('p99Ms') for x in intervals]),
                maximumRecentGapMs=max((x['gap'].get('maxMs', 0) for x in intervals), default=0),
                intervals=intervals)


first_telemetry = telemetry[0]['payload']['sampledAtMs']
# 留出3秒让遮挡阶段的256项滚动间隔退出统计窗口。
stable_sampling = sampling_interval_report(first_telemetry + 3000, stats[-1]['payload']['sampledAtMs'])
all_sampling = sampling_interval_report(stats[0]['payload']['sampledAtMs'], stats[-1]['payload']['sampledAtMs'])
sampling_peaks = []
previous_peak = 0
for row in sampling:
    peak = row['payload']['gap'].get('maxObservedMs', 0)
    if peak > previous_peak:
        sampling_peaks.append(dict(seq=row['seq'], sampledAtMs=row['payload']['sampledAtMs'], peakMs=peak))
        previous_peak = peak

input_report = dict(samples=len(delivery), gamepadFramesSent=events['gamepadFrameSent'],
                    latest=delivery[-1]['payload'],
                    currentValueDistributions={field: distribution([row['payload'].get(field) for row in delivery])
                                               for field in ['bufferedBytes', 'oldestBufferedAgeMs', 'pendingInputs',
                                                             'ledgerEntries', 'softLimitBytes']},
                    maximumObservedLatencyMs={field: max(row['payload'][field].get('maxObservedMs', 0) for row in delivery)
                                              for field in ['queueWait', 'observationToSend', 'physicalSampleAge', 'drainAge']},
                    percentileSemantics='latest latency values describe the retained 256 samples; maxima cover this InputService lifetime')

freeze_events = []
burst_events = []
for row in stats:
    p = row['payload']
    delta = p.get('delta', {})
    if delta.get('freezeCountDelta', 0) > 0 or delta.get('packetsLostDelta', 0) >= 100:
        event = dict(seq=row['seq'], tsMs=row['tsMs'], sampledAtMs=p['sampledAtMs'],
                     delta=select(delta, ['elapsedMs', 'framesReceivedDelta', 'framesDecodedDelta',
                                          'framesDroppedDelta', 'packetsReceivedDelta', 'packetsLostDelta',
                                          'freezeCountDelta', 'totalFreezesDurationDelta', 'keyFramesDecodedDelta',
                                          'pliCountDelta', 'nackCountDelta']),
                     jitterMs=p['inboundVideo']['jitter'] * 1000)
        if delta.get('freezeCountDelta', 0) > 0:
            freeze_events.append(event)
        if delta.get('packetsLostDelta', 0) >= 100:
            burst_events.append(event)

last_pixel = telemetry[-1]['payload']['mediaDiagnostics']['frameIdentity']['videoPixels']
closed = next(row for row in resources if row['payload']['reason'] == 'clientClosed')
zero_fields = ['currentRenderers', 'pendingRenderers', 'frameRequests', 'framePreparationRequests',
               'nativeFrameCallbacks', 'animationCallbacks', 'pixelProbeCanvases', 'pixelProbeGpuReads',
               'pixelProbeTimers', 'videoElements', 'audioContexts']
assert all(closed['payload']['resources'][field] == 0 for field in zero_fields)
consumers = collections.Counter(row['payload']['consumer'] for row in rows if row['event'] == 'browserObservationConsumed')
behavior_code, behavior_text = script_report(SKILL / 'trace_browser_webrtc_behavior_report.py')
behavior = json.loads(behavior_text)
strict_code, strict_text = script_report(SKILL / 'trace_browser_webrtc_behavior_report.py', '--require-stability')
strict_behavior = json.loads(strict_text)
receive_code, receive_text = script_report(SKILL / 'trace_receive_feedback_report.py')
receive = json.loads(receive_text)
midsegment_code, midsegment_text = script_report(SKILL / 'trace_midsegment_report.py')

header = rows[0]['payload']
result = dict(
    analysisConsistencyGate='PASS', trace=str(LOG), traceSha256=trace_hash, rows=len(rows),
    traceBounds=dict(firstTsMs=rows[0]['tsMs'], lastTsMs=rows[-1]['tsMs'],
                     durationSeconds=(rows[-1]['tsMs'] - rows[0]['tsMs']) / 1000),
    sourceIdentity=dict(build=build, currentSourceFingerprint=current_fingerprint,
                        sourceMatchesCurrent=current_fingerprint == build['sourceFingerprintSha256'],
                        fingerprintAlgorithm='src-tauri/build.rs source_fingerprint'),
    testContext=dict(scenes='用户补充：菜单和运动的游戏库场景',
                     earlyWindowOcclusion='用户确认曾切换或遮住窗口；早期约1Hz与秒级停顿单独归属该测试段'),
    schema=dict(version=3, profiles=dict(collections.Counter(row['traceProfile'] for row in rows)),
                importance=dict(collections.Counter(row['importance'] for row in rows)),
                dimensions=dict(collections.Counter(row['dimension'] for row in rows)),
                sequenceContiguous=True, writerBudget=header['budget'], writerThread=header['writerThread'],
                budgetNotices=events['traceBudgetNotice']),
    events=dict(events), network=base['network'], foregroundMedia=base['foregroundMedia'],
    freezeEvents=freeze_events, lossBurstEvents=burst_events,
    rendererPhases=phases, rendererDrawCosts=draw_costs,
    pixelProbe=select(last_pixel, pixel_fields + ['preparationState', 'disabledReason', 'readTiming',
                                                'preparationTiming', 'discarded', 'timing', 'supplyMode']),
    sampling=dict(allMediaInterval=all_sampling, stableLateInterval=stable_sampling, lifetimePeakAnchors=sampling_peaks),
    inputDelivery=input_report,
    frontendTraceQueue=dict(pendingMax=max(row['payload']['trace']['pending'] for row in telemetry),
                            inFlightMax=max(row['payload']['trace']['inFlight'] for row in telemetry),
                            droppedMax=max(row['payload']['trace']['dropped'] for row in telemetry),
                            essentialDroppedMax=max(row['payload']['trace']['droppedEssential'] for row in telemetry)),
    cleanup=dict(seq=closed['seq'], observedAtMs=closed['payload']['observedAtMs'],
                 zeroResources={field: closed['payload']['resources'][field] for field in zero_fields},
                 stages=[dict(seq=row['seq'], payload=row['payload']) for row in rows if row['event'] == 'cleanupFinished']),
    renderPolicy=[dict(seq=row['seq'], payload=row['payload']) for row in rows if row['event'] == 'renderPolicyApplied'],
    renderActions=base['renderActions'],
    observationConsumers=dict(consumers),
    gates=dict(browserBehavior=dict(exitCode=behavior_code, gate=behavior['browserWebRtcBehaviorGate']),
               browserRuntimeStability=select(strict_behavior['browserRuntimeStability'], ['gate', 'failures', 'missingEvidence', 'evidence']),
               strictBrowser=dict(exitCode=strict_code, failures=strict_behavior['failures']),
               nativeReceiveFeedback=dict(exitCode=receive_code, summary=receive['summary'],
                                          applicability='Browser trace has no native ReferenceChain/RecoveryLedger events; report is an empty-evidence gate'),
               nativeMidsegment=dict(exitCode=midsegment_code, text=midsegment_text,
                                     applicability='no statsSnapshot/host epoch; use rendererPhases complete Browser windows')),
    sourceFiles={str(path.relative_to(ROOT)): sha(path) for path in [
        Path(__file__), BASE, ROOT / 'dev-docs/validation/2026-10-30-display-deficit-fresh-trace-validation.py',
        ROOT / 'src-tauri/build.rs', ROOT / 'src/shared/gamepad/browser-backend.ts',
        ROOT / 'src/player/infra/render/video-frame-pixel-observation.ts',
        ROOT / 'src/player/infra/render/video-frame-source.ts', ROOT / 'src/player/infra/render/Renderers.ts',
        ROOT / 'src/player/infra/render/SuperResolutionWebGL2Renderer.ts',
        ROOT / 'src/player/app/input/InputService.ts']},
    limitations=[
        '帧率按同代际即时增量及完整3秒/15秒Browser窗口计算；短SR阶段的窗口数量单独保留。',
        'GPU timer扩展在本平台不可用；CPU调用耗时与物理GPU执行/submit-to-present分别取证。',
        '233组有效像素配对均未确认变化；菜单/运动混合场景尚无逐段场景标记，配对结论限定在实际采样窗口。',
        'getGamepads轮询频率表示浏览器API调用；硬件报告频率和输入到游戏画面响应由外部测量确认。',
        '输入latency快照保留最近256项；physicalSampleAge受稳定输入100ms心跳影响。',
        'sharedSamples缺证，严格Browser稳定性门禁保持MISSING；native恢复链和present门禁在此Browser日志中保持观测边界。',
        '本日志覆盖成功启动与会话关闭，setup失败退出码和Pointer多帧协议沿单测证据验收。',
    ])
assert sha(LOG) == trace_hash
OUTPUT.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
print(json.dumps(dict(output=str(OUTPUT), analysisConsistencyGate=result['analysisConsistencyGate'],
                      sourceMatchesCurrent=result['sourceIdentity']['sourceMatchesCurrent'],
                      rows=len(rows), phases=len(phases), freezes=len(freeze_events),
                      stablePollingHz=stable_sampling['aggregatePollingHz'],
                      strictBrowserRuntimeGate=result['gates']['browserRuntimeStability']['gate']),
                 ensure_ascii=False))
