"""沿已有统计口径复算，并补前后台、跨计数余额与同窗供给证据。"""
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
LOG = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793345174456-1.jsonl')
OUTPUT = Path(sys.argv[2]) if len(sys.argv) > 2 else Path(__file__).with_suffix('.metrics.json')
BASELINE = ROOT / 'dev-docs/validation/2026-10-30-display-deficit-fresh-trace-validation.metrics.json'
BASE_SCRIPT = ROOT / 'dev-docs/validation/2026-10-30-display-deficit-fresh-trace-validation.py'
REPLAY = ROOT / 'dev-docs/validation/2026-10-30-video-texture-import-fresh-trace-replay.cjs'
with tempfile.TemporaryDirectory(prefix='xbxrc-video-import-audit-') as folder:
    base_output = Path(folder) / 'base.json'
    subprocess.run([sys.executable, '-B', str(BASE_SCRIPT), str(LOG), str(base_output)], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
    data = json.loads(base_output.read_text())
    replay_output = Path(folder) / 'replay.json'
    subprocess.run(['node', str(REPLAY), str(replay_output)], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
    data['staleCounterCreditReplay'] = json.loads(replay_output.read_text())

rows = [json.loads(line) for line in LOG.read_text().splitlines() if line.strip()]
stats = [r for r in rows if r['event'] == 'browserWebRtcStatsObserved']
telemetry = [r for r in rows if r['event'] == 'renderTelemetryObserved']
consumed = [r for r in rows if r['event'] == 'browserObservationConsumed' and r['payload'].get('consumer') == 'supervision']
policies = [r for r in rows if r['event'] == 'renderPolicyApplied']

def summary(values):
    values = sorted(v for v in values if isinstance(v, (int, float)) and math.isfinite(v))
    def q(p):
        if not values:
            return None
        at = (len(values) - 1) * p
        a, b = math.floor(at), math.ceil(at)
        return values[a] + (values[b] - values[a]) * (at - a)
    return dict(n=len(values), p50=q(.5), p95=q(.95), p99=q(.99), min=min(values) if values else None, max=max(values) if values else None)

def interval_metrics(selected):
    duration = sum(r['payload']['delta']['elapsedMs'] for r in selected) / 1000
    fields = ['packetsReceivedDelta', 'packetsLostDelta', 'framesReceivedDelta', 'framesDecodedDelta', 'framesDroppedDelta', 'freezeCountDelta', 'totalFreezesDurationDelta', 'nackCountDelta', 'keyFramesDecodedDelta', 'bytesReceivedDelta']
    changes = {field: sum(r['payload']['delta'].get(field, 0) for r in selected) for field in fields}
    lost = sum(max(0, r['payload']['delta'].get('packetsLostDelta', 0)) for r in selected)
    return dict(intervalCount=len(selected), durationSeconds=duration, changes=changes,
        receivedFps=changes['framesReceivedDelta'] / duration if duration else None,
        decodedFps=changes['framesDecodedDelta'] / duration if duration else None,
        positiveLossRatio=lost / (changes['packetsReceivedDelta'] + lost) if changes['packetsReceivedDelta'] + lost else None,
        rttMs=summary([r['payload'].get('selectedCandidatePair', {}).get('currentRoundTripTime', float('nan')) * 1000 for r in selected]),
        jitterMs=summary([r['payload']['inboundVideo'].get('jitter', float('nan')) * 1000 for r in selected]))

periods = []
end_ms = stats[-1]['payload']['sampledAtMs']
for index, row in enumerate(policies):
    p = row['payload']
    start = p['observedAtMs']
    end = min(end_ms, policies[index + 1]['payload']['observedAtMs'] if index + 1 < len(policies) else end_ms)
    if end <= start:
        continue
    selected = [r for r in stats if r['payload'].get('delta') and start <= r['payload']['sampledAtMs'] - r['payload']['delta']['elapsedMs'] and r['payload']['sampledAtMs'] <= end]
    periods.append(dict(seq=row['seq'], startMs=start, endMs=end, reason=p['reason'], kind=p['pipelineType'], targetFps=p['targetFps'], network=interval_metrics(selected)))
data['policyPeriods'] = periods
for name, active in [('foreground', True), ('background', False)]:
    selected = [r for r in stats if r['payload'].get('delta') and any((p['targetFps'] > 0) == active and p['startMs'] <= r['payload']['sampledAtMs'] - r['payload']['delta']['elapsedMs'] and r['payload']['sampledAtMs'] <= p['endMs'] for p in periods)]
    data[name + 'Media'] = interval_metrics(selected)
data['wallClock'] = [datetime.fromtimestamp(r['tsMs'] / 1000, ZoneInfo('Asia/Shanghai')).isoformat() for r in [rows[0], rows[-1]]]

identity = []
for generation in sorted({r['payload']['rendererGeneration'] for r in telemetry if r['payload'].get('rendererGeneration') is not None}):
    items = [r for r in telemetry if r['payload'].get('rendererGeneration') == generation]
    a, b = [r['payload']['mediaDiagnostics'] for r in [items[0], items[-1]]]
    ia, ib = a['frameIdentity'], b['frameIdentity']
    committed_at = next(r['observedAtMs'] for r in data['rendererLifecycle'] if r['reason'] == 'rendererCommitted' and r['current'] == generation)
    # 帧身份环属于共享 video；按 renderer 提交边界过滤环内前代际证据。
    evidence = {(e['atMs'], e['trackingSource']): e for r in items for e in r['payload']['mediaDiagnostics']['frameIdentity']['evidence'] if committed_at <= e['atMs'] <= b['observedAtMs']}
    native = [e for e in evidence.values() if e['trackingSource'] == 'videoFrameCallback']
    poll = [e for e in evidence.values() if e['trackingSource'] == 'animationFrame']
    native_present_delta = native[-1]['nativePresentedFrames'] - native[0]['nativePresentedFrames']
    counter_delta = native[-1]['counterFrames'] - native[0]['counterFrames']
    identity.append(dict(rendererGeneration=generation, seq=[items[0]['seq'], items[-1]['seq']],
        diagnosticAtMs=[a['observedAtMs'], b['observedAtMs']], evidenceLowerBoundMs=committed_at,
        acceptedDelta={k: ib['accepted'][k] - ia['accepted'][k] for k in ib['accepted']},
        observedDelta={k: ib['observed'][k] - ia['observed'][k] for k in ib['observed']},
        nativeEvidenceCount=len(native), pollEvidenceCount=len(poll),
        nativeMinusPlaybackCounter=summary([e['nativePresentedFrames'] - e['counterFrames'] for e in native]),
        nativeCreditBalance=summary([e['uncountedNativeAfter'] for e in evidence.values()]),
        expectedDisplayLeadMs=summary([e['expectedDisplayTime'] - e['atMs'] for e in native if isinstance(e.get('expectedDisplayTime'), (int, float))]),
        nativeEvidenceSpan=dict(atMs=[native[0]['atMs'], native[-1]['atMs']], nativePresentedDelta=native_present_delta, playbackCounterDelta=counter_delta),
        pollReasonCounts=dict(collections.Counter(e['reason'] for e in poll)),
        creditAnchors=[dict(telemetrySeq=r['seq'], evidence={k: e.get(k) for k in ['atMs','trackingSource','counterFrames','nativePresentedFrames','counterDelta','creditedNativeFrames','counterAdvance','uncountedNativeBefore','uncountedNativeAfter','reason']}) for r in items for e in r['payload']['mediaDiagnostics']['frameIdentity']['evidence'] if e.get('counterDelta', 0) and e['counterDelta'] > 0][:3]))
data['counterCreditEvidence'] = identity
data['latestFrameIdentityCounts'] = {k: data['frameIdentity']['latest'][k] for k in ['observed', 'accepted', 'rejected', 'reasons']}

freeze_rows = [r for r in stats if r['payload'].get('delta', {}).get('freezeCountDelta', 0) > 0]
data['freezeTimeline'] = []
for anchor in freeze_rows:
    at = anchor['payload']['sampledAtMs']
    selected = [r for r in stats if at - 2000 <= r['payload']['sampledAtMs'] <= at + 1000]
    data['freezeTimeline'].append(dict(seq=anchor['seq'], rows=[dict(seq=r['seq'], sampledAtMs=r['payload']['sampledAtMs'],
        delta={k: r['payload'].get('delta', {}).get(k) for k in ['elapsedMs','bytesReceivedDelta','packetsReceivedDelta','packetsLostDelta','framesReceivedDelta','framesDecodedDelta','framesDroppedDelta','freezeCountDelta','totalFreezesDurationDelta','nackCountDelta','keyFramesDecodedDelta']},
        selectedPairObserved=bool(r['payload'].get('selectedCandidatePair')), connectionState=r['payload'].get('connectionState')) for r in selected]))

# 末次 freeze 后重新建立完整15秒供给窗口，避免把停顿混入稳态。
recovered = next(r['payload']['sampledAtMs'] for r in stats if r['payload']['sampledAtMs'] > freeze_rows[-1]['payload']['sampledAtMs'] and r['payload'].get('delta', {}).get('framesDecodedDelta', 0) > 0)
steady = [r for r in consumed if r['payload'].get('rendererGeneration') == 4 and r['payload']['sampledAtMs'] >= recovered + 15000 and all(r['payload']['facts']['windows']['long'].get(k, 0) >= 14999 for k in ['receivedCoverageMs','decodeCoverageMs','callbackCoverageMs','rendererCoverageMs','videoCoverageMs'])]
data['finalRecoveredSteady'] = dict(recoveredAtMs=recovered, windowMs=15000, n=len(steady), seq=[steady[0]['seq'], steady[-1]['seq']],
    fields={f: summary([r['payload']['facts']['windows']['long'][f] for r in steady]) for f in ['inboundFpsAvg','decodeFpsAvg','videoPresentFpsAvg','callbackFpsAvg','rendererSubmitFpsAvg']})

baseline = json.loads(BASELINE.read_text())
before = baseline['combinedDrawCosts']['avgMs']
after = data['combinedDrawCosts']['avgMs']
data['baselineComparison'] = dict(trace=baseline['trace'], traceSha256=baseline['traceSha256'],
    sameDimensions=True, sameBudgetFps=60, sameMode='passthrough-RCAS',
    limitation='Separate sessions; bitrate, foreground history and scene differ. CPU call costs compare per-frame means; rates require same-window evidence.',
    countedDraws=[baseline['combinedDrawCosts']['count'], data['combinedDrawCosts']['count']],
    costMs={k: dict(before=before[k], after=after[k], reductionRatio=1 - after[k] / before[k]) for k in before})
data['coldFirstDraws'] = [dict(seq=r['seq'], generation=r['payload']['resources']['rendererObservation']['generation'],
    uploadMs=r['payload']['resources']['rendererObservation']['resources']['uploadDurationMaxMs'],
    totalMs=r['payload']['resources']['rendererObservation']['resources']['drawDurationMaxMs']) for r in rows if r['event'] == 'browserMediaResourcesObserved' and r['payload']['reason'] == 'rendererCommitted' and r['payload']['resources']['rendererObservation']['kind'] == 'webgl2_sr']
data['nativeGateApplicability'] = dict(nativeStatsSnapshotCount=data['events']['statsSnapshot'], browserMediaAuthority=True, nativeMidsegmentGate='inapplicable: browser-direct media; dormant native singleton observations')
data['freshAnalysisScriptSha256'] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
OUTPUT.write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n')
print(json.dumps(dict(output=str(OUTPUT), sourceMatchesCurrent=data['sourceMatchesCurrent'], uploadComparison=data['baselineComparison']['costMs']['uploadDurationTotalMs'], foreground=data['foregroundMedia'], steady=data['finalRecoveredSteady']), ensure_ascii=False, indent=2))
