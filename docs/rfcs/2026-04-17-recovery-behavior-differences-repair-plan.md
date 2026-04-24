# 恢复系统行为差异修复计划 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: Codex / rtc recovery modules
- Last Updated: 2026-04-17

## Background

- `docs/test-behavior-differences.md` 与 `docs/p1-behavior-differences-detailed.md` 记录了新旧恢复系统在测试层面的行为差异。
- 当前差异表面上分散在 `transport_session`、`session::policy`、`recovery_integration`、`reconnect_lifecycle`、`connection::service`、runtime 集成等多个测试模块，但代码梳理后可归并为少数几个责任断点。
- 新恢复系统已经将门控、coalescing、preempt、epoch 协调主责迁移到 `StateRecoveryCoordinator` / `RecoveryCoordinator` / `RtcSessionPolicy`，而部分测试夹具和部分 `connection::service` 逻辑仍按旧分层语义工作，导致“测试契约失配”和“真实行为缺口”混杂在一起。

## Goal

- 将当前行为差异按代码责任层拆解为一组可执行修复项，而不是逐条按测试名做补丁式修复。
- 先清理分层契约失配造成的假差异，再收敛真实的恢复生命周期、reconnect 升级链、同 tick 仲裁和 service 执行边界问题。
- 修复完成后，恢复系统在以下方面与既定语义重新对齐：
  - coordinator 负责恢复门控与 coalescing 语义
  - policy 负责 episode 生命周期、epoch/clean-anchor/transport-await 收敛
  - reconnect 在 repeated deadline、budget、success-edge、terminal 语义上与旧系统兼容
  - transport/session/service 仅承担执行与回填，不再隐式重建旧状态机

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/session/policy.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/action_coordinator.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/state_coordinator.rs`
  - `crates/xbxengine/core/src/transport/rtc/connection/service.rs`
  - `crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs`
  - `crates/xbxengine/core/src/transport/rtc/policy/recovery.rs`
  - 对应测试夹具与回归测试：
    - `policy_tests/display_owner_ledger.rs`
    - `policy_tests/reconnect_lifecycle.rs`
    - `policy_tests/recovery_integration.rs`
    - `connection/service.test.rs`
    - `stack/transport_session.rs` 内测试
    - `api/runtime/runtime_tests/*` 中受恢复链影响的用例
- Out of scope:
  - 重做恢复系统整体架构
  - 引入第二套恢复状态机或回退到旧实现
  - 与本轮行为差异无关的前端展示、trace 展示或 DTO 重构
  - 非恢复主链的独立流媒体问题

## Plan

1. 清理 `transport_session` 与 ledger 测试夹具的分层契约失配，剥离“假差异”。
2. 补齐 `RtcSessionPolicy` 对 fresh output、clean anchor、transport-await、epoch 切换的恢复生命周期闭环。
3. 修复 repeated transport deadline 到 connectivity reconnect 的升级链，并统一 reconnect 的节流、budget、success-edge、failed terminal 规则。
4. 收紧同 tick 多信号仲裁顺序，确保 transport deadline/lifecycle reconnect 优先于本地 display/transport-await 恢复。
5. 修正 `RtcConnectionService` 的 keyframe stage 逻辑，使其重新服从 policy/coordinator 的 suppress 和 clean-anchor 语义。
6. 运行分层回归测试与 runtime 集成测试，按修复面记录结果并更新本 RFC。

## Validation

- [x] `cargo test -p xbxengine high_no_pending_but_fresh_present_does_not_force_keyframe -- --nocapture`
- [x] `cargo test -p xbxengine request_video_keyframe_clears_stage_after_clean_anchor -- --nocapture`
- [x] `cargo test -p xbxengine request_video_keyframe_does_not_suppress_stale_clean_anchor_when_transport_await_reappears -- --nocapture`
- [x] `cargo test -p xbxengine video_recovery_clean_anchor_clears_stage_token_and_new_epoch_restarts_from_pli -- --nocapture`
- [x] `cargo test -p xbxengine new_recovery_epoch_does_not_bypass_existing_recovery_suppression_chain -- --nocapture`
- [x] `cargo test -p xbxengine test_epoch_rotation_clears_previous_recovery_chain -- --nocapture`
- [x] `cargo test -p xbxengine connected_reconnect_request_does_not_clear_inflight_without_edge -- --nocapture`
- [x] `cargo test -p xbxengine reconnect_budget_snapshots_reflect_before_and_after_state -- --nocapture`
- [x] `cargo test -p xbxengine recovery_integration_repeated_transport_severe_deadline_upgrades_to_connectivity_reconnect -- --nocapture`
- [x] `cargo test -p xbxengine recovery_integration_repeated_transport_expired_deadline_upgrades_to_connectivity_reconnect -- --nocapture`
- [x] `cargo test -p xbxengine recovery_integration_transport_await_exits_after_completion_evidence -- --nocapture`
- [x] `cargo test -p xbxengine recovery_integration_cloud_stale_transport_await_replay_reenters_local_recovery_when_output_stalls -- --nocapture`
- [x] `cargo test -p xbxengine recovery_integration_fresh_transport_await_absorption_expires_once_output_stalls -- --nocapture`
- [x] `cargo test -p xbxengine recovery_intent_is_suppressed_within_same_epoch_via_coordinator_chain -- --nocapture`
- [x] `cargo test -p xbxengine reconnect_command_is_throttled_and_re_emitted_during_continuous_recovering -- --nocapture`
- [x] `cargo test -p xbxengine cloud_lifecycle_reconnect_interval_is_more_relaxed_than_non_cloud -- --nocapture`
- [x] `cargo test -p xbxengine recovery_integration_home_render_deadline_jitter_stays_local_display_path -- --nocapture`
- [x] `cargo test -p xbxengine runtime_home_render_deadline_jitter_replay_stays_local_and_never_reaches_reconnect -- --nocapture`
- [x] `cargo test -p xbxengine --lib --quiet`

## Risks

- `RtcSessionPolicy` 当前同时承担 owner orchestration、recovery proposal、ledger 记录与 reconnect lifecycle，多处语义互相耦合，局部修复可能导致另一组测试回退。
- `connection::service` 的简化逻辑已经默认“决策层完全接管”，若恢复 clean-anchor/epoch 阶段行为时边界把握不清，可能重新引入双重门控。
- repeated deadline 升级链既涉及 coordinator 侧 episode 记忆，也涉及 policy 侧 reconnect gate；若只改一层，容易出现 reconnect 过度或 budget 漂移。
- 一部分测试夹具直接手动写 runtime stats / ledger，若不先校正夹具，容易把旧契约误判为产品行为回归。

## Progress

- [x] Step 1: 盘点并修正 `transport_session`/ledger 测试夹具的分层契约失配
- [x] Step 2: 修复 fresh output / clean-anchor / transport-await / epoch 生命周期闭环
- [x] Step 3: 修复 repeated deadline -> reconnect 升级链与节流/budget/success-edge
- [x] Step 4: 修复同 tick transport 与 local recovery 仲裁顺序
- [x] Step 5: 修复 `RtcConnectionService` keyframe stage 与 clean-anchor/suppress 对齐
- [x] Step 6: 完成定向测试与全量基线回归，确认行为差异收敛

## Execution Notes

- Date: 2026-04-17 | Status: planned
- Update: 新建 RFC，基于代码而非仅基于失败测试名拆解行为差异，确定本轮主线不是 34 个独立修复点，而是 5 个责任断点的收口。
- Decision: 先修测试分层契约，再修真实行为差异；避免在 `transport_session` 和 `connection::service` 上用补丁方式重建旧状态机。
- Risk/Blocker: 当前尚未执行定向测试复核每一组差异的最新失败面，后续实现阶段需要以实际测试结果刷新本 RFC 的执行记录。

- Date: 2026-04-17 | Status: in-progress
- Update: `connection/service.rs` 恢复 clean-anchor 驱动的 epoch 轮换清理：epoch 变化时清空 `video_recovery_transport_state.stage` 与 `last_sent_at_ms`，并保留 clean anchor / transport-await / PLI 节流下的 stage 解析语义，修复 clean anchor 后旧 stage token 泄漏到新 epoch 的问题。
- Update: `recovery/runtime_state.rs` 的 `has_fresh_media_output()` 增加未来时间戳保护；当 present/decode 时间明显领先 `now_ms` 超过 10s 时回退到 `unix_now_ms()`，避免时钟偏斜把陈旧输出误判为 fresh。
- Update: `recovery/state_machine.rs` 在 `update_recovery_epoch()` 中补齐完整清链：epoch 轮换时重置 `Healthy` 状态、清空 keyframe / decoder-reset / reconnect in-flight，并刷新 `state_entered_at`，阻断旧 recovery chain 穿透到新 epoch。
- Update: `recovery/coordinator.rs` 新增 `escalation_controller` 与 epoch 同步；`LifecycleRecovering`、`TransportExpiredDeadline`、`TransportSevereDeadline`、`TransportRecoveredLate`、`TransportSampleLoss`` 统一走 connectivity escalation。connectivity 路径的 budget 快照改为读取状态机真实 `snapshot_budget_state()`，避免 proposal 阶段提前消费旧 controller 内部预算。
- Decision: 保留 `LifecycleRecovering` 的“已有 reconnect in-flight 时压成 `CooldownSuppressed`”规则，但不把同样抑制扩展到 `TransportExpiredDeadline`，否则 repeated deadline 无法正常升级到 connectivity reconnect。
- Update: `session/policy.rs` 收紧 fresh output 抑制规则，补齐 transport-await 完成态吸收与 stalled replay 重新落回 local probe 的闭环；`DisplaySupplyCritical` 仅在 fresh media + fresh present + renderer 未 stalled 时吸收，避免仅凭 decode/包到达误判为稳定恢复。
- Update: 为 cloud stale transport-await replay 增加 `should_reenter_transport_await_local_probe()`，在 clean anchor 已存在但 decoder / renderer 仍 stalled、且尚无 hard recovery evidence 时，允许把 `CooldownSuppressed` 拉回 `RequestKeyframe`，恢复本地探测。
- Fix: 上述 `should_reenter_transport_await_local_probe()` 首版在持锁读 `runtime_stats` 闭包内再次调用 `transport_await_has_hard_recovery_evidence()`，导致同一把 runtime stats 锁重入死锁。最终改为在 `recovery/coordinator.rs` 抽出 `transport_await_has_hard_recovery_evidence_from_stats(stats, now_ms)` 纯判定函数，保留外层带锁包装器供其它调用点复用，彻底消除该死锁。
- Validation: 最终完整验证为 `cargo test -p xbxengine --lib --quiet`，结果 `914 passed / 0 failed / 8 ignored`。
- Date: 2026-04-17 | Status: completed
- Update: 本 RFC 所列 5 个责任断点已全部收敛，详细总结见报告 `docs/reports/2026-04-17-recovery-behavior-differences-repair.md`。
