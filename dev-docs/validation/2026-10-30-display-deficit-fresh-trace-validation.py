import collections
import hashlib
import json
import math
import statistics
import subprocess
import sys
from datetime import datetime
from pathlib import Path
from zoneinfo import ZoneInfo

ROOT = Path(__file__).resolve().parents[2]
LOG = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793342364772-1.jsonl')
rows = [json.loads(line) for line in LOG.read_text().splitlines() if line.strip()]
events = collections.Counter(row['event'] for row in rows)

def quantile(values, p):
    if not values:
        return None
    values = sorted(values)
    at = (len(values) - 1) * p
    lo, hi = math.floor(at), math.ceil(at)
    return values[lo] + (values[hi] - values[lo]) * (at - lo)

def summary(values):
    values = [v for v in values if isinstance(v, (int, float)) and math.isfinite(v)]
    return dict(n=len(values), mean=statistics.mean(values) if values else None,
                p50=quantile(values, .5), p95=quantile(values, .95), p99=quantile(values, .99),
                min=min(values) if values else None, max=max(values) if values else None)

def bounds(items, key='sampledAtMs'):
    return dict(seq=[items[0]['seq'], items[-1]['seq']],
                monotonicMs=[items[0]['payload'][key], items[-1]['payload'][key]],
                wallClock=[datetime.fromtimestamp(item['tsMs']/1000, ZoneInfo('Asia/Shanghai')).isoformat() for item in [items[0], items[-1]]])

stats = [r for r in rows if r['event'] == 'browserWebRtcStatsObserved']
telemetry = [r for r in rows if r['event'] == 'renderTelemetryObserved']
consumed = [r for r in rows if r['event'] == 'browserObservationConsumed' and r['payload'].get('consumer') == 'supervision']
resources = [r for r in rows if r['event'] == 'browserMediaResourcesSnapshot']

def network(start=None, end=None):
    selected = [r for r in stats if (start is None or r['payload']['sampledAtMs'] >= start) and (end is None or r['payload']['sampledAtMs'] <= end)]
    if len(selected) < 2:
        return None
    a, b = [r['payload'] for r in [selected[0], selected[-1]]]
    first, last = a['inboundVideo'], b['inboundVideo']
    elapsed = (b['sampledAtMs'] - a['sampledAtMs']) / 1000
    changes = {k: last.get(k, 0) - first.get(k, 0) for k in ['packetsReceived','packetsLost','bytesReceived','framesReceived','framesDecoded','framesDropped','freezeCount','totalFreezesDuration','pauseCount','pliCount','firCount','nackCount','keyFramesDecoded','retransmittedPacketsReceived','retransmittedBytesReceived','jitterBufferDelay','jitterBufferEmittedCount','totalDecodeTime','totalProcessingDelay','totalAssemblyTime']}
    positive_loss = sum(max(0, r['payload'].get('delta', {}).get('packetsLostDelta', 0)) for r in selected[1:])
    emitted = changes['jitterBufferEmittedCount']
    decoded = changes['framesDecoded']
    per_interval = []
    for r in selected[1:]:
        p = r['payload']; d = p.get('delta', {})
        n = d.get('framesDecodedDelta', 0); nbuf = d.get('jitterBufferEmittedCountDelta', 0)
        if n > 0 and nbuf > 0:
            per_interval.append(dict(seq=r['seq'], sampledAtMs=p['sampledAtMs'],
                jitterBufferMs=d.get('jitterBufferDelayDelta', 0)*1000/nbuf,
                decodeMs=d.get('totalDecodeTimeDelta', 0)*1000/n,
                processingMs=d.get('totalProcessingDelayDelta', 0)*1000/n,
                assemblyMs=d.get('totalAssemblyTimeDelta', 0)*1000/n))
    pairs = [r['payload'].get('selectedCandidatePair', {}) for r in selected]
    return dict(bounds=bounds(selected), durationSeconds=elapsed, counterChanges=changes,
        receivedFps=changes['framesReceived']/elapsed, decodedFps=decoded/elapsed,
        bitrateMbps=changes['bytesReceived']*8/elapsed/1e6,
        positivePacketsLost=positive_loss, positiveLossRatio=positive_loss/(changes['packetsReceived']+positive_loss),
        netLossRatio=changes['packetsLost']/(changes['packetsReceived']+changes['packetsLost']),
        rttMs=summary([p.get('currentRoundTripTime', float('nan'))*1000 for p in pairs]),
        jitterMs=summary([r['payload']['inboundVideo'].get('jitter', float('nan'))*1000 for r in selected]),
        sampleDelayMs={k: summary([p[k] for p in per_interval]) for k in ['jitterBufferMs','decodeMs','processingMs','assemblyMs']},
        weightedDelayMs=dict(jitterBuffer=changes['jitterBufferDelay']*1000/emitted,
            decode=changes['totalDecodeTime']*1000/decoded, processing=changes['totalProcessingDelay']*1000/decoded,
            assembly=changes['totalAssemblyTime']*1000/decoded),
        connectionStates=dict(collections.Counter(r['payload'].get('connectionState') for r in selected)),
        pairIds=list(dict.fromkeys(p.get('id') for p in pairs)),
        candidateTypes=list(dict.fromkeys((p.get('localCandidate',{}).get('candidateType'),p.get('remoteCandidate',{}).get('candidateType')) for p in pairs)),
        worstDelaySamples=sorted(per_interval,key=lambda p:p['jitterBufferMs'],reverse=True)[:4])

