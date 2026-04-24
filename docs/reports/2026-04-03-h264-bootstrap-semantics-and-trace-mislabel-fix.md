# H264 Bootstrap Semantics and Trace Mislabel Fix Report

## Summary

- Related RFC: [`docs/rfcs/2026-04-03-h264-bootstrap-semantics-and-trace-mislabel-fix.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-03-h264-bootstrap-semantics-and-trace-mislabel-fix.md)
- 本轮已完成 `MissingSps` bootstrap 语义修正、delta slice 承接/解析观测增强，以及 `h264InspectionRejected` 的误导性收口。

## Delivered

- 将 `MissingSps / MissingPps` 语义收敛为 bootstrap 缺口，不再暗示 delta slice 解析失败。
- 为 H264 inspection 增加 `delta_continuation_ready` 与 `admission_accepted`，让 trace 可以区分“当前 AU bootstrap 不完整”和“实际 admission 拒绝”。
- 将 trace 事件名按实际 admission 结果拆分为 `h264InspectionObserved` / `h264InspectionRejected`，避免 bootstrap 缺口被误读成真实拒绝。

## Changes

- `crates/xbxengine/core/src/media/video/h264/inspection.rs`：补充 delta continuation 判定，收紧 bootstrap 枚举命名。
- `crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs`：把 inspection bootstrap 语义和 admission 语义拆开，并投到 runtime stats。
- `crates/xbxengine/core/src/api/backend.rs`、`crates/xbxengine/protocol/src/runtime.rs`、`src/shared/rpc/xbxengine.ts`、`crates/xbxengine/core/src/diagnostics/stats.rs`：贯通新字段到 DTO / 统计层。
- `src-tauri/src/mods/xbxengine/trace_projection.rs`：让 trace 事件名仅在真实 admission 拒绝时使用 `h264InspectionRejected`，并补回归用例。

## Validation

- `cargo fmt --all`
- `cargo check -p xbxengine`
- `cargo test -p xbxengine media::video::h264::inspection -- --nocapture`
- `cargo test -p xbxengine media::video::ingress::scheduler -- --nocapture`
- `cargo test -p xbxengine media::video::decode::video_decode -- --nocapture`
- `cargo test -p xbxrc trace_projection -- --nocapture --test-threads=1`
- `pnpm exec vue-tsc --noEmit --pretty false`

## Risks

- 既有 trace 分析脚本若仍按旧的 `MissingSps` / `h264InspectionRejected` 口径聚合，需要同步更新。
- 后续若继续扩展 H264 bootstrap 语义，还要保持 admission 与观测字段分层，避免再次混用。

## Follow-up

- 实机 trace 里继续观察 `bootstrapMissingSps`、`admissionAccepted`、`deltaContinuationReady` 的组合分布。
- 若后续发现新的 bootstrap 缺口类型，再按同一模式补充结构化语义。
