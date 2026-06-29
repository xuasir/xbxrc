# Runtime Trace Schema

## Envelope

Each trace row is one JSON object in `jsonl` format.

Stable top-level fields:

- `schemaVersion`: current schema version. Treat version drift as a parsing risk. Version `2` adds `traceMode`; version `3` adds `traceProfile`, `dimension`, and `importance`.
- `seq`: per-file monotonic sequence number. Use this as the most stable row anchor.
- `tsMs`: wall-clock timestamp in milliseconds.
- `traceMode` (schema ≥2): compatibility field. On v3 traces it matches the effective profile (`off`, `production`, or `dev`). On v2 traces it may contain older settings such as `minimal`, `standard`, `verbose`, or `trace`.
- `traceProfile` (schema ≥3): effective writer profile. Values are `off`, `production`, or `dev`.
- `dimension` (schema ≥3): diagnostic surface for the row. Values are `core`, `lifecycle`, `network`, `recovery`, `media_supply`, `presentation`, `input`, `native_video`, `frontend`, or `engine_log`.
- `importance` (schema ≥3): retention class for the row. Values are `essential`, `key`, `debug`, or `raw`.
- `category`: one of `event`, `decision`, `state`, `snapshot`, `log`.
- `domain`: emitting subsystem, for example `streaming`, `xbxengine`, `trace`, or `data`.
- `event`: event name within the domain.
- `sessionId`: optional session correlation id. Many engine-level logs may still be `null`.
- `payload`: structured event payload.

Primary writer: `src-tauri/src/mods/runtime_trace/service.rs`.

## Profile And Budget Semantics

- `production`: release default. Keeps essential/key rows for the production dimensions and writes bounded trace files.
- `dev`: debug default. Keeps detailed diagnostics; `engine_log` is enabled only when the dimension expression includes it.
- `off`: closes the trace writer.
- Production file budget: 16MB per file, 5 files retained.
- Dev file budget: 64MB per file, 10 files retained.
- `fileOpened` rows include `traceProfile`, active `dimensions`, and `budget`.
- `budgetRotate` in `fileOpened.payload.reason` means the active file exceeded its profile budget and a new trace file opened.
- `traceBudgetNotice` means debug/raw rows were dropped under writer queue pressure. Treat essential/key rows as the retained evidence lane.

Dimension configuration:

- Production ignores custom dimension expressions and uses the fixed production set: `core,lifecycle,network,recovery,media_supply,presentation,frontend,native_video`.
- Dev accepts `XBX_TRACE_DIMENSIONS` or hidden config `runtime_trace_dimensions`.
- Dimension expressions are comma separated. Positive names build an allowlist, `-name` removes from the default set, and `all` enables every dimension.

Legacy mode mapping:

- `off`, `none`, `0` map to `off`.
- `production`, `prod`, `minimal`, and `standard` map to `production`.
- `dev`, `debug`, `verbose`, and `trace` map to `dev`.
- Release builds downgrade stored `dev` to effective `production`.

## Category Semantics

- `state`: durable phase or status transitions. Start here for timeline reconstruction.
- `decision`: branch decisions, escalation choices, or outcome selection. Use to explain *why* the code chose a path.
- `snapshot`: point-in-time state capture. Use for capability, context, metrics, or transport summaries.
- `event`: discrete lifecycle events. Use to anchor notable actions.
- `log`: raw log projection. Useful for detail, but usually secondary evidence.

## Interpretation Rules

- Prefer `seq` when discussing exact order inside a file.
- Use `tsMs` for duration and latency calculations.
- Treat `sessionId=null` as common, not necessarily a bug.
- When `payload.tsMs` exists inside `log` rows, prefer the top-level `tsMs` for cross-row comparisons unless you need emitter-local detail.
- When a conclusion depends only on free-form `payload.message`, verify whether a structured `state` / `decision` / `snapshot` row exists nearby.

## Typical Locations

- Trace files: `runtime-logs/runtime-trace-*.jsonl`
- Recorder: `src-tauri/src/mods/runtime_trace/service.rs`
- Streaming-side trace writes: `src-tauri/src/mods/streaming/service.rs`
- Engine projection: `src-tauri/src/mods/xbxengine/trace_projection.rs`
- Engine runtime snapshots: `src-tauri/src/mods/xbxengine/runtime_state.rs`

