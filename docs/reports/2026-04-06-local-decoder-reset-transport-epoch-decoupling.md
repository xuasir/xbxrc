# 本地 Decoder Reset 与 Transport Epoch 解耦 Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-06-local-decoder-reset-transport-epoch-decoupling.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-06-local-decoder-reset-transport-epoch-decoupling.md)
- 已完成将本地 display/decode-render 修复链与 `transport_recovery_epoch` 推进语义拆开，避免 `displaySupplyDegraded` 一类本地 decoder reset 成功后开启新 transport 恢复轮次。

## Delivered

- recovery owner 层新增 reason-aware 的 epoch 推进策略。
- transport session 改为按 recovery reason 决定 decoder reset 成功后是否推进 transport epoch。
- 补齐 recovery / transport session / policy 定点测试，锁住本地 lane 与 transport lane 的分层语义。
- `coordinator/repeat_suppression` 已将本地 repair lane 的 suppression 语义从 `transport_recovery_epoch` 脱钩。
- `transportAwaitRecoveryAnchor` 新增 evidence gate：必须先观察到 keyframe episode 的明确失败，或“keyframe 已 decoded 但仍未形成 usable clean anchor / anchor candidate rejected”的失败证据，才允许升级到 decoder reset。
- `WaitKeyframe` 也改为 evidence-gated stage escalation；未命中 `missed/late`、`decoded-but-unusable`、anchor candidate `Rejected` 等证据时，停留在 keyframe lane，不再按固定 cadence 自动升级 reset。
- `repeat_suppression` 对 `WaitKeyframe` 同链路的 decoder reset in-flight 吸收窗口已对齐 transport session 的 600ms family coalescing，避免 `sameFamilyCoalesced:decoderResetInFlight` 在坏窗里继续刷高频 ledger。
- `requested && sent_at_ms == null` 已从长期“假 in-flight”语义中拆出为短暂 unsent grace；grace 过后会终结为 `expired-unsent`，不再把上层锁在 `coalesced:keyframeInFlight`。
- keyframe command 的 `deferred/failed` 已补齐 episode 级 telemetry，trace 现可直接区分“未真正发送”“发送失败”“已 sent 等响应”。

## Changes