groups=[]
for row in telemetry:
    p=row['payload']; key=(p.get('rendererKind'),p.get('rendererGeneration'),p.get('sourceWidth'),p.get('sourceHeight'))
    if not groups or groups[-1][0]!=key:
        groups.append((key,[]))
    groups[-1][1].append(row)

segments=[]
for key, items in groups:
    a,b=items[0]['payload'],items[-1]['payload']; start=a['sampledAtMs']; end=b['sampledAtMs']
    info=dict(key=key,bounds=bounds(items),telemetryCount=len(items))
    info['network']=network(start,end)
    info['windows']={}
    for label,length in [('short',3000),('long',15000)]:
        valid=[]
        for r in consumed:
            p=r['payload']; f=p.get('facts',{}); w=f.get('windows',{}).get(label,{})
            if p.get('rendererGeneration')==key[1] and f.get('sourceWidth')==key[2] and start+length<=p.get('sampledAtMs',-1)<=end and all(w.get(c,0)>=length-1 for c in ['receivedCoverageMs','decodeCoverageMs','callbackCoverageMs','rendererCoverageMs','videoCoverageMs']):
                valid.append(r)
        info['windows'][label]=dict(n=len(valid),bounds=bounds(valid) if valid else None,
            fields={f:summary([r['payload']['facts']['windows'][label].get(f) for r in valid]) for f in ['inboundFpsAvg','decodeFpsAvg','videoPresentFpsAvg','callbackFpsAvg','rendererSubmitFpsAvg','renderDrawDurationMsAvg','renderUploadDurationMsAvg','renderSubmitDurationMsAvg','rendererSubmitAgeMsAvg','rttMsAvg','jitterMsAvg','lossAvg','jitterBufferMsAvg']})
    info['telemetryFields']={f:summary([r['payload'].get(f) for r in items]) for f in ['rendererSubmitAgeMs','renderDrawDurationMsAvg','renderUploadDurationMsAvg','renderSubmitDurationMsAvg','maxCallbackIntervalMsSinceLastSample','callbackCountSinceLastSample','presentedFramesAdvancedSinceLastSample','droppedFramesSinceLastSample','sourceFpsEstimate','rendererSubmitAgeObservedAtMs']}
    info['sampleTotals']={f:sum(r['payload'].get(f,0) for r in items) for f in ['callbackCountSinceLastSample','callbackGapCountSinceLastSample','presentedFramesAdvancedSinceLastSample','presentedFramesJumpCountSinceLastSample','droppedFramesSinceLastSample']}
    info['renderCauseCounts']=dict(collections.Counter(r['payload'].get('renderCause') for r in items))
    info['trackingSourceCounts']=dict(collections.Counter(r['payload'].get('trackingSource') for r in items))
    # 即时诊断与缓存统计可以跨代际；各自时钟只结算同一 renderer。
    live_items = [r for r in items if r['payload'].get('mediaDiagnostics', {}).get('rendererGeneration') == key[1]
                  and r['payload'].get('mediaDiagnostics', {}).get('rendererKind') == key[0]]
    info['mixedGenerationTelemetrySeqs'] = [r['seq'] for r in items if r not in live_items]
    la, lb = (live_items[0]['payload'], live_items[-1]['payload']) if live_items else (a, a)
    d0,d1=[p.get('mediaDiagnostics',{}) for p in [la,lb]]; i0,i1=[p.get('frameIdentity',{}) for p in [d0,d1]]
    elapsed=(d1.get('observedAtMs',lb['observedAtMs'])-d0.get('observedAtMs',la['observedAtMs']))/1000
    if elapsed>0:
        marker=i1.get('localSequence',0)-i0.get('localSequence',0)
        submitted=None if 'submittedFramesTotal' not in d1 or 'submittedFramesTotal' not in d0 else d1['submittedFramesTotal']-d0['submittedFramesTotal']
        info['liveDiagnosticsDelta']=dict(durationSeconds=elapsed,marker=marker,markerFps=marker/elapsed,
            nativeObserved=i1.get('observed',{}).get('videoFrameCallback',0)-i0.get('observed',{}).get('videoFrameCallback',0),
            nativeAccepted=i1.get('accepted',{}).get('videoFrameCallback',0)-i0.get('accepted',{}).get('videoFrameCallback',0),
            pollAccepted=i1.get('accepted',{}).get('animationFrame',0)-i0.get('accepted',{}).get('animationFrame',0),
            submitted=submitted,submitFps=submitted/elapsed if submitted is not None else None)
    matching=[]
    for r in resources:
        p=r['payload']; ob=p.get('resources',{}).get('rendererObservation',{})
        if ob.get('generation')==key[1] and start<=p.get('observedAtMs',-1)<=end:
            matching.append(r)
    if len(matching)>=2 and key[0]!='video':
        first,last=[r['payload']['resources']['rendererObservation']['resources'] for r in [matching[0],matching[-1]]]
        count=last['realtimeDrawCount']-first['realtimeDrawCount']; elapsed=(matching[-1]['payload']['observedAtMs']-matching[0]['payload']['observedAtMs'])/1000
        info['resourceDrawDelta']=dict(seq=[matching[0]['seq'],matching[-1]['seq']],durationSeconds=elapsed,
            realtimeDrawCount=count,drawFps=count/elapsed,drawDurationMsAvg=(last['drawDurationTotalMs']-first['drawDurationTotalMs'])/count,
            cumulativeDrawDurationMaxMs=last['drawDurationMaxMs'],last=last)
    segments.append(info)

