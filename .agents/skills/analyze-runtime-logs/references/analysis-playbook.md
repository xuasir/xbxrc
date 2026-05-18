# Runtime Log Analysis Playbook

## Use This Playbook For

- Startup or provisioning failures
- Streaming session regressions
- RTC or media pipeline stalls
- Performance review from runtime traces
- Cross-trace comparison after a code change

## Analysis Procedure

### 1. Define The Question

Classify the task before reading everything:

- incident triage
- regression verification
- performance evaluation
- comparison between two traces

This changes what “important” means.

### 2. Establish The Skeleton Timeline

Collect:

- first and last `tsMs`
- duration
- active `sessionId`
- first major `state` rows
- terminal failure or recovery rows

Do not start from free-form `log` spam.

### 3. Find The First Abnormal Signal

Search for the earliest row that indicates drift:

- timeout
- unavailable
- dropped
- stall
- reconnect
- recovery escalation
- missing capability or null capability snapshot
- no ingress / no media / no ICE progress

The first abnormal signal is usually more useful than the last emitted error.

### 4. Correlate Across Layers

Map the trace across these boundaries:

- UI or command trigger
- Tauri service orchestration
- data or preflight snapshots
- `xbxengine` runtime observations
- transport / media / recovery consequences

Do not infer an engine fault when the trace already shows upstream provisioning or capability failure.

### 5. Separate Failure Modes

Keep these cases distinct:

- service-side provisioning never becomes ready
- capability or remote-play preflight is missing required fields
- transport is connected but media never flows
- media flows but recovery cannot obtain a keyframe
- media ingress is present but local pipeline is backpressured
- performance is degraded without a terminal failure
- browser-direct render callback cadence is unstable while transport and decode still look healthy

For keyframe and NACK analysis, split outcomes further:

- keyframe request was suppressed or coalesced before send
- keyframe request was sent but no response was observed
- response packet arrived but H264 bootstrap / admission rejected it
- response packet arrived, local window admitted it, but bootstrap rejected `NonIdrVcl` / delta continuation
- keyframe decoded but `cleanAnchorCommitted` did not happen
- `cleanAnchorCommitted` happened but `DisplayStable` did not happen
- `DisplayStable` happened and later degradation started a new recovery episode
- NACK recovered in time
- NACK recovered late
- NACK was skipped by policy
- NACK expired and should hand off to keyframe / stronger recovery
- `rx closed` is the initiating cause for the episode
- `rx closed` is a downstream result after prior recovery failure
- chain build success rate (decoded -> healthy chain) aggregation
- NACK effective rate aggregation
- repairability score persistence continuity (sample coverage / missing streak / missing gap)
- recovery effectiveness composite score (keyframe + chain build + NACK + repairability persistence)

For browser-direct render analysis, split outcomes further:

- source cadence is stable, callback cadence is stable, local render is healthy
- source cadence is stable, callback cadence has long-tail spikes, local render shows intermittent backpressure
- source cadence is stable, `presentedFramesDelta` repeatedly jumps, browser side is skipping or batch-presenting frames
- source cadence already falls to 30fps or lower, local render follows source and is not the primary bottleneck
- `trackingSource=timeupdate` only, current trace supports coarse stutter judgement and supports limited fps attribution

### 6. Evaluate Performance Explicitly

For performance analysis, always state:

- measured time window
- startup or recovery phase boundaries
- obvious long gaps between milestones
- repeated retries, reconnects, or resets
- sustained low-throughput / no-throughput windows
- whether evidence points to network, provisioning, control-plane, or local pipeline pressure
- for browser-direct mode, whether evidence points to source cadence, callback cadence, decode pressure, or local drawing pressure

## Output Format

Use this template unless the user asks for something else:

### Scope

- file(s)
- question answered

### Timeline

- earliest relevant anchors with `seq` / `tsMs`
- first abnormal signal
- terminal symptom or recovery point
- if recovery is involved, list structured events in this order:
  - `pictureRecoveryTransition`
  - `pictureRecoveryBlockerObserved`
  - `videoIngressTermination`
  - `firstFrameLatencyObserved`
- then place the stall on the canonical chain:
  - `PliRequested -> PliSent -> ResponseObserved/PacketSeen -> Decoded -> CleanAnchorCommitted -> DisplayStable`
