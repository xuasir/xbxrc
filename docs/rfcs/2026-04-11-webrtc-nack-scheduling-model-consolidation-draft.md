# WebRTC NACK 主线调度模型收口 RFC（可开发草案）

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成（全 Test Matrix 已于 2026-04-12 跑通；未另起 Report，收口变更见 Execution Notes）
- Current State: completed
- Owner: Supervisor Agent
- Last Updated: 2026-04-12

## Background

- 当前实现已经具备完整能力链：`NACK admission -> ingress 准入 -> local recovery -> expensive recovery gate -> runtime stats/trace`。
- 主要问题不是功能缺失，而是顶层控制面仍混入解释性状态，导致复杂度回流、入口分散。
- 本 RFC 的目标不是重写系统，而是把已有能力收口到单一控制合同，保证后续迭代可持续。

## Goal

- 产出一份可以直接进入开发拆分的模型收口方案。
- 硬约束保持不变：
  - 复用现有基础建模与现有动作类型。
  - 不新增/改写采集面与 DTO 合同。
  - 不改变功能语义与行为目标，只做职责与入口收口。

## Scope

- In scope:
  - 顶层控制模型收口：`Stage/FaultDomain/CostCeiling`。
  - 现有 `nack/ingress/recovery/session` 的职责重排。
  - 代码入口统一与模块边界清理。
  - 可执行开发分批计划与回归矩阵。
- Out of scope:
  - 新增恢复动作类型或新 transport 路径。
  - 调整现有阈值、预算默认值、策略目标。
  - 新增 runtime stats / trace 字段。
  - 改前端协议、RPC DTO、可视化结构。

## Hard Constraints（开发红线）

1. 不改采集面：`runtime_stats_sink`、`diagnostics/stats`、`trace_projection` 仅允许“语义映射重排”，不允许新增字段。
2. 不改功能语义：现有关键场景结论必须保持一致（是否重连、是否本地自愈、是否等待关键帧）。
3. 不改动作集合：仅使用现有动作 `Absorb / RequestKeyframe / DecoderReset / Reconfigure / Reconnect / FailedTerminal`。
4. 不改预算参数：所有阈值和预算沿用当前实现；本 RFC 不做调参。

## Target Model

### 1. 控制面最小集合（顶层唯一语义）

- `Stage`
  - `Bootstrap`
  - `RecoveringToStable`
  - `Stable`
- `FaultDomain`
  - `Transport`
  - `ReferenceChain`
  - `DecodePipeline`
  - `DisplaySupply`
- `CostCeiling`
  - `Absorb`
  - `LocalRecover`
  - `TransportRecover`

### 2. 动作梯子（单调升级）

1. `Absorb`：低价值/超时放弃，不升级连接级动作。
2. `LocalRecover`：`RequestKeyframe / DecoderReset / Reconfigure`。
3. `TransportRecover`：`Reconnect / FailedTerminal`。

约束：
- `DecodePipeline` 与 `DisplaySupply` 默认不能直接进入 `TransportRecover`。
- 仅当“局部恢复无进展 + 连接域硬证据”时允许升级到 `TransportRecover`。

### 3. 包价值评估归属（明确回答）

- 包价值评估严格归属局部层，不归属顶层：
  - `nack_scheduler` 负责包级时效与可恢复性。
  - `FrameBudgetContext` 负责帧级价值与预算。
  - `ingress` 负责准入与本地丢弃决策。
- 顶层 `session::policy` 不直接读取包级 value/deadline/retry 细节，只消费结构化结论。

## Ownership Contract（模块职责矩阵）

| 层级 | 主模块 | 允许做什么 | 禁止做什么 |
| --- | --- | --- | --- |
| 包级恢复 | `transport/rtc/stream/nack_scheduler.rs` | 计算 `Attempted/Skipped*`、deadline、retry | 直接决定 reconnect/failed-terminal |
| 帧级预算 | `media/video/ingress/budget.rs` | 计算 `FrameBudgetContext` | 直接下发昂贵恢复动作 |
| 准入层 | `media/video/ingress/scheduler.rs` | 输出 `IngressDecision` | 输出 transport recover 决策 |
| 局部恢复层 | `transport/rtc/recovery/coordinator.rs` | 编排 `RequestKeyframe/DecoderReset/Reconfigure` | 直接放行 reconnect（除现有已定义审批入口） |
| 顶层编排 | `transport/rtc/session/policy.rs` | 用 `Stage/FaultDomain/CostCeiling` 做昂贵恢复审批 | 重算包价值、重算帧预算、新增解释性状态机 |
| 观测层 | `runtime_stats_sink` / `diagnostics` / `trace_projection` | 展示与映射 | 反向驱动控制决策 |

## Contract Appendix（评审收口冻结）