evidence={}
for r in telemetry:
    for e in r['payload'].get('mediaDiagnostics',{}).get('frameIdentity',{}).get('evidence',[]):
        evidence[(e['atMs'],e['trackingSource'])]=e
native=[e for e in evidence.values() if e['trackingSource']=='videoFrameCallback']
inputs=['src','src-tauri/src','src-tauri/build.rs','src-tauri/tauri.conf.json','src-tauri/Cargo.toml','crates','Cargo.toml','Cargo.lock','rust-toolchain.toml','package.json','pnpm-lock.yaml','vite.config.ts','tsconfig.json','tsconfig.node.json','tsconfig.app.json']
paths=sorted(set(subprocess.check_output(['git','ls-files','-z','--cached','--others','--exclude-standard','--',*inputs],cwd=ROOT).decode().strip('\0').split('\0')))
hash=hashlib.sha256()
for relative in paths:
    name=relative.encode(); hash.update(len(name).to_bytes(8,'little'));hash.update(name)
    path=ROOT/relative
    if path.exists():
        content=path.read_bytes();hash.update(b'\1');hash.update(len(content).to_bytes(8,'little'));hash.update(content)
    else: hash.update(b'\0')
build=next(r['payload']['buildFingerprint'] for r in rows if r['event']=='runtimeBuildInfo')
data=dict(trace=str(LOG),traceSha256=hashlib.sha256(LOG.read_bytes()).hexdigest(),rows=len(rows),
    traceBounds=dict(firstSeq=rows[0]['seq'],lastSeq=rows[-1]['seq'],durationSeconds=(rows[-1]['tsMs']-rows[0]['tsMs'])/1000),
    build=build,currentSourceFingerprint=hash.hexdigest(),sourceInputCount=len(paths),sourceMatchesCurrent=hash.hexdigest()==build['sourceFingerprintSha256'],
    events=dict(events),network=network(),segments=segments,
    frameIdentity=dict(latest=telemetry[-1]['payload']['mediaDiagnostics']['frameIdentity'],uniqueEvidence=len(evidence),nativeEvidence=len(native),nativeStateCounts=dict(collections.Counter(e['nativeState'] for e in native)),rtpStateCounts=dict(collections.Counter(e['rtpTimestampState'] for e in native)),mediaTimeStateCounts=dict(collections.Counter(e['mediaTimeState'] for e in native))),
    collectionDurationMs=summary([r['payload'].get('collectionDurationMs') for r in stats]),
    traceQueue={f:summary([r['payload'].get('trace',{}).get(f) for r in telemetry]) for f in ['inFlight','pending','dropped','droppedEssential']},
    limitations=['GPU execution time and compositor/screen presentation unavailable','Browser-internal ReferenceChain and per-request NACK/keyframe recovery outcomes unavailable','Per-sample delay percentiles describe window means, not individual frames','Telemetry sample totals can span segment transitions; same-generation monotonic live deltas and fully covered policy windows are primary rate evidence'])
