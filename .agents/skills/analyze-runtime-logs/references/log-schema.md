# Runtime Trace Schema

## Envelope

Each trace row is one JSON object in `jsonl` format.

Stable top-level fields:

- `schemaVersion`: current schema version. Treat version drift as a parsing risk.
- `seq`: per-file monotonic sequence number. Use this as the most stable row anchor.
- `tsMs`: wall-clock timestamp in milliseconds.
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

## Practical Read Order

1. File open and session bootstrap rows.
2. `state` rows for phase movement.
3. `decision` rows for branch reasoning.
4. `snapshot` rows for capability, transport, and performance context.
5. `log` rows only around suspicious windows.