- if browser-direct render is involved, list structured events in this order:
  - `renderTelemetryObserved`
  - `renderFrameDropped`
  - `renderBackpressureChanged`
  - `renderCauseClassified`
  - `renderPolicyApplied`

### Findings

- high-confidence finding
- secondary observations
- competing hypotheses if confidence is limited
- if keyframe/NACK is involved, say whether it was effective, merely attempted, or explicitly invalid
- if H264 inspection is involved, say whether `rejectClassification` points to:
  - remote missing usable IDR
  - local admission accepted but bootstrap rejected continuation delta
  - post-recovery degradation
- include aggregate rates/scores when available:
  - `keyframeEffectiveness.chainBuildSuccessRate`
  - `nackEffectiveness.effectiveRate`
  - `repairabilityPersistence`
  - `recoveryEffectiveness.score`
- for browser-direct render, state these fields explicitly when available:
  - `trackingSource`
  - `callbackCountSinceLastSample`
  - `callbackGapCountSinceLastSample`
  - `presentedFramesAdvancedSinceLastSample`
  - `sourceFpsEstimate`
  - `sourceFrameIntervalMs`
  - `mediaTimeDeltaSec`
  - `expectedDisplayLeadMs`
  - `callbackIntervalMs`
  - `maxCallbackIntervalMsSinceLastSample`
  - `presentedFramesDelta`
  - `presentedFramesJumpCountSinceLastSample`
  - `maxPresentedFramesDeltaSinceLastSample`
  - `droppedFramesSinceLastSample`
  - `droppedLikeStreak`
  - `renderBackpressure`
  - `renderCause`
  - `displayDegradeLevel`

### Evidence

- rows, payload fields, and transitions that support the conclusion

### Gaps

- what the trace does not prove
- which instrumentation is still missing

### Next Step

- concrete code path to inspect
- extra trace to capture
- targeted experiment to run

## Project-Specific Hints

- Prior analyses are recorded in `docs/project-task.md` and archived task files. Reuse prior terminology when a trace resembles an earlier incident.
- `remoteManagementEnabled`, `consoleStreamingEnabled`, and address-count snapshots are often decisive for home streaming preflight analysis.
- For `xbxengine` traces, watch for transport progress, ingress activity, recovery state, keyframe flow, and backlog or capacity warnings before blaming rendering.
- When performance degrades without a hard failure, compare `snapshot` rows and recovery decisions before focusing on individual debug lines.
- `keyframeRequestEpisode` is the canonical keyframe request lifecycle. Prefer it over `recovery_keyframe_request_count` when the question is about success/failure/effectiveness.
- `pictureRecoveryTransition` is the canonical recovery phase chain. Prefer it over free-form `keyframe-closure` text logs.
- `pictureRecoveryBlockerObserved` is the canonical gate blocker signal. Prefer it over inferring stalls from scattered owner / display debug rows.
- `videoIngressTermination` carries the causal labels for `RtcVideoFrameSource rx closed`. Read `cause` together with `upstreamCause`.
- `firstFrameLatencyObserved` is the canonical first-frame latency breakdown. Prefer it over parsing `firstFrameLatencyTrace ...` text.
- `videoChainTransition` is the quickest structured check for “关键帧成功后是否真的建链恢复 healthy”.
- `nackSent` / `nackRecovered` / `nackSkipped` / `nackExpired` already encode terminal NACK outcome classes; pair them with `nackDisposition` and `frameUnrecoverableReason`.
- `cleanAnchorCommitted` is the media gate; `DisplayStable` is the display gate; `stableServingSettled` is the event name / close reason for `DisplayStable`.
- repairability score may appear as `repairabilityScore` / `repairability_score` / `repairability` / `repairabilityIndex`; persistence should be judged from continuity, not single-point value.
- `renderTelemetryObserved` is the canonical browser-direct render sample summary. Prefer it over guessing from sparse `fps/presentFps` snapshots.
- `trackingSource=videoFrameCallback` supports finer render cadence judgement than `timeupdate`.
- `callbackGapCountSinceLastSample` separates callback sparsity from aggregate dropped-like counts.
- `presentedFramesJumpCountSinceLastSample` separates `presentedFrames` coalescing from callback cadence issues.
- `renderFrameDropped` reports dropped-like cadence evidence. Read it with `callbackGap`, `presentedFramesJump`, `presentedFramesDelta`, and `renderBackpressure`, then decide whether the issue is local draw pressure, callback batching, or source-side cadence drift.