sr_resource_rows = [r for r in resources if r['payload'].get('resources', {}).get('rendererObservation', {}).get('kind') == 'webgl2_sr']
data['srLiveSubmitAgeMs'] = summary([r['payload']['observedAtMs'] - r['payload']['resources']['rendererObservation']['resources']['lastDrawAtMs'] for r in sr_resource_rows])
data['collectionDurationBySegmentMs'] = [dict(key=s['key'], values=summary([r['payload']['collectionDurationMs'] for r in stats if s['bounds']['monotonicMs'][0] <= r['payload']['sampledAtMs'] <= s['bounds']['monotonicMs'][1]])) for s in segments]
data['networkBurstAnchors'] = [dict(seq=r['seq'], sampledAtMs=r['payload']['sampledAtMs'], delta={k:v for k,v in r['payload'].get('delta', {}).items() if k in ['elapsedMs','packetsReceivedDelta','packetsLostDelta','framesReceivedDelta','framesDecodedDelta','freezeCountDelta','totalFreezesDurationDelta','pliCountDelta','nackCountDelta','keyFramesDecodedDelta']}) for r in stats if r['payload'].get('delta',{}).get('freezeCountDelta',0)>0 or r['payload'].get('delta',{}).get('packetsLostDelta',0)>=30]
data['recoveryObservability'] = dict(keyframeRequestsSuppressed='unavailable', requestResponsePairing='unavailable', decodedKeyframeDelta=data['network']['counterChanges']['keyFramesDecoded'], cleanAnchorCommitted='unavailable', DisplayStable='unavailable', nackEffectiveRate='unavailable', chainBuildSuccessRate='unavailable', repairabilityPersistence='unavailable', recoveryEffectiveness='unavailable')
browser_report = json.loads(subprocess.check_output([sys.executable, '-B', str(ROOT/'.agents/skills/analyze-runtime-logs/scripts/trace_browser_webrtc_behavior_report.py'), str(LOG)]))
receive_report = json.loads(subprocess.check_output([sys.executable, '-B', str(ROOT/'.agents/skills/analyze-runtime-logs/scripts/trace_receive_feedback_report.py'), str(LOG)]))
data['gates'] = dict(browserBehavior=browser_report['browserWebRtcBehaviorGate'], browserRuntimeStability={k:v for k,v in browser_report['browserRuntimeStability'].items() if k in ['gate','failures','evidence','missingEvidence']}, receiveFeedback=dict(gate=receive_report['receiveFeedbackGate'], summary=receive_report['summary'], rates=receive_report['rates'], keyframeChain=receive_report['keyframeChain']))
# 公共统计沿用上一份分析口径，具体启动/年龄锚点由本次日志重建。
data['analysisScriptSha256'] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()