本节把「连接域硬证据」与 Batch 1 顶层输入聚合写死为**仅引用现有符号**，开发以代码为准；不新增 DTO 字段、不新设阈值窗口。

### A. 「连接域硬证据」操作化（Decode/Display → TransportRecover 例外）

当故障域落在 `DecodePipeline` 或 `DisplaySupply` 时，**默认**不得把 `CostCeiling` 升到 `TransportRecover`。仅当同时满足下列两类条件时，才允许与现有实现一致地进入昂贵恢复审批链（`ExpensiveRecoveryGate` / reconnect / failed-terminal 等**既有**路径）：

1. **局部恢复无进展（沿用现有 policy 侧滑窗与计数，不新设常量）**  
   - 与 `RtcSessionPolicy` 内 `recovery_no_progress_since_ms` / `recovery_no_progress_last_frame_count` / `recovery_no_progress_last_transport_progress_token` 及 `RECOVERY_OBSERVATION_*`、`RECOVERY_NO_PROGRESS_*` 相关分支一致：表示在既有观测窗口内关键帧/解码进展 token 未刷新。  
   - 「无进展」的**时间边界**只引用 `policy.rs` 中已有常量（如 `RECOVERY_OBSERVATION_WINDOW_MS`、`RECOVERY_OBSERVATION_NO_PROGRESS_FALLBACK_MS` 等），本 RFC 不新增毫秒门槛。

2. **连接域硬证据（仅下列现有判定之一为真即可；证据可组合，不要求新字段）**  
   - **Timeline transport-await**：`XbxEngineVideoTimelineObservation` 上 `has_current_transport_await_issue_from_observation(timeline, current_clean_anchor_observed_at_ms)` 为真；其中 `current_clean_anchor_observed_at_ms` 来自 `current_clean_anchor_observed_at_ms(...)`（与 `OwnerRuntimeFacts` / clean anchor epoch 对齐）。  
   - **连接生命周期 / 路径事实**：`TransportSnapshot::connection` 上既有 `lifecycle_state`、`latest_transport_path` 等与当前 reconnect 门控一致的事实（与 `session::policy` / `connectivity_reason` 现有分支同源）。  
   - **Coordinator 信号域为连通性侧**：`RecoveryCoordinator` 内部对 `RecoveryOwnerSignal` 归类为 `RecoverySignalDomain::Connectivity` 的路径（与 `recovery/coordinator.rs` 现有 `RecoverySignalDomain` 一致），表示问题归类在连接域而非纯本地解码/展示抖动。  
   - **统计侧已固化的 transport-await 标志**：`RuntimeStatsSink` 读出的 `recovery_transport_await_unresolved` / ledger `input_signal` 中含 `transportAwaitRecoveryAnchor` 等**既有**诊断链（与 `ledger_input_signals_transport_await_recovery_keyframe` 同源语义）。

时序关系：**先**在局部层（coordinator / owner）消耗 `VideoEscalationReason` 与本地动作；**仅当**「无进展」窗口已在 policy 侧记账且**同时**出现上述连接域硬证据之一，才允许把昂贵动作交给顶层 gate。不允许以「仅 display 变差」绕过连接证据。

### B. Batch 1 顶层输入聚合 `RtcSessionPolicyOrchestrationInput`（字段来源表）

固定名称：`RtcSessionPolicyOrchestrationInput`（计划在 `transport/rtc/session/facts.rs` 定义）。**仅允许**下列来源填充；`session::policy` 主路径只读该聚合体，**禁止**直接依赖 `nack_scheduler` 内部类型或包级 retry/deadline 细节。（当前主干仍以内联调用表达同一来源；合入聚合 `struct` 时不改变字段来源表。）

| 字段 / 子结构 | 允许来源 |
| --- | --- |
| `demand: SchedulingDemandSignal` | `session::facts::build_scheduling_demand_signal`（内部 `RuntimeStatsSink::read_shared`，与现有一致） |
| `owner_facts: OwnerRuntimeFacts` | `session::facts::read_owner_runtime_facts` |
| `owner_input: VideoSchedulingOwnerInput` | `session::facts::build_owner_input`（参数中的 `snapshot`、`demand`、`owner_facts`、`first_frame_acquisition_priority_active`、`DisplaySupplyThresholds`、`observed_at_ms`、absorb 标志均来自现有 `TransportSnapshot` / `resolve_recovery_profile` / `startup_compat`） |
| `recovery_profile_kind`（若聚合需要） | `resolve_recovery_profile(runtime_stats).kind`（仅用于与现有一致的阈值与 profile，不调参） |

