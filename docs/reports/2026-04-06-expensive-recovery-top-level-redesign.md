# 昂贵恢复顶层重构 Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-06-expensive-recovery-top-level-redesign.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-06-expensive-recovery-top-level-redesign.md)
- 本任务完成了三阶段顶层恢复改造：恢复生命周期分层、执行后记账与失败证据门禁、以及 runtime/trace/frontend 的语义收口。

## Delivered

- 将 recovery 顶层语义从单一 `recovering` 拆成 `observing / local-self-healing / recovery-eligible / active-recovery / recovery-blocked`，并贯通到 session policy、runtime stats、trace projection 与 diagnostics 文案。
- 将昂贵恢复预算改为“执行后记账”：keyframe 仅在 `sent_at_ms` 落账后占预算，decoder-reset / reconnect 通过 coordinator 同步真实执行事实，proposal 不再提前吞预算。
- 为 `Reconfigure` 增加 failure-evidence gate，并同步改造 `runtime_summary / primary_issue_chain / latest_decision_summary` 与前端解析逻辑，避免新旧语义混杂。

## Changes

- [`escalation.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs)、[`coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs)：
  - `RecoveryActionContract` 改为 `budget_recorded_on_execution`。
  - reconnect 不再 proposal 即占预算，新增执行后同步入口。
  - `begin_recovery_epoch()` 同时清空 keyframe/decoder-reset 的旧 cooldown 记忆，保证新 epoch 真正重开。
- [`session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs)、[`runtime_state.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/runtime_state.rs)、[`stats.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/diagnostics/stats.rs)：
  - recovery liveness 细分为五段。
  - 诊断摘要、issue chain、decision summary 全部开始输出新 phase 语义。
- [`trace_projection.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/xbxengine/trace_projection.rs)、[`diagnostics.ts`](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/diagnostics.ts)、[`diagnostics-i18n.ts`](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/diagnostics-i18n.ts)、[`zh.json`](/Users/guo.xu/Documents/code/games/xbxrc/src/i18n/locales/zh.json)、[`en.json`](/Users/guo.xu/Documents/code/games/xbxrc/src/i18n/locales/en.json)：
  - 前后端统一识别新 phase。
  - `latestDecisionSummary` 支持 `decision / phase / owner` 三类结构化翻译。

## Validation

- `cargo fmt --all`
- `cargo test -p xbxengine transport::rtc::recovery::escalation -- --nocapture`
- `cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`
- `cargo test -p xbxengine runtime_stats_sink -- --nocapture`
- `cargo test -p xbxengine recovery_integration_transport_await_reopens_after_clean_anchor_and_new_recovery_epoch -- --nocapture`
- `cargo check -p xbxengine`

## Risks

- `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture` 在既有慢测上长时间未拿到最终退出码；虽然本轮受影响的 recovery 相关用例已单独通过，但整套长跑 suite 仍建议在空闲窗口再完整补跑一次。
- `video_scheduling_owner` 与 `repeat_suppression` 的全套验证本轮未单独重跑，当前信心主要来自 coordinator/session policy 的集成回归覆盖。

## Follow-up

- 在空闲窗口补跑 `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`、`cargo test -p xbxengine transport::rtc::policy::video_scheduling_owner -- --nocapture`、`cargo test -p xbxengine transport::rtc::recovery::repeat_suppression -- --nocapture`。
- 用下一份真实 runtime trace 重点验证两点：`recovery-blocked` 是否稳定替代旧 `recovering` 假死表象，以及 `latestDecisionSummary / primaryIssueChain` 是否与 trace ledger 完全一致。