# 原始同代际资源增量补足短SR段；窗口均值与实际处理计数分别保存。
resource_rows = [r for r in rows if r['event'] in ['browserMediaResourcesObserved', 'browserMediaResourcesSnapshot']
                 and r['payload'].get('resources', {}).get('rendererObservation')]
draw_costs = []
for generation in sorted(set(r['payload']['resources']['rendererObservation']['generation'] for r in resource_rows)):
    selected = [r for r in resource_rows if r['payload']['resources']['rendererObservation']['generation'] == generation]
    kind = selected[0]['payload']['resources']['rendererObservation']['kind']
    if kind == 'video':
        continue
    first, last = selected[0], selected[-1]
    a, b = [r['payload']['resources']['rendererObservation']['resources'] for r in [first, last]]
    count = b.get('realtimeDrawCount', 0) - a.get('realtimeDrawCount', 0)
    seconds = (last['payload']['observedAtMs'] - first['payload']['observedAtMs']) / 1000
    costs = {field: (b.get(field, 0) - a.get(field, 0)) / count if count else None
             for field in ['drawDurationTotalMs', 'uploadDurationTotalMs', 'submitDurationTotalMs']}
    draw_costs.append(dict(rendererGeneration=generation, rendererKind=kind, seq=[first['seq'], last['seq']],
        durationSeconds=seconds, count=count, realtimeFps=count/seconds if seconds else None,
        costMsAvg=costs, countedCostSamples=b.get('drawCostSampleCount',0)-a.get('drawCostSampleCount',0), last=b))
data['rendererDrawCosts'] = draw_costs
cost_count = sum(c['count'] for c in draw_costs)
cost_totals = {field:sum(c['count'] * c['costMsAvg'][field] for c in draw_costs if c['count'])
               for field in ['drawDurationTotalMs','uploadDurationTotalMs','submitDurationTotalMs']}
data['combinedDrawCosts'] = dict(count=cost_count,
    avgMs={field:total/cost_count if cost_count else None for field,total in cost_totals.items()},
    uploadShare=cost_totals['uploadDurationTotalMs']/cost_totals['drawDurationTotalMs'] if cost_totals['drawDurationTotalMs'] else None)

actions = [r for r in rows if r['event'] in ['renderLocalActionPlanned', 'renderLocalActionEffectEvaluated', 'renderPipelineSwitched']]
data['renderActions'] = [dict(seq=r['seq'], tsMs=r['tsMs'], event=r['event'], payload=r['payload']) for r in actions]
data['rendererLifecycle'] = [dict(seq=r['seq'], tsMs=r['tsMs'], observedAtMs=r['payload']['observedAtMs'], reason=r['payload']['reason'],
    current=r['payload'].get('resources',{}).get('rendererObservation',{}).get('generation'),
    readyTimers=r['payload'].get('resources',{}).get('readyTimers'),
    renderers=r['payload'].get('resources',{}).get('renderers',[]))
    for r in rows if r['event']=='browserMediaResourcesObserved']