- [`escalation.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs) 新增 `VideoEscalationReason::from_recovery_reason_label()` 与 `action_success_advances_transport_recovery_epoch()`，并将 `RecoveryActionContract` 收束为 owner/budget contract。
- [`transport_session.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs) 不再按静态 contract 推进 epoch，而是通过 reason-aware helper 把 `displaySupplyDegraded` 等本地 decoder reset 保持在当前 epoch。
- [`escalation.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/escalation.test.rs) 与 [`transport_session.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs) 测试新增本地 lane 不推进 epoch 的回归覆盖。
- [`coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 已将 `AdapterThinStream/AdapterIdleTimeout/Reconfigure/DecoderBackendFailure` 归入 `Local` signal domain；[`repeat_suppression.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/repeat_suppression.rs) 对 local lane 改为纯本地 time-window/in-flight coalescing，不再以 transport epoch 轮转作为放行条件。
- [`coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 继续收紧 `transportAwaitRecoveryAnchor`：只有在 `keyframeRequestEpisode` 为 `missed/late`、`decoded` 后短宽限内仍 unresolved 且无 recent clean anchor、或 anchor candidate 被 `awaitingRecoveryKeyframe/referenceChainUnrecoverable/bootstrap reject` 等失败原因拒绝时，才允许 stage upgrade 到 decoder reset。
- [`escalation.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs) 新增 `allow_transport_await_stage_escalation` 门禁，禁止 controller 仅凭 `persistent_transport_await_recovery_keyframe` 自动把 transport-await 推到 decoder reset / reconnect。
- [`coordinator.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.test.rs) 新增/更新回归：无失败证据时继续停留在 keyframe stage；`missed`、`decoded 但仍无 clean anchor`、anchor candidate `Rejected` 时才升级到 decoder reset。
- [`coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 现对 `WaitKeyframe` 复用同类 keyframe failure evidence gate，并把 gate 结果传入 [`escalation.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs) 的 `allow_wait_keyframe_stage_escalation`，从 controller 侧切断 `persistent_wait_keyframe -> RequestDecoderReset` 的固定节拍升级。
- [`repeat_suppression.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/repeat_suppression.rs) 对 `WaitKeyframe/transportAwaitRecoveryKeyframe` 链路新增更长的 decoder-reset in-flight 吸收窗，优先把最近一次 `requestDecoderReset` 视为 `CoalescedDecoderResetInFlight`，避免 transport session 已 defer 时上层继续刷 ledger/budget。
- [`escalation.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/escalation.test.rs) 与 [`coordinator.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.test.rs) 新增 `WaitKeyframe` 反例/正例和 recent decoder-reset repeat suppression 回归。
- [`coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 新增 220ms unsent grace：`requested && sent_at_ms == null` 只在该窗口内作为短暂 priming 吸收，超窗后立即终结为 `expired-unsent`，释放 keyframe in-flight 语义。
- [`transport_session.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs) 现在会在 keyframe command `deferred/failed` 时同步把 `latest_keyframe_request_episode` 标记为 `deferred/failed`，不再让 trace 停留在长期 `requested`。
- [`runtime_stats_sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/runtime_stats_sink.rs) 新增 `keyframeRequestEpisodeDeferred`、`keyframeRequestEpisodeFailed`、`keyframeRequestEpisodeUnsentExpired` 三类观测及测试，补齐 unsent/failed 路径的 episode 闭环。

## Validation

- `cargo fmt --all`
- `cargo test -p xbxengine epoch_advance_rule_is_reason_aware_for_local_decoder_reset_paths -- --nocapture`
- `cargo test -p xbxengine transport_session_maps_local_decoder_reset_reason_to_non_advancing_epoch_policy -- --nocapture`
- `cargo test -p xbxengine reconnect_advances_recovery_epoch_by_contract -- --nocapture`
- `cargo test -p xbxengine connected_track_attached_without_first_frame_feedback_does_not_escalate_display_supply_degraded_during_priming_window -- --nocapture`
- `cargo test -p xbxengine connected_track_attached_without_first_frame_feedback_eventually_escalates_transport_await_after_priming_window -- --nocapture`
- `cargo test -p xbxengine local_idle_timeout_repeat_suppression_ignores_transport_epoch_rotation -- --nocapture`
- `cargo test -p xbxengine local_thin_stream_repeat_suppression_coalesces_keyframe_chain_without_transport_epoch_dependency -- --nocapture`
- `cargo test -p xbxengine trace_1775319678083_short_adapter_idle_timeout_burst_stays_in_decoder_reset_stage -- --nocapture`
- `cargo test -p xbxengine unsent_requested_keyframe_is_rolled_back_before_transport_await_stage_upgrade -- --nocapture`
- `cargo test -p xbxengine sent_pending_keyframe_with_thin_stall_pressure_upgrades_to_decoder_reset -- --nocapture`
- `cargo test -p xbxengine decoded_transport_await_keyframe_without_clean_anchor_upgrades_to_decoder_reset -- --nocapture`
- `cargo test -p xbxengine coordinator_staged_recovery_handles_sparse_transport_await_signals -- --nocapture`
- `cargo test -p xbxengine sent_pending_keyframe_with_recent_rtcp_unavailable_does_not_upgrade_to_decoder_reset -- --nocapture`
- `cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`
- `cargo test -p xbxengine persistent_wait_keyframe_without_failure_evidence_does_not_escalate_to_decoder_reset -- --nocapture`
- `cargo test -p xbxengine recent_wait_keyframe_decoder_reset_suppresses_repeat_wait_keyframe_reset -- --nocapture`
- `cargo test -p xbxengine wait_keyframe_without_failure_evidence_does_not_upgrade_to_decoder_reset -- --nocapture`
- `cargo test -p xbxengine rejected_wait_keyframe_anchor_candidate_upgrades_to_decoder_reset -- --nocapture`
- `cargo test -p xbxengine transport::rtc::recovery::escalation -- --nocapture`
- `cargo test -p xbxengine runtime_stats_sink -- --nocapture`
- `cargo test -p xbxengine transport::rtc::stack::transport_session -- --nocapture`
- `cargo check -p xbxengine`

## Risks

- 目前已解决 epoch 与 suppression 两个最关键维度的耦合，但本地 lane 仍共享部分 recovery budget；后续若 trace 仍显示 reset 频率异常，需要继续拆 local repair budget。
- recovery reason label 到 enum 的映射需要随着新 reason 同步维护，否则未识别标签会回退到默认 transport 语义。
- `keyframeRequestEpisode` 仍以 latest episode 为主，若后续 trace 再出现 episode 被更晚 decode 改写的现象，需要单独修 episode 归因，否则 evidence gate 可能被污染。
- controller 侧 budget 仍是 proposal-based contract；本轮通过前置 gate/coalescing 已显著减少 `WaitKeyframe` 坏窗里的假高频，但若后续仍有其它 reset reason 在 transport 层频繁 defer，可能还需要继续拆 success-aware budget。
- 当前 `unsent-expired` 已能打断“requested 但未发送”的假在飞语义，但 queue-for-replay 仍未单独建模；若后续 trace 继续显示 control replay 长时间未消费，需要继续把 `queued-for-replay` 提成 episode 级状态。

## Follow-up

- 用新的 runtime trace 复核 `displaySupplyDegraded` 是否已不再周期性打穿 repeat suppression。
- 如果 local decoder reset 仍过频，继续把 `AdapterThinStream/DisplaySupplyCritical` 从 transport-style budget/state machine 中拆成独立 local repair lane。
