# Steady Recovery Stale Diagnosis Expiry Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-04-stale-recovery-diagnosis-expiry.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-04-stale-recovery-diagnosis-expiry.md)
- 已完成 steady 恢复后陈旧 `adapterIdleTimeout` 重放问题的主线修复，阻断 policy 在没有新恢复意图时持续把旧 diagnosis 推回 coordinator。

## Delivered

- 在 [`session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 增加 `adapterIdleTimeout` fallback 门控。
- 补齐 steady-progress 与 real-stall 两侧回归测试。
- 完成 `xbxengine` 定向测试与编译验证。

## Changes

- `RtcSessionPolicy::build_recovery_proposal()` 在 fallback 使用 `latest_diagnosis_label` 前，先检查 `adapterIdleTimeout` 是否已被 fresh media output / current clean anchor 吸收。
- 新门控复用 `runtime_state::has_fresh_media_output()`，并叠加当前 recovery epoch 的 clean anchor 与 decoder/renderer 非 stalled 条件，避免继续把健康链路打回恢复态。
- 新增 `stale_adapter_idle_timeout_does_not_replay_during_steady_progress` 与 `active_adapter_idle_timeout_still_reaches_recovery_path` 两个回归测试，确保既能止住恢复风暴，也不吞掉真实 idle stall。

## Validation

- `cargo test -p xbxengine stale_adapter_idle_timeout_does_not_replay_during_steady_progress -- --nocapture`
- `cargo test -p xbxengine active_adapter_idle_timeout_still_reaches_recovery_path -- --nocapture`
- `cargo check -p xbxengine`

## Risks

- 门控目前只针对 `adapterIdleTimeout`，其他可能陈旧化的 diagnosis 仍依赖各自链路的状态机与预算收敛。
- 若后续 trace 出现“无 clean anchor、present/decode 也不新鲜，但链路实际已恢复”的特殊窗口，还需要继续补更细的进展证据。

## Follow-up

- 用下一份真实 runtime trace 确认尾段不再持续刷 `adapterIdleTimeout:adapterIdleTimeout` / `cooldownSuppressed`。
- 继续观察 `video_source/sink.rs` repair/RTX 与恢复链路之间是否还存在会放大抖动的边缘耦合。
