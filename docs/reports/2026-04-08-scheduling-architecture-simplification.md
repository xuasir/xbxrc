# 调度层架构简化与能力保留 Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-08-scheduling-architecture-simplification.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-08-scheduling-architecture-simplification.md)
- 本任务已完成：在不回退首帧获取优先、昂贵恢复升级面收缩、恢复爬升期保护三类能力的前提下，重构调度层主线，把顶层控制模型收敛为 `Facts -> FirstFrameAcquisitionPriority -> DomainClassifier -> LocalRecoveryCoordinator -> ExpensiveRecoveryGate -> RecoveryRampGuard -> Planner / Reporting`。

## Delivered

- 新增 `facts / startup_compat / expensive_recovery_gate / recovery_ramp_guard / connectivity_reason` 五个 session 子模块，把事实提取、首帧获取优先、昂贵恢复 gate、恢复爬升保护与 connectivity 映射从 [`session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 中拆出。
- 收窄 [`video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs) 对外契约，形成“结构化控制输出 + diagnostics surface”分层，字符串 reason label 不再主导控制面。
- 收窄 [`recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 的 transport-await 暴露面，将外部多布尔探测口统一成 staged local recovery surface。
- 修正 `video_source` 最后两条回归合同，使 `clean anchor` 与 ramp-up 语义与当前恢复模型一致。
- 参考 `moonlight-qt` 补上全链路动作边界收口：码流/decoder/显示供应等本地域问题只能停留在 `Wait / RequestKeyframe / RequestDecoderReset`，[`session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 只允许连接域 reason 进入 `RequestReconnectCandidate`，[`recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 的 `TransportAwaitRecoveryKeyframe` hard fallback 也不再上抬成 reconnect。
- 继续参考 `moonlight-qt` 的防风暴边界，在 [`source.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs) 抬高 `sample loss` 进入 `WaitKeyframe` 的门槛，并在 [`escalation.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs) 移除 `AdapterThinStream / AdapterIdleTimeout / TransportSampleLoss` 直接推动 `RequestDecoderReset` 的短链升级，让小抖动默认停留在 `drop / wait / request keyframe`。
- 进一步在 [`source.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs) 为 `idle timeout / thin stream stall` 增加 source 侧短确认窗：第一次命中先本地记账，只有短窗后仍持续满足条件才上报 observation；收到新包或样本完成会立即清空待确认状态。
- 继续补上“首帧获取优先”的最后一层动作边界：[`source.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs) 现明确禁止 pre-first-frame 阶段仅凭 `deltaContinuationReady` 放行 non-IDR continuation，且 waiting-keyframe 时也不会再把未建立首帧前的 delta 当成可吸收本地波动；首帧未建立前只能继续 `WaitKeyframe / RequestKeyframe`。
- 补强 keyframe episode 观测而不增加新的宏观概念：[`runtime_stats_sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/runtime_stats_sink.rs) 新增 `response-observed` 状态，复用现有 `first_video_packet* / first_keyframe_packet* / status_detail` 记录“首个响应 non-keyframe”或“首个 keyframe 仍不可用”的原因；[`trace_projection.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/xbxengine/trace_projection.rs) 同步新增 `keyframeRequestEpisodeResponseObserved` trace 事件。

## Changes

- [`session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 现在更接近 orchestration 层：负责拼接 Facts、owner、local recovery、expensive gate 与 planner，不再回头解释局部媒体细节。
- [`video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs) 引入 `OwnerRecoveryReason` 与 `VideoSchedulingOwnerDiagnostics`，把控制字段与 trace/runtime-stats 字段切开。
- [`recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 将 transport-await lane 降级为内部子状态机，外层只消费 recovery stage，而不是 probe/decode/reset 三套并列判据。
- [`recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 现在对 `TransportAwaitRecoveryKeyframe` 强制执行本地恢复边界：即便 hard fallback 超时，也只回到 `RequestDecoderReset / CoalescedDecoderResetInFlight`，不再产出 reconnect candidate。
- [`session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 新增“昂贵恢复 reason 白名单”收口，只有连接域 reason 才能保留 `RequestReconnectCandidate`；媒体域 reconnect proposal 会被统一压回 `CooldownSuppressed` 并记录 `reconnectBlocked:localDomainOwnsRecovery`。
- [`runtime_stats_sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/runtime_stats_sink.rs) 与 [`source.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.test.rs) 对齐为同一合同：`clean anchor` 记录当前 recovery epoch，但只有 stable settle 才会关闭 recovery episode。
- [`source.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs) 现将 `sample loss` 的入口吸收层改为更保守的历史窗口：至少连续 3 次坏窗才从 `DropAndRequestKeyframe` 进入 `TriggerWaitKeyframe`，并要求更多 clean sample 才清空 burst。
- [`source.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs) 现为 `idle timeout / thin stream stall` 加入短 confirmation window，避免单次尖峰或短抖动直接变成 transport observation。
- [`escalation.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs) 现明确禁止 `AdapterThinStream / AdapterIdleTimeout / TransportSampleLoss` 仅凭“keyframe 后短窗再次出现”就直接升级到 `RequestDecoderReset`；decoder reset 只保留给 `persistent WaitKeyframe / persistent TransportAwaitRecoveryKeyframe / Reconfigure / DecoderBackendFailure` 等更硬证据路径。
- [`source.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs) 现把 `resolve_inspection_admission()` 与 `resolve_recovery_keyframe_action()` 都显式绑定到 `first_frame_acquired`，从动作边界上切断“首帧未建立但 delta chain 看起来 healthy”的误判。
- [`runtime_stats_sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/runtime_stats_sink.rs) 与 [`trace_projection.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/xbxengine/trace_projection.rs) 现可直接投影 keyframe request 的首个响应阶段，避免再靠人工串 `keyframe episode + h264 inspection` 两条线猜首帧失败原因。

## Validation

- `cargo fmt --all`
- `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
- `cargo test -p xbxengine transport::rtc::policy::video_scheduling_owner -- --nocapture`
- `cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`
- `cargo test -p xbxengine transport::rtc::recovery::escalation -- --nocapture`
- `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
- `cargo test -p xbxengine transport::rtc::stream::video_source::source -- --nocapture`
- `cargo test -p xbxengine transport::rtc::stream::video_source -- --nocapture`
- `cargo test -p xbxengine runtime_stats_sink -- --nocapture`
- `cargo test -p xbxrc trace_projection -- --nocapture`
- `cargo check -p xbxengine`
- `cargo check -p xbxrc`

## Risks

- 当前仍存在一些未使用代码与告警，虽不影响本轮架构收口，但后续继续演进时容易掩盖真正的新告警。
- 顶层概念已经收敛，但 `session/policy.rs` 仍然偏大；若未来继续加临时特判而不回收到 gate/facts 子模块，复杂度仍可能回流。

## Follow-up

- 继续观察真实 trace，确认云端慢反馈、无 SPS/无 IDR、重连恢复三类远端画像在新架构下的运行态投影仍稳定。
- 后续若继续瘦身，应优先沿 `facts / gate / reporting` 现有边界拆分，而不是重新引入新的 lane/source/summary 顶层概念。
