# 本地 Decoder Reset 与 Transport Epoch 解耦 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 近期 recovery trace 反复出现 `displaySupplyDegraded -> requestDecoderReset -> transport_recovery_epoch++` 的恢复风暴。
- 当前实现把 `RequestDecoderReset` 统一建模为“成功即推进 transport recovery epoch”，导致本地 display/decode-render 修复链与 transport 恢复轮次耦合。

## Goal

- 将本地 display/decode-render 修复链与 transport recovery epoch 拆层。
- 保留 transport/anchor 类恢复动作的既有升级语义，同时避免本地 decoder reset 成功后重置 transport recovery suppression/budget 窗口。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs`
  - `crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs`
  - 对应 recovery / transport session 测试
- Out of scope:
  - 重写 `VideoEscalationController` 的动作升级图
  - 调整 `displaySupplyDegraded` 到 `AdapterThinStream` 的 owner reason 映射
  - 变更 trace schema 或前端展示合同

## Plan

1. 把 recovery action 的 owner/budget contract 与“成功是否推进 transport epoch”分离。
2. 引入 reason-aware 的 epoch 推进规则，让本地 decoder reset 路径停留在当前 transport epoch。
3. 补充定点测试并验证 display/transport 两条主链仍符合预期。

## Validation

- [x] `cargo fmt --all`
- [x] `cargo test -p xbxengine epoch_advance_rule_is_reason_aware_for_local_decoder_reset_paths -- --nocapture`
- [x] `cargo test -p xbxengine transport_session_maps_local_decoder_reset_reason_to_non_advancing_epoch_policy -- --nocapture`
- [x] `cargo test -p xbxengine reconnect_advances_recovery_epoch_by_contract -- --nocapture`
- [x] `cargo test -p xbxengine connected_track_attached_without_first_frame_feedback_does_not_escalate_display_supply_degraded_during_priming_window -- --nocapture`
- [x] `cargo test -p xbxengine connected_track_attached_without_first_frame_feedback_eventually_escalates_transport_await_after_priming_window -- --nocapture`
- [x] `cargo test -p xbxengine local_idle_timeout_repeat_suppression_ignores_transport_epoch_rotation -- --nocapture`
- [x] `cargo test -p xbxengine local_thin_stream_repeat_suppression_coalesces_keyframe_chain_without_transport_epoch_dependency -- --nocapture`
- [x] `cargo test -p xbxengine trace_1775319678083_short_adapter_idle_timeout_burst_stays_in_decoder_reset_stage -- --nocapture`

## Risks

- 当前只拆了 epoch 推进语义，`displaySupplyDegraded -> AdapterThinStream` 的升级图和 budget 仍沿用现状，后续 trace 若仍有风暴，下一步应继续拆 local lane 的 budget/state。
- `from_recovery_reason_label()` 采用显式标签映射，后续若新增本地 decoder reset reason 但未接入该映射，会回退到默认 transport 语义。

## Progress

- [x] Step 1: action contract 收缩为 owner/budget 语义，不再静态声明 epoch 推进。
- [x] Step 2: transport session 成功落账改为按 reason-aware helper 决定是否推进 transport epoch。
- [x] Step 3: `coordinator/repeat_suppression` 已把 `AdapterThinStream/AdapterIdleTimeout/Reconfigure/DecoderBackendFailure` 归入 local lane 语义，local repeat suppression 不再依赖 transport epoch。
- [x] Step 4: 已补充 recovery/transport session/policy 定点验证并完成回归。
- [x] Step 5: `transportAwaitRecoveryAnchor` 现要求先拿到“keyframe episode 明确失败”或“decoded 但仍无 usable clean anchor / anchor candidate rejected”证据，才允许升级到 decoder reset；未命中证据时停留在 keyframe stage，不再仅凭 streak + stall evidence 周期性重打 reset。
- [x] Step 6: `WaitKeyframe` 也与 `transportAwait` 拆出同类 failure-evidence gate；同时 `repeat_suppression` 对同链路 decoder reset in-flight 的吸收窗口对齐 transport session 的 600ms coalescing 窗口，避免 `sameFamilyCoalesced:decoderResetInFlight` 在坏窗里继续刷高频 ledger/budget。
- [x] Step 7: `requested && sent_at_ms == null` 从长期“假 in-flight”语义中拆出短暂 unsent grace；grace 超时后将 episode 落为 terminal `expired-unsent`，并允许后续 `waitKeyframe` 重新触发真实发送尝试。
- [x] Step 8: transport session 对 keyframe command 的 `deferred/failed` 补齐 episode 级 telemetry，避免 trace 里只看到 `requested` 而看不到“其实没发出去/已失败”的终态。

## Execution Notes

- Date: 2026-04-06 | Status: completed
- Update: 新增 `VideoEscalationReason::from_recovery_reason_label()` 与 `action_success_advances_transport_recovery_epoch()`，将 `displaySupplyDegraded/displaySupplyCritical/adapterIdleTimeout/reconfigure/decoderBackendFailure` 等本地 decoder reset 原因从 transport epoch 推进中剥离。
- Decision: 不在 `RuntimeStatsSink` 里做字符串特判，而是在 recovery owner 层提供 reason-aware outcome policy，transport_session 只负责桥接 label 到 reason。
- Risk/Blocker: 尚未继续拆本地 lane 的独立 budget/state machine；如果 trace 仍显示 local recovery 频率异常，需要下一轮继续切分。
- Date: 2026-04-06 | Status: completed
- Update: 继续将 `AdapterThinStream/AdapterIdleTimeout/Reconfigure/DecoderBackendFailure` 从 transport-style 节拍器中拆出：`classify_signal_domain()` 现将其归入 `Local`，`repeat_suppression` 对 local lane 改为基于本地 in-flight/time window 抑制，不再依赖 `transport_recovery_epoch` 轮转。
- Decision: 第二阶段先拆 “epoch/suppression 依赖”，暂不同时修改 budget ledger 合同；否则会牵动 runtime protocol / diagnostics 展示，超出本轮最小闭环。
- Risk/Blocker: local lane 仍复用部分共享 budget 计数；如果新 trace 里仍有 reset 风暴，再继续拆 local repair budget。
- Date: 2026-04-06 | Status: completed
- Update: 基于 [`runtime-trace-1775475953892-1.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775475953892-1.jsonl) 继续收口 `transportAwaitRecoveryAnchor`：`coordinator` 新增 failure-evidence gate，仅当最近 `keyframeRequestEpisode` 为 `missed/late`、`decoded` 后经过短宽限仍无 clean anchor，或 anchor candidate 被 `awaitingRecoveryKeyframe/referenceChainUnrecoverable/bootstrap reject` 一类失败原因拒绝时，才放开 transport-await 到 decoder reset 的 stage escalation；`VideoEscalationController` 也新增显式门禁，避免仅凭 persistent transport-await 定时升级到 decoder reset / reconnect。
- Decision: 这轮不把 `recent RTCP unavailable / thin stall` 单独视为足以升级 reset 的硬失败证据，而是要求更直接的“keyframe 无法形成 usable clean anchor”证据，优先先把 trace 里的周期性 reset cadence 打掉。
- Risk/Blocker: `keyframeRequestEpisode` 目前仍只有 latest episode 级别的观测，后续若出现 episode 归因污染，可能继续影响 evidence gate 的精度；必要时再单独修 episode 关联。
- Date: 2026-04-06 | Status: completed
- Update: 继续基于同一 trace 收口 `ingressWaitKeyframe` 与 decoder-reset burst：`coordinator` 现在对 `WaitKeyframe` 复用同类 keyframe failure evidence gate，未命中 `missed/late`、`decoded-but-unusable`、anchor candidate `Rejected` 等硬证据前，不再让 `persistent_wait_keyframe` 自动升级到 decoder reset；[`repeat_suppression.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/repeat_suppression.rs) 同时把 `WaitKeyframe` 链路的 decoder-reset in-flight 吸收窗口扩到 620ms，以对齐 transport session 600ms `sameFamilyCoalesced:decoderResetInFlight` 语义。
- Decision: 第二阶段不推翻“提案即记账”的 budget 合同，而是优先在 coordinator/controller 前置吸收近期 decoder reset in-flight，先把 trace 里 458ms 内 26 次 deferred reset 的假高频压掉；若后续 trace 仍显示 budget 被 defer 结果污染，再单独拆 controller 成功落账语义。
- Risk/Blocker: `decoder_reset_budget_used` 仍然是 proposal-based contract，虽然本轮已显著减少 waitKeyframe/transportAwait 的误触发，但若后续还有其它 reset reason 在 transport 层频繁 defer，同样可能需要进一步把 controller budget 改成 success-aware。
- Date: 2026-04-06 | Status: completed
- Update: 基于 [`runtime-trace-1775478427122-1.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775478427122-1.jsonl) 继续收口“requested but unsent”自锁链：[`coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 为 `requested && sent_at_ms == null` 增加 220ms 的 unsent grace，超窗后不再把它视为 `keyframeInFlight`，而是终结为 `expired-unsent`；[`transport_session.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs) 现在会把 keyframe command 的 `deferred/failed` 显式回写为 episode 终态；[`runtime_stats_sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/runtime_stats_sink.rs) 新增 `keyframeRequestEpisodeDeferred/Failed/UnsentExpired` 观测，避免 trace 再出现“只有 requested、没有 sent、却被上层长期当成 in-flight”的假挂单。
- Decision: 这一轮优先修正“episode 终态缺失 + in-flight 语义错位”，不把 transport queue/replay 再单独拆成新状态机；先确保只有真正 `sent` 的请求才参与 in-flight/coalescing，未发送请求只在极短 priming 窗口内保留。
- Risk/Blocker: `waitKeyframe` 的媒体债务仍然依赖 clean keyframe 才能真正清掉；如果后续 trace 仍显示 request 在 transport/control 侧长期 queue 而非 sent，下一轮需要继续把 `queued-for-replay` 单独建模成可观测 episode 状态。