## Recovery-Focused Structured Events

When the question is about recovery effectiveness, prefer these structured events before reading raw logs:

- `pictureRecoveryTransition`:
  recovery phase progression mainline. Use `fromPhase`, `toPhase`, `phase`, `cause`, `detail`,
  `episodeId`, and `recoveryEpoch` to reconstruct the canonical chain
  `PliRequested -> PliSent -> ResponseObserved/PacketSeen -> Decoded -> CleanAnchorCommitted -> DisplayStable`.
  `stableServingSettled` is the close reason for `DisplayStable`.
- `pictureRecoveryBlockerObserved`:
  recovery gate blocker event. Use `gate`, `blockerKind`, `severity`, `firstObservedAtMs`, and `count`
  to identify whether the stall is at media gate (`cleanAnchorCommitted`) or display gate (`DisplayStable`),
  and whether the blocker is recurring or one-shot.
- `videoIngressTermination`:
  ingress termination causal chain. Use `terminationId`, `derivedFromTerminationId`, `kind`, `cause`,
  `upstreamCause`, `sourceSubsystem`, `linkedRecoveryEpoch`, and `linkedEpisodeId`
  to analyze `RtcVideoFrameSource rx closed` and its upstream closure reason.
- `firstFrameLatencyObserved`:
  first-frame latency breakdown across five segments:
  `controlReadyToPliSentMs`,
  `pliSentToFirstIdrPacketMs`,
  `firstIdrPacketToFirstDecodeMs`,
  `firstDecodeToCleanAnchorCommittedMs`,
  `cleanAnchorCommittedToDisplayStableMs`.
  Read `terminalPhase` and `incompleteReason` together to see where the chain stopped.
- `keyframeRequestEpisode`:
  canonical keyframe request lifecycle with `status`, `requestReason`, `requestKind`, `responseVerdict`,
  `firstKeyframePacketAtMs`, `firstKeyframeDecodedAtMs`, `timedOut`,
  `linkedH264AdmissionAccepted`, and `linkedH264BootstrapRejectReason`.
- `videoChainTransition`:
  use `chain.state` / `chain.reason` to check whether recovery actually rebuilt a healthy chain,
  instead of stopping at “packet seen” or “decoded”.
- `nackSent` / `nackRecovered` / `nackSkipped` / `nackExpired`:
  terminal NACK outcome classes. Read together with `action`, `nackDisposition`,
  `frameUnrecoverableReason`, and deadline fields.
- `recoveryDecisionLedger`:
  use `gateResult`, `actionSelected`, `recoveryPrimaryAction`, and `commandDetail`
  to identify suppression, coalescing, cooldown, failed-terminal, and unlock behavior.
- `h264InspectionObserved` / `h264InspectionRejected`:
  packet-level H264 bootstrap result. Prefer the newer fields
  `rejectClassification`,
  `boundRecoveryEpoch`,
  `episodePhaseAtObservation`,
  and `isPostRecoveryDegradation`
  to separate:
  - remote side never sent a usable IDR
  - local window admitted the packet but bootstrap rejected a delta continuation
  - recovery had already succeeded and the observation belongs to a later degradation
- `repairabilityScore` (or `repairability_score` / `repairability` / `repairabilityIndex`):
  repairability 评分样本字段。分析时应检查连续性（coverage、max missing streak、longest gap），
  不要只看单个时间点。

Script-level recovery aggregates (from `scripts/summarize_runtime_trace.py`):

- `recoveryAudit.keyframeEffectiveness.chainBuildSuccessRate`:
  decoded 后真正恢复到 healthy chain 的聚合成功率。
- `recoveryAudit.nackEffectiveness.effectiveRate`:
  NACK 有效恢复占已发送 NACK 的聚合比例。
- `recoveryAudit.repairabilityPersistence`:
  repairability 评分持久化统计（样本数、均值、连续缺失段、最长缺口等）。
- `recoveryAudit.recoveryEffectiveness.score`:
  综合恢复有效性评分（基于 keyframe/NACK/建链/repairability 持久化的加权结果）。

## Browser-Direct Render Structured Events

当问题集中在浏览器直连模式的 WebGL2 / video 呈现稳定性时，优先读这些结构化事件：

