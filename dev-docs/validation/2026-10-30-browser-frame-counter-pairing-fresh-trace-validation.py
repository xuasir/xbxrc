"""沿 renderer/可见区间复算，并核验配对计数与回调累计时序。"""
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
BASE = ROOT/'dev-docs/validation/2026-10-30-frame-credit-window-and-pacing-fresh-trace-validation.py'
if len(sys.argv) > 3:
    data = json.loads(Path(sys.argv[3]).read_text())
    assert data['traceSha256'] == hashlib.sha256(LOG.read_bytes()).hexdigest()
    assert data['freshAnalysisScriptSha256'] == hashlib.sha256(BASE.read_bytes()).hexdigest()
else:
    with tempfile.TemporaryDirectory(prefix='xbxrc-counter-pair-fresh-') as folder:
        output = Path(folder)/'base.json'
        subprocess.run([sys.executable, '-B', str(BASE), str(LOG), str(output)], cwd=ROOT,
                       check=True, stdout=subprocess.DEVNULL)
        data = json.loads(output.read_text())

rows = [json.loads(line) for line in LOG.read_text().splitlines() if line.strip()]
telemetry = [r for r in rows if r['event'] == 'renderTelemetryObserved']
stats = [r for r in rows if r['event'] == 'browserWebRtcStatsObserved']
commits = [r for r in rows if r['event'] == 'browserMediaResourcesObserved' and r['payload']['reason'] == 'rendererCommitted']


def summary(values):
    values = sorted(v for v in values if isinstance(v, (int, float)) and math.isfinite(v))
    def q(p):
        if not values:
            return None
        at = (len(values)-1)*p
        lo, hi = math.floor(at), math.ceil(at)
        return values[lo]+(values[hi]-values[lo])*(at-lo)
    return dict(n=len(values), p50=q(.5), p95=q(.95), p99=q(.99),
                min=min(values) if values else None, max=max(values) if values else None)


def cumulative_delta(first, last):
    timing = {}
    for source, b in last.get('callbackTiming', {}).items():
        a = first.get('callbackTiming', {}).get(source, {})
        changes = {k: b[k]-a[k] for k in ['count', 'intervalCount', 'intervalTotalMs', 'clockLagCount', 'clockLagTotalMs']}
        timing[source] = dict(changes, intervalMsAvg=changes['intervalTotalMs']/changes['intervalCount'] if changes['intervalCount'] else None,
                              callbackClockLagMsAvg=changes['clockLagTotalMs']/changes['clockLagCount'] if changes['clockLagCount'] else None)
    a, b = first.get('counterCadence', {}), last.get('counterCadence', {})
    cadence = {k: b[k]-a[k] for k in ['updateCount', 'advanceTotal', 'intervalCount', 'intervalTotalMs', 'resetCount']}
    cadence['updatesBySource'] = {k: b['updatesBySource'][k]-a['updatesBySource'][k] for k in b.get('updatesBySource', {})}
    cadence['batchSizeAvg'] = cadence['advanceTotal']/cadence['updateCount'] if cadence['updateCount'] else None
    cadence['updateIntervalMsAvg'] = cadence['intervalTotalMs']/cadence['intervalCount'] if cadence['intervalCount'] else None
    return dict(callbackTiming=timing, counterCadence=cadence)


evidence = {}
for r in telemetry:
    for e in r['payload'].get('mediaDiagnostics', {}).get('frameIdentity', {}).get('evidence', []):
        evidence.setdefault((e['atMs'], e['trackingSource']), dict(telemetrySeq=r['seq'], **e))
paired = [e for e in evidence.values() if e.get('pairedCounters')]
latest = telemetry[-1]['payload']['mediaDiagnostics']['frameIdentity']
data['pairedCounterEvidence'] = dict(uniqueSamples=len(paired),
    playbackQualityValidSamples=sum(e['pairedCounters'].get('playbackQualityFrames') is not None for e in paired),
    webkitValidSamples=sum(e['pairedCounters'].get('webkitFrames') is not None for e in paired),
    latestTiming=latest.get('callbackTiming'), latestCadence=latest.get('counterCadence'),
    sampleRateHz=latest.get('sampledCount', 0)/((telemetry[-1]['payload']['mediaDiagnostics']['observedAtMs']-latest['collectionStartedAtMs'])/1000),
    qualityDroppedValues=sorted({e['pairedCounters']['playbackQualityDroppedFrames'] for e in paired if e['pairedCounters']['playbackQualityDroppedFrames'] is not None}),
    sampleEvidence=list(sorted(paired, key=lambda e: (e['atMs'], e['trackingSource']))))

