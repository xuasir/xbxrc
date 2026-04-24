# Moonlight 帧价值模型落地 Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-03-moonlight-frame-value-model-convergence.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-03-moonlight-frame-value-model-convergence.md)
- 本轮已将 Moonlight 的帧价值思路落成可执行的两层预算模型，并完成 ingress / NACK / recovery / trace / frontend 的统一贯通

## Delivered

- 建立了统一的 [`FrameBudgetContext`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/ingress/budget.rs) 预算合同，保留 [`FrameValue`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/types.rs) 作为帧本身的内在价值
- 让 [`source.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs)、[`scheduler.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/ingress/scheduler.rs)、[`nack_scheduler.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/nack_scheduler.rs)、[`nack_policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/nack_policy.rs)、[`video_decode.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/video_decode.rs) 与 [`pacer/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/pacer/actor.rs) 共享同一份预算上下文
- 将恢复/观测链路补齐到 [`runtime_state.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/runtime_state.rs)、[`coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs)、[`stats.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/diagnostics/stats.rs) 与 [`trace_projection.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/xbxengine/trace_projection.rs)

## Changes

- 在 ingress 侧把帧预算从固定 `min/max delay` 提升为显式上下文驱动的准入与回退逻辑
- 在 NACK / recovery 侧把 `chain_broken`、`wait_keyframe`、`reconfigure`、`repair_priority` 与 RTT 余量统一纳入一个预算合同
- 在 stats / trace / frontend 中增加了可直接解释“为什么救、为什么丢、为什么升级”的字段投影

## Validation

- `cargo fmt --all`
- `cargo check -p xbxengine`
- `cargo test -p xbxengine media::video::ingress::budget -- --nocapture`
- `cargo test -p xbxengine media::video::ingress::scheduler -- --nocapture`
- `cargo test -p xbxengine transport::rtc::stream::video_source::nack_policy -- --nocapture`
- `cargo test -p xbxengine media::video::decode::video_decode -- --nocapture`
- `cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`
- `cargo test -p xbxengine transport::rtc::stream::nack_scheduler -- --nocapture`
- `pnpm exec vue-tsc --noEmit --pretty false`

## Risks

- 当前仍保留少量与本任务无关的 dead-code warnings，后续若持续演进预算合同，可能需要再收一轮 helper/测试残余
- 预算模型已经统一接线，但真实实机 trace 下的阈值仍需持续回放校准，避免过于保守或过于激进

## Follow-up

- 用下一份真实 runtime trace 继续验证 `FrameBudgetContext` 的决策解释力
- 如需要，再继续把部分 helper 收紧为更小的内部模块，减少 dead-code 噪音
