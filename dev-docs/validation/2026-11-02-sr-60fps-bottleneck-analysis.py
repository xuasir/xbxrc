"""按同代际实时诊断复算 SR 供给边界，并回放现有生产模块。"""

import hashlib
import json
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LOG = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(
    '/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793353382323-1.jsonl')
OUTPUT = Path(sys.argv[2]) if len(sys.argv) > 2 else Path(__file__).with_suffix('.metrics.json')
BASE = ROOT / 'dev-docs/validation/2026-11-02-network-render-fresh-trace-analysis.py'
REPLAY = ROOT / 'dev-docs/validation/2026-10-30-browser-frame-counter-pairing-audit.cjs'
SUMMARY = ROOT / '.agents/skills/analyze-runtime-logs/scripts/summarize_runtime_trace.py'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(command):
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(result.stderr or result.stdout)
    return result.stdout


# 同一次复算先审计日志，再核对完整窗口，最后运行既有生产模块回放。
summary = json.loads(run([sys.executable, '-B', str(SUMMARY), str(LOG), '--json']))
with tempfile.TemporaryDirectory(prefix='xbxrc-sr-60fps-') as folder:
    base_path, replay_path = [Path(folder) / name for name in ['base.json', 'replay.json']]
    run([sys.executable, '-B', str(BASE), str(LOG), str(base_path)])
    run(['node', str(REPLAY), str(replay_path)])
    base, replay = [json.loads(path.read_text()) for path in [base_path, replay_path]]

rows = [json.loads(line) for line in LOG.read_text().splitlines() if line.strip()]
by_seq = {row['seq']: row for row in rows}
assert len(by_seq) == len(rows) == summary['rows']
assert base['traceSha256'] == sha(LOG)
assert base['verification']['status'] == 'PASS'

phases = []
for phase in base['phases']:
    live = phase['liveDelta']
    if not phase['visible'] or live is None:
        continue
    first, last = [by_seq[seq] for seq in live['seq']]
    a, b = [row['payload']['mediaDiagnostics'] for row in [first, last]]
    assert a['rendererGeneration'] == b['rendererGeneration'] == phase['rendererGeneration']
    assert b['frameIdentity']['localSequence'] - a['frameIdentity']['localSequence'] == live['candidate']
    clocks = {}
    for source in ['animationFrame', 'videoFrameCallback']:
        ta, tb = [d['frameIdentity']['callbackTiming'][source] for d in [a, b]]
        delta = {key: tb[key] - ta[key] for key in [
            'count', 'intervalCount', 'intervalTotalMs', 'clockLagCount', 'clockLagTotalMs']}
        assert delta['count'] == live['observed'][source]
        delta.update(
            observedFps=delta['count'] / live['durationSeconds'],
            accepted=live['accepted'][source],
            meanIntervalMs=delta['intervalTotalMs'] / delta['intervalCount'] if delta['intervalCount'] else None,
            meanEntryClockLagMs=delta['clockLagTotalMs'] / delta['clockLagCount'] if delta['clockLagCount'] else None)
        clocks[source] = delta
    item = {key: phase[key] for key in ['rendererGeneration', 'kind', 'startMs', 'endMs']}
    item.update(visible=True, liveDelta=live, callbackClocks=clocks,
                counterCadenceDelta=phase.get('counterCadenceDelta'), windows=phase['windows'])
    if live['submitted'] is not None:
        assert b['submittedFramesTotal'] - a['submittedFramesTotal'] == live['submitted']
        item.update(candidateFps=live['candidate'] / live['durationSeconds'],
                    submittedFps=live['submitted'] / live['durationSeconds'],
                    candidateDeliveryRatio=live['submitted'] / live['candidate'])
    phases.append(item)