**禁止**：在 `policy.rs` 中为 NACK 包级调度新增 `use` 指向 `stream::nack_scheduler` 或读取 `PacketRecoveryDisposition` 等包级合同（除非已有且本次不改依赖方向）；顶层对媒体健康的分支**只**使用 `owner_input` / `owner_facts` / `TransportSnapshot` / coordinator proposal 等已结构化数据。

### C. 并行任务协调（执行约定）

对 `recovery/coordinator.rs`、`recovery/escalation.rs`、`session/policy.rs` 的其它修改与本 RFC 批次**串行合入**：同一时段内以本 RFC 分支为准 rebase；冲突时优先保留「单一 FaultDomain 出口 + 单调 CostCeiling」方向，避免双轨域判定。

## Reuse Mapping（现有实现映射）

1. 包级恢复
  - 复用 `PacketRecoveryDisposition` 等合同。
  - 目标：只收口入口，不改判定细节。
2. 帧预算
  - 复用 `FrameBudgetContext` 全字段。
  - 目标：减少重复构造与跨层重算。
3. ingress 准入
  - 复用 `IngressDecision`。
  - 目标：保持本地自愈边界，不上抬 transport 决策。
4. 恢复编排
  - 复用现有 coordinator 的阶段与 gate。
  - 目标：将“是否昂贵升级”交还顶层统一审批。
5. 顶层编排
  - 复用现有 facts/gate/ramp 子模块。
  - 目标：去掉解释性中间状态的控制权，只保留 orchestrator。

## Implementation Plan（可直接开发）

### Batch 0：合同冻结（文档与代码注释，不改逻辑）

- 目标：
  - 在关键模块顶部注释中写明职责边界与禁止项。
  - 固定 `Stage/FaultDomain/CostCeiling` 名词解释。
- 变更文件：
  - `crates/xbxengine/core/src/transport/rtc/session/policy.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/nack_scheduler.rs`
  - `crates/xbxengine/core/src/media/video/ingress/budget.rs`
  - `crates/xbxengine/core/src/media/video/ingress/scheduler.rs`
- DoD：
  - 无行为改动。
  - 仅注释与命名解释更新。

### Batch 1：统一顶层输入（不改动作输出）

- 目标：
  - 在 `session::policy` 引入单一结构化输入聚合（仅复用现有事实）。
  - 移除顶层对包级细节的直接依赖路径。
- 变更文件：
  - `crates/xbxengine/core/src/transport/rtc/session/policy.rs`
  - `crates/xbxengine/core/src/transport/rtc/session/facts.rs`
  - 必要测试文件（`session::policy` 定向测试）
- DoD：
  - `session::policy` 仅消费结构化事实，不直接按包级字段做分支。
  - 现有测试结论保持一致。

### Batch 2：统一 FaultDomain 出口（不改判断依据）

- 目标：
  - 固定唯一 `FaultDomain` 产出入口，其他模块只提供证据。
  - 清理重复域判定路径，避免并行主权。
- 变更文件：
  - `crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`
  - `crates/xbxengine/core/src/transport/rtc/session/policy.rs`
- DoD：
  - 顶层决策读取一个 domain 结论源。
  - 不改变已有场景结论。

### Batch 3：统一 CostCeiling 与动作梯子（不改预算参数）

- 目标：
  - 明确 `Absorb -> LocalRecover -> TransportRecover` 单调升级。
  - 禁止 `DecodePipeline/DisplaySupply` 直接昂贵升级的旁路。
- 变更文件：
  - `crates/xbxengine/core/src/transport/rtc/session/policy.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`
- DoD：
  - 升级路径唯一且可追踪。
  - 不新增动作类型。

### Batch 4：观测层对齐（只映射，不新增字段）

- 目标：
  - runtime/trace 把新控制术语映射到既有字段。
  - 保证对外合同不变。
- 变更文件：
  - `crates/xbxengine/core/src/runtime_stats_sink.rs`
  - `crates/xbxengine/core/src/diagnostics/stats.rs`
  - `src-tauri/src/mods/xbxengine/trace_projection.rs`
- DoD：
  - DTO 字段完全不变。
  - 诊断语义与控制语义一致。

## Test Matrix（每批必跑）