data['startup'] = dict(
    milestones=browser_report.get('firstFrameLatencies'),
    timeline=[dict(seq=r['seq'], tsMs=r['tsMs'], kind=r['payload'].get('kind'), observedAtMs=r['payload'].get('observedAtMs'))
              for r in rows if r['event']=='browserWebRtcTimelineObserved'],
    failures=[dict(seq=r['seq'], payload=r['payload']) for r in rows
              if r['event']=='browserMediaResourcesObserved' and r['payload'].get('reason')=='rendererFailed'])
data['schema'] = dict(profiles=dict(collections.Counter(r.get('traceProfile') for r in rows)),
                     importance=dict(collections.Counter(r.get('importance') for r in rows)),
                     budgetNotices=events.get('traceBudgetNotice',0))

data['telemetryCoverage'] = dict(submitAgeMs=summary([r['payload'].get('rendererSubmitAgeMs') for r in telemetry if r['payload'].get('rendererKind')!='video']),
    liveSameGeneration=[dict(seq=r['seq'], sampledAtMs=r['payload']['sampledAtMs'], observedAtMs=r['payload']['observedAtMs'],
                            ageObservedAtMs=r['payload'].get('rendererSubmitAgeObservedAtMs'), submitAgeMs=r['payload'].get('rendererSubmitAgeMs'))
                       for r in telemetry if r['payload'].get('rendererKind')!='video'],
    collectionDurationByKind={kind:summary([r['payload']['collectionDurationMs'] for r in stats
                        if any(s['key'][0]==kind and s['bounds']['monotonicMs'][0] <= r['payload']['sampledAtMs'] <= s['bounds']['monotonicMs'][1] for s in segments)])
                         for kind in set(s['key'][0] for s in segments)})
data['callbackEvidence'] = [dict(rendererGeneration=g,
    maxCallbackIntervalMs=max(r['payload'].get('maxCallbackIntervalMsSinceLastSample',0) for r in telemetry if r['payload'].get('rendererGeneration')==g),
    numericSourceFpsSamples=sum(isinstance(r['payload'].get('sourceFpsEstimate'),(int,float)) for r in telemetry if r['payload'].get('rendererGeneration')==g),
    sourceFpsUnavailableReasons=dict(collections.Counter(r['payload'].get('sourceFpsUnavailableReason') for r in telemetry if r['payload'].get('rendererGeneration')==g)))
    for g in sorted(set(r['payload']['rendererGeneration'] for r in telemetry if r['payload'].get('rendererGeneration') is not None))]
data['scene'] = dict(displayPolicy=[r['payload'] for r in rows if r['event']=='renderPolicyApplied'],
    sourceDimensions=dict(collections.Counter(f"{r['payload']['inboundVideo'].get('frameWidth')}x{r['payload']['inboundVideo'].get('frameHeight')}" for r in stats)),
    surfaces=[dict(seq=r['seq'], rendererGeneration=r['payload'].get('mediaDiagnostics',{}).get('rendererGeneration'),
        rendererKind=r['payload'].get('mediaDiagnostics',{}).get('rendererKind'),
        video=r['payload'].get('mediaDiagnostics',{}).get('surface',{}).get('video'),
        canvas=r['payload'].get('mediaDiagnostics',{}).get('surface',{}).get('canvas')) for r in telemetry])

output = Path(sys.argv[2]) if len(sys.argv)>2 else ROOT/'dev-docs/validation/2026-10-30-display-deficit-fresh-trace-validation.metrics.json'
output.write_text(json.dumps(data,ensure_ascii=False,indent=2)+'\n')
print(json.dumps(dict(output=str(output), sourceMatchesCurrent=data['sourceMatchesCurrent'],
    network=data['network'], drawCosts=draw_costs,
    segments=[dict(key=s['key'],seq=s['bounds']['seq'], shortWindowCount=s['windows']['short']['n'],
                   longWindowCount=s['windows']['long']['n'],live=s.get('liveDiagnosticsDelta'),
                   windowFps={f:values['p50'] for f,values in s['windows']['short']['fields'].items()}) for s in segments]),ensure_ascii=False,indent=2))