- `renderTelemetryObserved`:
  浏览器端绘制 sample 窗口总览。重点字段：
  - `trackingSource`: `videoFrameCallback` 或 `timeupdate`
  - `callbackCountSinceLastSample`: sample 窗口内浏览器回调次数
  - `callbackGapCountSinceLastSample`: sample 窗口内回调间隔超过本地阈值的次数
  - `callbackIntervalMs`
  - `presentedFramesAdvancedSinceLastSample`: sample 窗口内 `presentedFrames` 总推进量
  - `presentedFramesDelta`
  - `presentedFramesJumpCountSinceLastSample`: sample 窗口内 `presentedFramesDelta > 1` 的次数
  - `mediaTimeDeltaSec`
  - `expectedDisplayLeadMs`
  - `sourceFpsEstimate`
  - `sourceFrameIntervalMs`
  - `droppedFrames`
  - `droppedFramesSinceLastSample`
  - `droppedLikeStreak`
  - `frameEventsSinceLastSample`
  - `maxCallbackIntervalMsSinceLastSample`
  - `maxPresentedFramesDeltaSinceLastSample`
  - `renderBackpressure`
  - `renderCause`
  - `displayDegradeLevel`
  这条事件适合回答“当前时间窗内回调节奏稳不稳、实际呈现推进有没有接近 60、源节拍稳不稳、局部跳帧是否在累积”。
- `renderFrameDropped`:
  单次 dropped-like 证据。它表示浏览器侧回调间隔或 `presentedFrames` 推进跨过本地阈值，
  适合作为“哪一拍开始恶化”的锚点。它不直接等价于 GPU draw failure。
  重点补充字段：
  - `callbackGap`: 该次事件是否由 callback 间隔过长触发
  - `presentedFramesJump`: 该次事件是否由 `presentedFramesDelta > 1` 触发
- `renderBackpressureChanged`:
  本地 backpressure 状态切换锚点。结合 `callbackIntervalMs`、`backpressureThresholdMs`、
  `sourceFpsEstimate` 与 `sourceFrameIntervalMs` 判断是偶发长尾还是持续供给不足。
- `renderCauseClassified`:
  浏览器端本地 render 解释层。用 `cause`、`renderDecisionDigest`、`renderBackpressure`、
  `frontEndVideoFrameSourceFps`、`frontEndVideoFrameSourceFpsCeiling` 区分：
  - `renderStable`
  - `renderStarvation`
  - `decodeBackpressure`
- `renderPolicyApplied`:
  当前浏览器 renderer 策略落地事件。用 `pipelineType`、`processing`、`processingMode`、
  `shaderPreset`、`displayDegradeLevel`、`renderFpsBudget`、`reason` 判断 runtime 是否已切换
  WebGL2 / 原生 video / SR 路径，或是否进入 display degrade。

浏览器端绘制字段的实用解释：

- `trackingSource=videoFrameCallback`:
  更接近浏览器真实解码/呈现节拍，优先级高于 `timeupdate`。
- `trackingSource=timeupdate`:
  fallback 粗粒度节拍，适合判定“明显卡顿”，不适合做精细 30/60fps 结论。
- `presentedFramesDelta > 1`:
  当前回调窗口内浏览器已呈现帧跨步推进，表示跳帧或批量呈现迹象。
- `callbackGapCountSinceLastSample > 0` 且 `presentedFramesAdvancedSinceLastSample` 仍接近 60fps:
  优先判定为 callback 稀疏，不要直接把它读成“真实显示只有 50fps”。
- `droppedLikeStreak`:
  连续 sample 中 dropped-like 现象是否持续，适合区分偶发毛刺和持续抖动。
- `sourceFpsEstimate` / `sourceFrameIntervalMs`:
  视频源节拍估算。优先用于区分源本身是 30fps，还是 60fps 源在本地绘制阶段出现抖动。

## Practical Read Order

1. File open and session bootstrap rows.
2. `pictureRecoveryTransition` / `pictureRecoveryBlockerObserved` / `videoIngressTermination` / `firstFrameLatencyObserved`.
3. `state` rows for phase movement.
4. `decision` rows for branch reasoning.
5. `snapshot` rows for capability, transport, and performance context.
6. `log` rows only around suspicious windows.
7. For browser-direct render investigations, add this local render order:
   `renderTelemetryObserved` -> `renderFrameDropped` -> `renderBackpressureChanged`
   -> `renderCauseClassified` -> `renderPolicyApplied`.
