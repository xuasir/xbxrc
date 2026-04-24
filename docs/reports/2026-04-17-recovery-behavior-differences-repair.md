# 恢复系统行为差异修复 Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-17-recovery-behavior-differences-repair-plan.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-17-recovery-behavior-differences-repair-plan.md)
- 本任务已完成对恢复系统行为差异的代码级收敛，覆盖 clean-anchor / epoch 生命周期、transport-await 完成态吸收与 stalled replay 重入 local probe、repeated deadline 到 connectivity reconnect 的升级链、budget 快照一致性，以及 `RtcConnectionService` 的 stage 清理边界。

## Delivered

- 修复 `connection/service.rs` 在 clean anchor 后的 stage token 遗留与跨 epoch 泄漏问题。
- 修复 `state_machine.rs`、`coordinator.rs`、`policy.rs` 之间的 recovery epoch / reconnect / transport-await 协调语义。
- 新增并验证 transport-await hard evidence 的无锁 stats 版判定，消除 cloud stale replay 回归测试死锁。

## Changes

- `recovery/state_machine.rs` 在 epoch 轮换时完整清理 recovery chain，避免旧 in-flight 恢复命令穿透到新 epoch。
- `recovery/coordinator.rs` 将 connectivity escalation 与状态机 epoch、budget 快照统一，connectivity 路径不再依赖旧 escalation controller 的内部预算视图。
- `session/policy.rs` 补齐 transport-await replay 的两端语义：恢复完成时可被 clean anchor + healthy chain 吸收；输出再次 stalled 且仍无 hard evidence 时，可以重新进入 local keyframe probe。
- `recovery/runtime_state.rs` 对 future timestamp 增加保护，避免时钟偏斜污染 fresh output 判断。
- `connection/service.rs` 在新 recovery epoch 开始时主动清空旧 stage / send 时间戳，重新从 PLI/FIR 探测起步。

## Validation

- `cargo test -p xbxengine recovery_integration_cloud_stale_transport_await_replay_reenters_local_recovery_when_output_stalls -- --nocapture`
- `cargo test -p xbxengine recovery_integration_fresh_transport_await_absorption_expires_once_output_stalls -- --nocapture`
- `cargo test -p xbxengine recovery_integration_transport_await_exits_after_completion_evidence -- --nocapture`
- `cargo test -p xbxengine connected_reconnect_request_does_not_clear_inflight_without_edge -- --nocapture`
- `cargo test -p xbxengine reconnect_budget_snapshots_reflect_before_and_after_state -- --nocapture`
- `cargo test -p xbxengine video_recovery_clean_anchor_clears_stage_token_and_new_epoch_restarts_from_pli -- --nocapture`
- `cargo test -p xbxengine test_epoch_rotation_clears_previous_recovery_chain -- --nocapture`
- `cargo test -p xbxengine --lib --quiet` -> `914 passed / 0 failed / 8 ignored`

## Risks

- 当前 `xbxengine` 仍有一批历史 warnings，本轮未处理；它们不阻断测试通过，但会降低后续回归时的问题聚焦效率。
- `RtcSessionPolicy` 与 `RecoveryCoordinator` 仍是高耦合热区；后续如果继续调整 transport-await hard evidence 或 reconnect gate，仍需优先跑本报告列出的 integration 回归集。

## Follow-up

- 若继续推进恢复系统收敛，建议下一轮先单独清理 `recovery/coordinator.rs` / `state_coordinator.rs` 的 `private_interfaces` 与未使用字段 warnings，减少诊断噪音。
- 若后续再改 transport-await / local probe 逻辑，优先复用 `transport_await_has_hard_recovery_evidence_from_stats()`，避免再次在持锁闭包中重入 runtime stats 读取。
