# Runtime Trace Schema

## Envelope

Each trace row is one JSON object in `jsonl` format.

Stable top-level fields:

- `schemaVersion`: current schema version. Treat version drift as a parsing risk. Version `2` adds `traceMode` and keeps prior fields.
- `seq`: per-file monotonic sequence number. Use this as the most stable row anchor.
- `tsMs`: wall-clock timestamp in milliseconds.
- `traceMode` (schema ≥2): setting at file creation time — one of `off`, `minimal`, `standard`, `verbose`, `trace`. Missing on legacy files (treat as unknown / pre-settings UI).
- `category`: one of `event`, `decision`, `state`, `snapshot`, `log`.
- `domain`: emitting subsystem, for example `streaming`, `xbxengine`, `trace`, or `data`.
- `event`: event name within the domain.
- `sessionId`: optional session correlation id. Many engine-level logs may still be `null`.
- `payload`: structured event payload.

Primary writer: `src-tauri/src/mods/runtime_trace/service.rs`.

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

## Practical Read Order

1. File open and session bootstrap rows.
2. `state` rows for phase movement.
3. `decision` rows for branch reasoning.
4. `snapshot` rows for capability, transport, and performance context.
5. `log` rows only around suspicious windows.
