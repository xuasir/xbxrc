# Video Source Backpressure Priority And Recovery Softening Report

## Summary

- Related RFC: [`docs/rfcs/2026-04-05-video-source-backpressure-priority-and-recovery-softening.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-05-video-source-backpressure-priority-and-recovery-softening.md)
- 本轮完成了 `video_source` 的 ingress 分级背压改造，收口了“steady 健康链路在开始游玩后因为 sink 满队列先丢本地包、再被放大成恢复风暴”的主问题链。

## Delivered

- 在 [`sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/sink.rs) 落地 priority / best-effort 分级背压与有界本地 backlog。
- 让 repair passthrough、RTX reinject 以及恢复优先 H264 primary 不再与普通 delta 一起被统一 `try_send` 挤掉。
- 为本地背压丢弃补充显式 `video_frame_drop` 观测，并补齐 `sink` 定向回归测试。
- 基于 [`runtime-trace-1775345271853.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775345271853.jsonl) 追加在 [`nack.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs) 与 [`timeline.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.rs) 增加 `displayStarvedLowValueAdmission`，把显示链已 `critical starved` 时的小型 delta gap 直接降级为软缺口。
- 第三阶段继续在 [`session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs)、[`recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs)、[`recovery/nack_outcome.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/nack_outcome.rs)、[`api/runtime/lifecycle.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/runtime/lifecycle.rs) 与 [`api/runtime/mod.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/runtime/mod.rs) 完成 recovery domain gate，把 `displaySupplyCritical` 明确收口为本地域问题，禁止它被洗成 transport reconnect。
- 收尾阶段进一步把 `reason_domain` 强类型透传补齐到测试与验证层：runtime gate 现在按 `Local / ConnectivityTransport / Unknown` 结构化字段验收 pending reconnect candidate，`planner/scheduling/transport_session/runtime` 的回归都覆盖了 domain 保留与放行/拒绝边界。

## Changes

- `RtcVideoSourceSink` 新增 `new(...)` 构造与本地 pending 队列，发送路径改为先 `flush_pending()`，再按优先级选择直接投递或本地缓存。
- `BestEffort` 流量只保留最新一个 pending 包，旧包被替换时记录 `localBackpressureBestEffortReplaced`；`Priority` 流量进入有界 backlog，溢出时记录 `localBackpressurePriorityOverflow`。
- `source/timeline` 未再追加新补丁，因为现有 `localBackpressureDeltaGap` / `hard_recovery_gap_risk` 软化路径已经覆盖“本地背压低价值缺口不应直接升级为强恢复”的设计目标，并已通过回归测试确认持续成立。
- 第二阶段收口把“显示链已长期 starved 时的小 gap”纳入同一套低价值缺口语义：`nack` admission 现在会结合 `host_no_pending_pressure_level=critical`、`host_no_pending_streak` 与 `latest_video_host_present_time_ms` 识别 `displayStarvedLowValueAdmission`，阻止这类 gap 再进入 `gapRepairInFlight -> awaitingRecoveryKeyframe`。
- 第三阶段把 `displaySupplyCritical` 从 `AdapterIdleTimeout` 的恢复语义里拆开：owner/supply 仍然保留原始 label，但 session policy 改为映射到 `DisplaySupplyCritical` 独立本地域 reason；coordinator 内部把它视为 `Local` 域；`nack_outcome` 遇到重要帧过期时不再把它改写成 `TransportAwaitRecoveryKeyframe`；runtime lifecycle 在最终消费 pending reconnect candidate 前再做一层 domain gate，把 `displaySupplyCritical`、`localBackpressureDeltaGap`、`stack.manualRequest` 等本地 reason 全部拒绝在 reconnect 之外。
- 第四阶段补齐了结构化透传合同的测试闭环：`planner` 和 `scheduling` 确认 reconnect command 继续携带 `reason_domain`，`transport_session` 确认 staged pending action 保留 `reason_domain`，`runtime` 确认 `displaySupplyCritical` 被拒绝、`peer-closed / livenessNoProgressTimeout / transportExpiredDeadline` 被放行。

## Validation

- `cargo test -p xbxengine transport::rtc::stream::video_source::sink -- --nocapture`
- `cargo test -p xbxengine transport::rtc::stream::video_source::source -- --nocapture`
- `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
- `cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`
- `cargo test -p xbxengine transport::rtc::stream::video_source::timeline -- --nocapture`
- `cargo test -p xbxengine transport::rtc::stream::video_source::nack -- --nocapture`
- `cargo test -p xbxengine api::runtime::tests -- --nocapture`
- `cargo test -p xbxengine transport::rtc::stack::transport_session -- --nocapture`
- `cargo check -p xbxengine`

## Risks

- H264 恢复优先识别当前基于 NAL 类型做启发式判定，后续如果服务端封包型态出现新变种，还需要结合新 trace 校正判定口径。
- 本轮已经把“本地 ingress/显示供给抖动被误升级到 reconnect”的主链切断，且 `reason_domain` 已从 planner/scheduling 贯通到 runtime gate；残余风险转为“未来新增 reconnect reason 时，必须同步补 domain 赋值与回归测试”，否则仍可能在上游产生错误域别。

## Follow-up

- 用下一份真实 runtime trace 回看 steady 建连后开始游玩阶段，确认不再先出现 `video source sink ingress dropped err=no available capacity` 后立即升级成 `awaitingRecoveryKeyframe / reconnect`。
- 用下一份真实 runtime trace 回看显示链长时间 `displaySupplyCritical` 时，小 delta gap 是否已经只停留在局部软缺口，不再被写成 `gapRepairInFlight -> awaitingRecoveryKeyframe -> reconnect`。
- 用下一份真实 runtime trace 回看 `displaySupplyCritical` 连续抖动时，是否已经稳定停留在 keyframe/decoder reset 这类本地恢复动作，不再由 pending reconnect candidate 落成重连。
- 如果后续 trace 仍有局部恢复抖动，再评估是否需要把 H264 恢复优先判定从启发式提升为更明确的帧级语义。