sr = [phase for phase in phases if phase['kind'] == 'webgl2_sr']
replay_results = replay['results']
assert len(replay_results) == 36
for result in replay_results:
    assert result['uniqueImportedImages'] == result['submitted']
    assert result['duplicateDrawCount'] == 0
    if result['diagnostics']:
        off = next(r for r in replay_results if r['renderer'] == result['renderer']
                   and r['scenario'] == result['scenario'] and not r['diagnostics'])
        for field in ['acceptedCandidates', 'acceptedPoll', 'submitted', 'uniqueImportedImages']:
            assert result[field] == off[field]

collection_seq = base['collectionBoundary']['seq'][1]
collection_row = by_seq[collection_seq]
assert collection_row['payload']['mediaDiagnostics']['frameIdentity']['videoPixels']['readCount'] == 0
first_pixel = next(row for row in rows if row['event'] == 'renderTelemetryObserved'
                   and row['payload']['mediaDiagnostics']['frameIdentity']['videoPixels']['readCount'] > 0)
refresh_rows = [dict(seq=row['seq'], tsMs=row['tsMs'], payload=row['payload']) for row in rows
                if row['event'] == 'renderPolicyApplied'
                and row['payload'].get('displayGeometryTrigger') == 'refreshRateEstimateChanged']

report = {
    'trace': str(LOG), 'traceSha256': sha(LOG), 'rows': len(rows),
    'analysisDate': '2026-11-02', 'captureBounds': base['traceBounds'],
    'build': base['build'], 'sourceMatchesCurrent': base['sourceMatchesCurrent'],
    'sourceInputCount': base['sourceInputCount'],
    'network': base['network'], 'foregroundMedia': base['foregroundMedia'],
    'foregroundPhases': phases, 'srLivePhases': sr,
    'collectionBoundary': base['collectionBoundary'],
    'firstPixelReadObserved': {'seq': first_pixel['seq'], 'tsMs': first_pixel['tsMs'],
                             'pixels': first_pixel['payload']['mediaDiagnostics']['frameIdentity']['videoPixels']},
    'refreshPolicyRows': refresh_rows,
    'combinedSteadyDrawCosts': base['combinedSteadyDrawCosts'],
    'srSubmitAgeMs': base['srSubmitAgeMs'], 'renderTelemetrySegments': base['renderTelemetrySegments'],
    'recovery': base['recovery'], 'gates': base['gates'],
    'traceQueue': base['traceQueue'], 'latestFrameIdentityCounts': base['latestFrameIdentityCounts'],
    'productionReplay': replay,
    'verification': {'status': 'PASS', 'fullyCoveredWindowSamples': base['verification']['fullyCoveredWindowSamples'],
                     'sameGenerationCallbackDeltas': 'PASS', 'sourceImageUniqueness': 'PASS',
                     'diagnosticsOnOffDeliveryEqual': 'PASS', 'productionReplayCases': len(replay_results)},
    'scriptsSha256': {str(path.relative_to(ROOT)): sha(path) for path in [Path(__file__), BASE, REPLAY, SUMMARY]},
    'limitations': [
        '完整窗口允许重叠；即时诊断 delta 与缓存采样分别使用各自同窗计数。',
        'RVFC 计数和 video 推进属于浏览器接口事实；每帧可绘制像素与物理屏幕呈现继续待验。',
        'clockLag 衡量浏览器回调时间戳到 JS 入口的差；实际源帧等待与合成排队继续待验。',
        '诊断启停的设备因果关系继续由同条件对照确认；受控回放只验证相同输入下的代码行为。',
        '本次 SR 为 1440p 等尺寸 passthrough；实际放大效果路径继续需要独立样本。',
        'GPU 执行、submit-to-present、ReferenceChain 与逐 NACK 效果继续待验。']}
OUTPUT.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
print(json.dumps({'output': str(OUTPUT), 'verification': report['verification'],
                  'srClocks': [{'generation': p['rendererGeneration'],
                                'rafFps': p['callbackClocks']['animationFrame']['observedFps'],
                                'nativeFps': p['callbackClocks']['videoFrameCallback']['observedFps'],
                                'submittedFps': p['submittedFps']} for p in sr]}, ensure_ascii=False, indent=2))
