# Steady Jitter Gentle Absorption Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-04-steady-jitter-gentle-absorption.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-04-steady-jitter-gentle-absorption.md)
- 本轮已完成 steady 后游玩阶段短时抖动的温和吸收改造，覆盖 owner/source/recovery/session/sink 五层闭环，并补齐对应回归。

## Delivered

- 在 owner 状态机中落地 `DegradedServing`，让已有 clean anchor 的轻度供给抖动继续留在 steady 主路径。
- 恢复 clean-anchor 的有限跨 episode 滞回，减少刚恢复后再次被 `transportAwaitRecoveryAnchor` 立即放大。
- 增强 repair/RTX 归一化鲁棒性，让 repair 流轻微异常时不再一律静默丢弃。

## Changes

- [`video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs) 增加 `DegradedServing` 的迁移、health 映射与 clean-anchor 短窗滞回。
- [`session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs)、[`policy/scheduling.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/scheduling.rs)、[`diagnostics/stats.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/diagnostics/stats.rs) 把 `DegradedServing` 作为 steady-serving 的同语义变体接入 liveness/BWE/diagnostics。
- [`recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 为 clean anchor 增加上一 epoch 的短时复用窗口，降低恢复斜坡的过敏感度。
- [`packet_router.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/packet_router.rs) 与 [`sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/sink.rs) 补充 primary video PT 识别、唯一主 PT 回退与 repair route primary 直通策略。

## Validation

- `cargo test -p xbxengine transport::rtc::policy::video_scheduling_owner -- --nocapture`
- `cargo test -p xbxengine transport::rtc::stream::video_source::source -- --nocapture`
- `cargo test -p xbxengine transport::rtc::stream::video_source::sink -- --nocapture`
- `cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`
- `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
- `cargo test -p xbxengine diagnostics::stats -- --nocapture`
- `cargo test -p xbxengine transport::rtc::policy::scheduling -- --nocapture`
- `cargo check -p xbxengine`

## Risks

- `DegradedServing` 当前仍把 `video_health` 投影为健康 steady，主要依赖 `owner_state/reason` 暴露轻退化语义；如果前端后续需要单独显示“温和吸收中”，还需要再细化 UI 映射。
- repair 路径目前只做“RTX 解包 + primary 直通 + unsupported repair 丢弃”的保守增强，尚未引入独立 repair quarantine 队列或更细的 repair telemetry。

## Follow-up

- 用新的真实 runtime trace 复核 `degraded-serving` 与 `transportAwaitRecoveryAnchor` 的时间关系，确认 steady 后短抖动确实不再快速升级为 reconnect。
- 如果后续 trace 仍显示 repair 流量与主视频推进存在口径偏差，再继续把 repair/RTX 从“直接归一化”升级为“带 provenance 的显式 reinject 队列”。