for phase in data['visibilityRendererPhases']:
    generation, start, end = phase['rendererGeneration'], phase['startMs'], phase['endMs']
    commit = next(r for r in commits if r['payload']['resources']['rendererObservation']['generation'] == generation)
    ob = commit['payload']['resources']['rendererObservation']
    phase['dimensions'] = dict(source=[ob['sourceWidth'], ob['sourceHeight']], output=[ob['resources']['outputWidth'], ob['resources']['outputHeight']])
    live = [r for r in telemetry if (d := r['payload'].get('mediaDiagnostics', {})).get('rendererGeneration') == generation
            and start <= d['observedAtMs'] <= end]
    if len(live) >= 2:
        phase['diagnosticCumulativeDelta'] = cumulative_delta(*[r['payload']['mediaDiagnostics']['frameIdentity'] for r in [live[0], live[-1]]])
    items = [e for e in phase['identity']['evidence'] if e.get('pairedCounters')]
    phase['pairedSamples'] = dict(n=len(items), webkitValid=sum(e['pairedCounters']['webkitFrames'] is not None for e in items),
        playbackQualityValid=sum(e['pairedCounters']['playbackQualityFrames'] is not None for e in items),
        callbackIntervalMsBySource={s: summary([e.get('callbackIntervalMs') for e in items if e['trackingSource'] == s]) for s in ['videoFrameCallback', 'animationFrame']},
        callbackClockLagMsBySource={s: summary([e.get('callbackClockLagMs') for e in items if e['trackingSource'] == s]) for s in ['videoFrameCallback', 'animationFrame']},
        expectedDisplayLeadAtEntryMs=summary([e['expectedDisplayTime']-e['callbackEnteredAtMs'] for e in items
                                            if isinstance(e.get('expectedDisplayTime'), (int, float)) and isinstance(e.get('callbackEnteredAtMs'), (int, float))]))

data['sourceDimensionTransitions'] = []
previous = None
for r in stats:
    v = r['payload']['inboundVideo']; size = (v.get('frameWidth'), v.get('frameHeight'))
    if size != previous:
        data['sourceDimensionTransitions'].append(dict(seq=r['seq'], sampledAtMs=r['payload']['sampledAtMs'], size=size))
        previous = size
data['inboundIdentity'] = dict(ssrcs=list(dict.fromkeys(r['payload']['inboundVideo'].get('ssrc') for r in stats)),
                              statsIds=list(dict.fromkeys(r['payload']['inboundVideo'].get('id') for r in stats)))
data['same1440pDrawCosts'] = {}
selected = [c for c in data['rendererDrawCosts'] if c['last'].get('outputWidth') == 2560 and c['last'].get('outputHeight') == 1440]
count = sum(c['count'] for c in selected)
data['same1440pDrawCosts'] = dict(count=count, costMsAvg={f: sum(c['count']*c['costMsAvg'][f] for c in selected)/count
                                                        for f in ['drawDurationTotalMs', 'uploadDurationTotalMs', 'submitDurationTotalMs']} if count else {})
data['freezeDetails'] = []
for anchor in stats:
    p = anchor['payload']; d = p.get('delta', {})
    if d.get('freezeCountDelta', 0) <= 0:
        continue
    at = p['sampledAtMs']
    preceding = [r for r in stats if at-1500 <= r['payload']['sampledAtMs'] <= at]
    data['freezeDetails'].append(dict(seq=anchor['seq'], sampledAtMs=at, durationSeconds=d['totalFreezesDurationDelta'],
        reportedInRendererGeneration=next((s['rendererGeneration'] for s in data['visibilityRendererPhases'] if s['startMs'] <= at < s['endMs']), None),
        intervals=[dict(seq=r['seq'], sampledAtMs=r['payload']['sampledAtMs'],
                        delta={k:r['payload'].get('delta', {}).get(k) for k in ['elapsedMs','packetsReceivedDelta','packetsLostDelta','framesReceivedDelta','framesDecodedDelta','framesDroppedDelta','keyFramesDecodedDelta','pliCountDelta','nackCountDelta']}) for r in preceding]))
data['submitAgeTails'] = [dict(seq=r['seq'], **{k:r['payload'].get(k) for k in ['rendererGeneration','sampledAtMs','observedAtMs','rendererSubmitAgeObservedAtMs','rendererSubmitAgeMs','maxCallbackIntervalMsSinceLastSample']})
                          for r in telemetry if r['payload'].get('rendererKind') != 'video'
                          and isinstance(r['payload'].get('rendererSubmitAgeMs'), (int, float)) and r['payload']['rendererSubmitAgeMs'] > 50]
data['srSubmitAgeByGeneration'] = {str(p['rendererGeneration']): summary([r['payload'].get('rendererSubmitAgeMs') for r in telemetry
    if r['payload'].get('rendererGeneration') == p['rendererGeneration']]) for p in data['visibilityRendererPhases'] if p['kind'] != 'video'}
freezing_generations = {p['rendererGeneration'] for p in data['visibilityRendererPhases']
                        if p['network']['changes']['freezeCountDelta'] > 0}
data['srSubmitAgeOutsideFreezingPhases'] = summary([r['payload'].get('rendererSubmitAgeMs') for r in telemetry
    if r['payload'].get('rendererKind') != 'video' and r['payload'].get('rendererGeneration') not in freezing_generations])
data['visibilityInterpretation'] = 'foregroundMedia excludes intervals straddling any policy change; with all targetFps positive and no visibility changes, full network metrics describe the active media span.'
data['pairedFreshAnalysisScriptSha256'] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
OUTPUT.write_text(json.dumps(data, ensure_ascii=False, indent=2)+'\n')
print(json.dumps(dict(output=str(OUTPUT), sourceMatchesCurrent=data['sourceMatchesCurrent'],
    paired={k:v for k,v in data['pairedCounterEvidence'].items() if k != 'sampleEvidence'},
    draw1440p=data['same1440pDrawCosts'], dimensions=data['sourceDimensionTransitions'],
    tails=data['submitAgeTails']), ensure_ascii=False, indent=2))
