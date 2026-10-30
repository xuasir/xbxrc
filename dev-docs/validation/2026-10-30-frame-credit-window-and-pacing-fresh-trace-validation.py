"""复算抵扣窗口修正后的日志；按 renderer、可见区间及冻结恢复分开取证。"""
import collections
import hashlib
import json
import math
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LOG = Path(sys.argv[1])
OUTPUT = Path(sys.argv[2]) if len(sys.argv) > 2 else Path(__file__).with_suffix('.metrics.json')
BASE = ROOT / 'dev-docs/validation/2026-10-30-display-deficit-fresh-trace-validation.py'
if len(sys.argv) > 3:
    data = json.loads(Path(sys.argv[3]).read_text())
    assert data['traceSha256'] == hashlib.sha256(LOG.read_bytes()).hexdigest()
    assert data['analysisScriptSha256'] == hashlib.sha256(BASE.read_bytes()).hexdigest()
else:
    with tempfile.TemporaryDirectory(prefix='xbxrc-credit-fresh-') as folder:
        base_output = Path(folder) / 'base.json'
        subprocess.run([sys.executable, '-B', str(BASE), str(LOG), str(base_output)],
                       cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
        data = json.loads(base_output.read_text())

rows = [json.loads(line) for line in LOG.read_text().splitlines() if line.strip()]
stats = [r for r in rows if r['event'] == 'browserWebRtcStatsObserved']
telemetry = [r for r in rows if r['event'] == 'renderTelemetryObserved']
consumed = [r for r in rows if r['event'] == 'browserObservationConsumed'
            and r['payload'].get('consumer') == 'supervision']
resources = [r for r in rows if r['event'] in ['browserMediaResourcesObserved', 'browserMediaResourcesSnapshot']]
policies = [r for r in rows if r['event'] == 'renderPolicyApplied']
closed_at = next(r['payload']['observedAtMs'] for r in resources if r['payload']['reason'] == 'clientClosed')
commits = [r for r in resources if r['payload']['reason'] == 'rendererCommitted']


def summary(values):
    values = sorted(v for v in values if isinstance(v, (int, float)) and math.isfinite(v))
    def q(p):
        if not values:
            return None
        at = (len(values) - 1) * p
        a, b = math.floor(at), math.ceil(at)
        return values[a] + (values[b] - values[a]) * (at - a)
    return dict(n=len(values), p50=q(.5), p95=q(.95), p99=q(.99),
                min=min(values) if values else None, max=max(values) if values else None)


def interval_metrics(selected):
    seconds = sum(r['payload']['delta']['elapsedMs'] for r in selected) / 1000
    fields = ['packetsReceivedDelta', 'packetsLostDelta', 'bytesReceivedDelta', 'framesReceivedDelta',
              'framesDecodedDelta', 'framesDroppedDelta', 'freezeCountDelta', 'totalFreezesDurationDelta',
              'pliCountDelta', 'firCountDelta', 'nackCountDelta', 'keyFramesDecodedDelta']
    changes = {f: sum(r['payload']['delta'].get(f, 0) for r in selected) for f in fields}
    lost = sum(max(0, r['payload']['delta'].get('packetsLostDelta', 0)) for r in selected)
    return dict(intervalCount=len(selected), durationSeconds=seconds, changes=changes,
                receivedFps=changes['framesReceivedDelta']/seconds if seconds else None,
                decodedFps=changes['framesDecodedDelta']/seconds if seconds else None,
                positiveLossRatio=lost/(changes['packetsReceivedDelta']+lost) if changes['packetsReceivedDelta']+lost else None,
                rttMs=summary([r['payload'].get('selectedCandidatePair', {}).get('currentRoundTripTime', float('nan'))*1000 for r in selected]),
                jitterMs=summary([r['payload']['inboundVideo'].get('jitter', float('nan'))*1000 for r in selected]))


def intervals(start, end):
    return [r for r in stats if r['payload'].get('delta')
            and start <= r['payload']['sampledAtMs']-r['payload']['delta']['elapsedMs']
            and r['payload']['sampledAtMs'] <= end]


periods = []
for i, row in enumerate(policies):
    p = row['payload']; start = p['observedAtMs']
    end = min(closed_at, policies[i+1]['payload']['observedAtMs'] if i+1 < len(policies) else closed_at)
    if end > start:
        periods.append(dict(seq=row['seq'], startMs=start, endMs=end, reason=p['reason'],
                            kind=p['pipelineType'], targetFps=p['targetFps']))
data['policyPeriods'] = periods
for name, visible in [('foreground', True), ('background', False)]:
    selected = [r for r in stats if r['payload'].get('delta') and any(
        (p['targetFps'] > 0) == visible and p['startMs'] <= r['payload']['sampledAtMs']-r['payload']['delta']['elapsedMs']
        and r['payload']['sampledAtMs'] <= p['endMs'] for p in periods)]
    data[name+'Media'] = interval_metrics(selected)

coverage = ['receivedCoverageMs', 'decodeCoverageMs', 'callbackCoverageMs', 'rendererCoverageMs', 'videoCoverageMs']
fps_fields = ['inboundFpsAvg', 'decodeFpsAvg', 'videoPresentFpsAvg', 'callbackFpsAvg', 'rendererSubmitFpsAvg']


def windows(generation, kind, start, end, length):
    label = 'short' if length == 3000 else 'long'
    selected = [r for r in consumed if r['payload'].get('rendererGeneration') == generation
                and start+length <= r['payload']['sampledAtMs'] <= end
                and all(r['payload']['facts']['windows'][label].get(k, 0) >= length-1 for k in coverage)]
    values = [r['payload']['facts']['windows'][label] for r in selected]
    output_field = 'videoPresentFpsAvg' if kind == 'video' else 'rendererSubmitFpsAvg'
    return dict(windowMs=length, n=len(selected), seq=[selected[0]['seq'], selected[-1]['seq']] if selected else None,
                outputDefinition=output_field, fields={f: summary([v.get(f) for v in values]) for f in fps_fields},
                supplyDeltaFps=summary([v['decodeFpsAvg']-v[output_field] for v in values]),
                candidateToSubmitDeltaFps=summary([v['callbackFpsAvg']-v['rendererSubmitFpsAvg'] for v in values]) if kind != 'video' else None,
                rows=[dict(seq=r['seq'], sampledAtMs=r['payload']['sampledAtMs'],
                           values={f: r['payload']['facts']['windows'][label].get(f) for f in fps_fields}) for r in selected])


phase_metrics = []
for i, commit in enumerate(commits):
    p = commit['payload']; observation = p['resources']['rendererObservation']
    generation, kind = observation['generation'], observation['kind']
    begin = p['observedAtMs']; finish = commits[i+1]['payload']['observedAtMs'] if i+1 < len(commits) else closed_at
    # 仅可见性变更切分 renderer，几何/锐化等政策变化继续沿同一媒体阶段。
    breaks = [begin]+sorted({x for period in periods if period['reason'] in ['documentHidden', 'documentVisibleResume']
                            for x in [period['startMs']] if begin < x < finish})+[finish]
    for start, end in zip(breaks, breaks[1:]):
        policy = max((q for q in periods if q['startMs'] <= start), key=lambda q: q['startMs'])
        live = [r for r in telemetry if (d := r['payload'].get('mediaDiagnostics', {})).get('rendererGeneration') == generation
                and d.get('rendererKind') == kind and start <= d['observedAtMs'] <= end]
        delta = None
        if len(live) >= 2:
            a, b = [r['payload']['mediaDiagnostics'] for r in [live[0], live[-1]]]
            seconds = (b['observedAtMs']-a['observedAtMs'])/1000
            ia, ib = a['frameIdentity'], b['frameIdentity']
            submitted = b['submittedFramesTotal']-a['submittedFramesTotal'] if 'submittedFramesTotal' in a and 'submittedFramesTotal' in b else None
            delta = dict(seq=[live[0]['seq'], live[-1]['seq']], durationSeconds=seconds,
                         candidate=ib['localSequence']-ia['localSequence'], submitted=submitted,
                         accepted={k: ib['accepted'][k]-ia['accepted'][k] for k in ib['accepted']},
                         observed={k: ib['observed'][k]-ia['observed'][k] for k in ib['observed']})
        # 共享 video 身份环按实际证据时钟过滤，避免跨 renderer/前后台残留。
        evidence = {}
        for r in live:
            for e in r['payload']['mediaDiagnostics']['frameIdentity']['evidence']:
                if start <= e['atMs'] <= end:
                    evidence.setdefault((e['atMs'], e['trackingSource']), dict(telemetrySeq=r['seq'], **e))
        native = sorted((e for e in evidence.values() if e['trackingSource'] == 'videoFrameCallback'), key=lambda e: e['atMs'])
        poll = sorted((e for e in evidence.values() if e['trackingSource'] == 'animationFrame'), key=lambda e: e['atMs'])
        batch = [e for e in evidence.values() if (e.get('counterDelta') or 0) > 0]
        leads = [e['expectedDisplayTime']-e['atMs'] for e in native if isinstance(e.get('expectedDisplayTime'), (int, float))]
        cached = [r for previous, r in zip(telemetry, telemetry[1:])
                  if previous['payload'].get('rendererGeneration') == generation
                  and r['payload'].get('rendererGeneration') == generation
                  and start <= previous['payload']['sampledAtMs'] <= r['payload']['sampledAtMs'] <= end]
        selected_resources = [dict(seq=r['seq'], atMs=r['payload']['observedAtMs'],
                                  **{k: r['payload']['resources'].get(k) for k in ['uncountedNativeFrames', 'expiredNativeFramesTotal', 'nativeCreditWindowExpirationCount', 'nativeCallbackCount', 'polledFrameCount']})
                              for r in resources if start <= r['payload']['observedAtMs'] < end
                              and r['payload'].get('resources', {}).get('rendererObservation', {}).get('generation') == generation]
        phase_metrics.append(dict(rendererGeneration=generation, kind=kind, startMs=start, endMs=end,
            visible=policy['targetFps'] > 0, network=interval_metrics(intervals(start, end)), liveDelta=delta,
            windows={str(length): windows(generation, kind, start, end, length) for length in [3000, 15000]},
            resources=selected_resources,
            identity=dict(nativeSamples=len(native), pollSamples=len(poll), pollCounterUnchanged=sum(e.get('counterDelta') == 0 for e in poll),
                          pollReasons=dict(collections.Counter(e['reason'] for e in poll)),
                          positiveCounterDelta=summary([e['counterDelta'] for e in batch]),
                          positiveCounterDeltaBySource=dict(collections.Counter(e['trackingSource'] for e in batch)),
                          batchProgressAgeMs=summary([e.get('counterProgressAgeMs') for e in batch]),
                          expectedDisplayLeadMs=summary(leads), expectedDisplayLatenessMs=summary([-v for v in leads]),
                          lateExpectedDisplayCount=sum(v < 0 for v in leads),
                          rawNativeDelta=summary([e.get('rawNativeDelta') for e in native]),
                          mediaTimeStates=dict(collections.Counter(e['mediaTimeState'] for e in native)),
                          evidence=list(sorted(evidence.values(), key=lambda e: (e['atMs'], e['trackingSource'])))),
            callbackSampleTotals={f: sum(r['payload'].get(f, 0) for r in cached) for f in ['callbackCountSinceLastSample', 'callbackGapCountSinceLastSample', 'presentedFramesAdvancedSinceLastSample', 'presentedFramesJumpCountSinceLastSample', 'droppedFramesSinceLastSample']},
            maxCallbackIntervalMs=summary([r['payload'].get('maxCallbackIntervalMsSinceLastSample') for r in cached])))
data['visibilityRendererPhases'] = phase_metrics

data['freezeTimeline'] = []
for anchor in stats:
    if anchor['payload'].get('delta', {}).get('freezeCountDelta', 0) <= 0:
        continue
    at = anchor['payload']['sampledAtMs']
    selected = [r for r in stats if at-2000 <= r['payload']['sampledAtMs'] <= at+1000]
    data['freezeTimeline'].append(dict(seq=anchor['seq'], rows=[dict(seq=r['seq'], sampledAtMs=r['payload']['sampledAtMs'],
        delta={k: r['payload'].get('delta', {}).get(k) for k in ['elapsedMs', 'bytesReceivedDelta', 'packetsReceivedDelta', 'packetsLostDelta', 'framesReceivedDelta', 'framesDecodedDelta', 'framesDroppedDelta', 'freezeCountDelta', 'totalFreezesDurationDelta', 'pliCountDelta', 'nackCountDelta', 'keyFramesDecodedDelta']},
        jitterMs=r['payload']['inboundVideo'].get('jitter', 0)*1000, connectionState=r['payload'].get('connectionState')) for r in selected]))

data['steadyForegroundWindows'] = []
for phase in phase_metrics:
    if phase['kind'] != 'video' or not phase['visible']:
        continue
    recovered = phase['startMs']
    for f in data['freezeTimeline']:
        at = next(r['payload']['sampledAtMs'] for r in stats if r['seq'] == f['seq'])
        if phase['startMs'] < at < phase['endMs']:
            recovered = max(recovered, next(r['payload']['sampledAtMs'] for r in stats if r['payload']['sampledAtMs'] > at and r['payload'].get('delta', {}).get('framesDecodedDelta', 0) > 0))
    data['steadyForegroundWindows'].append(dict(rendererGeneration=phase['rendererGeneration'], startMs=recovered, endMs=phase['endMs'],
                                               windows=windows(phase['rendererGeneration'], 'video', recovered, phase['endMs'], 15000)))
data['latestFrameIdentityCounts'] = {k: data['frameIdentity']['latest'][k] for k in ['observed', 'accepted', 'rejected', 'reasons']}
data['nativeGateApplicability'] = dict(nativeStatsSnapshotCount=data['events'].get('statsSnapshot', 0),
                                      browserMediaAuthority=True, nativeMidsegmentGate='inapplicable: dormant native singleton')
data['analysisInterpretation'] = dict(identityEvidence='bounded sample ring; counts/fractions describe sampled evidence only',
    windowSamples='overlapping fully covered policy windows', videoOutput='browser video counter advancement',
    srOutput='successful local renderer processing', presentedFramesDelta='max(counterAdvance, nativeAdvance): inferred union; rawNativeDelta stored separately',
    submitAge='age of latest local successful submission; submit-to-present unavailable')
data['freshAnalysisScriptSha256'] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
OUTPUT.write_text(json.dumps(data, ensure_ascii=False, indent=2)+'\n')
print(json.dumps(dict(output=str(OUTPUT), sourceMatchesCurrent=data['sourceMatchesCurrent'],
    phases=[dict(generation=p['rendererGeneration'], kind=p['kind'], visible=p['visible'], start=p['startMs'], end=p['endMs'], live=p['liveDelta'],
                 windows={k: dict(n=v['n'], seq=v['seq'], fps={f: s['p50'] for f, s in v['fields'].items()}) for k,v in p['windows'].items()},
                 identity={k:v for k,v in p['identity'].items() if k != 'evidence'}) for p in phase_metrics],
    steady=[dict(generation=p['rendererGeneration'], start=p['startMs'], n=p['windows']['n'], seq=p['windows']['seq'], fields=p['windows']['fields']) for p in data['steadyForegroundWindows']]),
    ensure_ascii=False, indent=2))