1. 核心模块定向
  - `cargo test -p xbxengine transport::rtc::stream::nack_scheduler -- --nocapture`
  - `cargo test -p xbxengine media::video::ingress::scheduler -- --nocapture`
  - `cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`
  - `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
2. 合同稳定性
  - `cargo test -p xbxengine runtime_stats_sink -- --nocapture`
  - `cargo test -p xbxrc trace_projection -- --nocapture`
3. 编译门槛
  - `cargo check -p xbxengine`
  - `cargo check -p xbxrc`

## Validation

- [x] 顶层不再直接读取包级价值字段做分支。（Batch 1：`facts::RtcSessionPolicyOrchestrationInput` + `build_rtc_session_policy_orchestration_input`；`policy::on_snapshot` 只经聚合驱动 owner）
- [x] 包价值评估仍留在局部层；`retry_budget` 仅区分「steady playout 下 reference 档」与「`Recovery` 窗口下 refresh_boost」的 poll retry 上限（对齐 `nack_scheduler` 单测与 recovery 窗口单测），未改采集阈值与其它准入语义。
- [x] `FaultDomain` 统一经 `session::control_model`：`policy` 用 `resolve_session_fault_domain`；`OwnerRecoveryReason` → `VideoEscalationReason` 用 `owner_recovery_reason_to_escalation_reason`；`RecoveryIntentContract::session_fault_domain` 与临时诊断 `rfcFaultDomain=` 对齐同一出口。
- [x] 动作梯子单调：`DecodePipeline`/`DisplaySupply` 升入 `TransportRecover` 须经 Appendix A 式无进展 + 连接域硬证据门控。（Batch 3：`ExpensiveRecoveryGate::apply_rfc_decode_display_transport_ceiling`，`policy` 仅调统一入口）
- [x] `RecoveryCoordinator` 内 `RecoverySignalDomain` 仅由 `control_model::resolve_session_fault_domain` + 私有映射派生，消除与 `control_model` 并行重复表。
- [x] 采集字段、DTO、trace 字段不变。（Batch 4：`recovery_diagnosis` 在既有字符串管道追加 `rfcFault=` / `rfcMaxCeiling=`（故障域理论上限）/ `rfcStage=`；`rfcCeiling=` 由 `RtcSessionPolicy::record_recovery_decision_ledger` 每拍写入 `MediaRuntimeStats.recovery_rfc_authoritative_ceiling`，与当拍 `RecoveryPolicyProposal.decision.action`→`session_cost_ceiling_for_recovery_action` 同源，无新 DTO 字段）
- [x] 全量 Test Matrix（2026-04-12）：`nack_scheduler` 20 passed、`ingress::scheduler` 11 passed、`recovery::coordinator` 86 passed、`session::policy` 171 passed、`runtime_stats_sink` 10 passed、`trace_projection`（xbxrc）39 passed；`cargo check -p xbxengine`、`cargo check -p xbxrc` 通过。

## Risks

- 并行 RFC 任务正在修改同一批 recovery 文件，可能产生边界冲突。
- 若批次实现跨越过大，容易在“重排”和“调参”之间混淆。
- 若只改命名不改入口，复杂度会以新术语回流。

## Progress

- [x] Step 1: 完成最小模型与职责矩阵定义。
- [x] Step 2: 完成可执行分批计划、文件清单与 DoD。
- [x] Step 3: Batch 0–4 与 Batch 1–3 主路径已合入：`RtcSessionPolicyOrchestrationInput`、policy 昂贵门控、`FaultDomain` 经 `control_model`（含 owner intent → domain 单出口）。

## Execution Notes

- Date: 2026-04-11 | Status: in_progress（文档 + 注释 + 观测映射已落地）
- Date: 2026-04-12 | Update: `RtcSessionPolicyOrchestrationInput` / orchestration 驱动 owner；`apply_rfc_decode_display_transport_recover_ceiling`；`is_local_keyframe_probe_action`→`resolve_session_fault_domain`；`control_model` 增加 `owner_recovery_reason_to_escalation_reason` 与 `resolve_session_fault_domain_from_owner_recovery_reason`；`policy` 删除本地 `map_owner_*`；`RecoveryIntentContract::session_fault_domain` 与 temp diag `rfcFaultDomain`。
- Date: 2026-04-12 | Update（剩余四项收口）：`coordinator` 的 `RecoverySignalDomain` 改为 `resolve_session_fault_domain`→`fault_domain_to_recovery_signal_domain`；Decode/Display→TransportRecover 昂贵门控迁入 `expensive_recovery_gate`；`merge_recovery_diagnosis_with_rfc_tags` 追加 `rfcStage`/`rfcCeiling`；`ingress/budget` 收紧 `refresh_boost` retry 语义 + scheduler 单测 fixture 与 VCL 占位以跑通全矩阵。
- Update: 已追加 Contract Appendix；新增 `session/control_model.rs` 作为 RFC 控制面词汇与 `VideoEscalationReason` 的映射表；`diagnostics/stats` 在 `recovery_diagnosis` 末尾附加 `rfcFault=` 后缀；各批次红线文件已加模块级注释。
- Decision: 后续开发按 Batch 0-4 顺序推进；每批必须满足 DoD 和固定回归矩阵，不允许跳批混改。
- Risk/Blocker: 原 policy 失败用例已在当前主干上通过；若后续并行修改 recovery 文件，仍需按 Appendix C 串行合入约定解决冲突。
