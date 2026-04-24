# RTC 控制面八项清理 RFC

> 复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内。

## Status

- Completion: 已完成
- Current State: completed
- Owner: xbxengine / rtc session
- Last Updated: 2026-04-12

## Background

- 控制面词汇应以 `session::control_model` 的 `Stage` / `FaultDomain` / `CostCeiling` 为唯一编排语义。
- 观察层 `recovery_diagnosis` 的 RFC 后缀须与当拍策略一致，避免「字符串反解析」与 ledger 叙事状态双轨漂移。

## Goal

- **FaultDomain 单源**：`resolve_session_fault_domain(VideoEscalationReason)` 为唯一分类入口；`recovery/coordinator::RecoverySignalDomain` 仅作内部桶，由 `fault_domain_to_recovery_signal_domain` 映射，语义冻结。
- **昂贵重连**：`RequestReconnectCandidate` 的升格与抑制仅经 `session::expensive_recovery_gate` + `RtcSessionPolicy` 主链。
- **本地媒体层边界**：包级 `value` / `deadline` / `retry` 留在 `nack_scheduler`、ingress、`FrameBudgetContext`；session/policy 不重算包级预算。
- **RFC 诊断单源**：`rfcFault` / `rfcMaxCeiling` / `rfcStage` / `rfcCeiling` 由当拍 policy 写入 `XbxEngineMediaRuntimeStats` 权威字段，`diagnostics/stats` 合并时优先消费。
- **叙事状态**：`RecoveryLedgerNarrativeState`（原 `RecoveryLivenessState`）仅用于 recovery decision ledger 文案，不参与分支编排。

## Scope

- In scope: `control_model.rs`、`api/backend.rs`、`session/policy.rs`、`diagnostics/stats.rs`、本 RFC 与回归矩阵文档。
- Out of scope: 修改 `XbxEngineStatsDto` / 协议独立字段；大规模重写 `VideoSchedulingOwner`。

## Plan

1. 本 RFC 与 `control_model` 模块契约注释。
2. `MediaRuntimeStats` 增加 `recovery_rfc_authoritative_fault_domain` / `recovery_rfc_authoritative_stage`；policy 每拍写入；`merge_recovery_diagnosis_with_rfc_tags` 优先权威字段。
3. 将 `RecoveryLivenessState` 重命名为 `RecoveryLedgerNarrativeState` 并注明 ledger-only。
4. 收缩明显冗余的 `reason_label` 分支（在不影响语义前提下）。
5. 静态审计结论记入「Execution Notes」。
6. 跑通 Validation 矩阵。

## Validation

- [x] `cargo test -p xbxengine transport::rtc::stream::nack_scheduler`
- [x] `cargo test -p xbxengine media::video::ingress`
- [x] `cargo test -p xbxengine transport::rtc::recovery::coordinator`
- [x] `cargo test -p xbxengine transport::rtc::session::policy`
- [x] `cargo test -p xbxengine runtime_stats_sink`
- [x] `cargo test -p xbxengine diagnostics::stats`
- [x] `cargo test -p xbxrc trace_projection -- --test-threads=1`
- [x] `cargo check -p xbxengine`
- [x] `cargo check -p xbxrc`

## Risks

- 诊断字符串黄金断言需与权威字段写入路径同步更新。
- 部分测试夹具使用完整 `XbxEngineMediaRuntimeStats { ... }` 字面量，新增字段后需补字段或改用 `..Default::default()`。

## Progress

- [x] Step 1: RFC + control_model 注释
- [x] Step 2: 权威 fault/stage 字段与 stats 合并
- [x] Step 3: RecoveryLedgerNarrativeState 重命名
- [x] Step 4: 字符串控制收缩与审计笔记
- [x] Step 5: 回归矩阵

## Execution Notes

- Date: 2026-04-12 | Status: completed
- Update: 已落地 `recovery_rfc_authoritative_fault_domain` / `recovery_rfc_authoritative_stage`；`merge_recovery_diagnosis_with_rfc_tags` 优先权威字段；`RecoveryLedgerNarrativeState` 替代旧名；`is_exploratory_transport_await_keyframe` 去掉冗余 `reason_label` 判定；新增单测 `merge_recovery_diagnosis_prefers_authoritative_rfc_tags_over_owner_strings`。

### 静态审计（FaultDomain / Reconnect / 本地层）

- **FaultDomain**：`coordinator::classify_signal_domain` 已委托 `resolve_session_fault_domain`；新增域映射必须落在 `control_model`。
- **Reconnect**：`RequestReconnectCandidate` 由 `escalation`/`coordinator` 产出动作，session 侧由 `ExpensiveRecoveryGate::apply_to_proposal` 等门控；`recovery_ramp_guard` 等仅读取 `RecoveryAction`，不单独制造重连语义。
- **本地层**：policy 不 `use` nack_scheduler / ingress scheduler 内部预算类型做二次决策；帧价值以 runtime stats / timeline 聚合为准。

---

**Audit date**: 2026-04-12
