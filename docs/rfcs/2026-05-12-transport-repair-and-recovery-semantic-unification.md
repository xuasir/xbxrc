# Transport Repair And Recovery Semantic Unification RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: planned
- Owner: Codex Supervisor
- Last Updated: 2026-05-12

## Background

- 运行时日志 `runtime-trace-1778549415932-1.jsonl` 显示，cloud 会话在首个可用 IDR 到达后可以短暂进入稳态，但很快又被 `small gap -> transportExpiredDeadline -> transportAwaitRecoveryAnchor` 拉回恢复态。
- 同一坏窗内，TWCC 仍保持 `deliveryRatio=1.0`、`lossRatio=0.0`、`receiveKbps≈22~24Mbps`、RTT `183~191ms`，说明平均网络质量不是主因。
- 当前主问题落在“补包语义”和“恢复语义”没有统一：
  - 媒体层先按 `keyframe/reference/delta` 做初始价值判断。
  - 预算层再按 `Disposable / Supply / Anchor` 与 `LocalDrop / WaitKeyframe / ChainBroken` 计算 admission。
  - transport/timeline/recovery 又把同一批缺包重新编码为 `gap-repair-in-flight / transportAwaitRecoveryAnchor / ReferenceGap / AnchorGap / ChainBroken`。
- 这条链路里存在三类语义漂移：
  - 原始低价值 gap 会在 `repairing / awaiting-keyframe` 语境下被抬成 `Supply`。
  - `timeline.gap.is_some()` 会直接被解释成 `ReferenceGap`。
  - `gap-repair-in-flight` 会过早写入 `AwaitingRecoveryKeyframe`，把“正在本地补”误写成“正在等关键帧”。

## Goal

- 建立一套从媒体价值、预算、transport NACK、timeline 到 recovery 的单一语义合同。
- 明确区分三类缺包：
  - 直接丢掉的低价值包
  - 可以先做本地 NACK 的中等价值包
  - 必须恢复或等待关键帧的高价值包
- 让系统在每个时刻都能明确回答：
  - 当前是在 `drop`
  - 还是在 `local NACK repair`
  - 还是在 `wait keyframe`
  - 还是已经升级到 `request IDR`
- 收紧 `ReferenceGap / AnchorGap / ChainBroken` 的成立条件，避免 small gap 在证据不足时过早进入恢复主线。

## Scope

- In scope:
  - `crates/xbxengine/core/src/media/video/ingress/budget.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/nack_policy.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/contract.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`
  - `crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`
  - 相关 `session/policy`、trace、tests
- Out of scope:
  - 新 transport 路径
  - 更换现有恢复动作集合
  - 调整远端 PLI/IDR 行为
  - 改前端协议和对外 DTO

## Problem Statement

当前系统缺的不是某个阈值，而是以下三条合同。

### 1. 缺少统一的价值分层

- 当前有 `delta/reference/keyframe`。
- 当前有 `Disposable / Supply / Anchor`。
- 当前还有 `MinorGap / ReferenceGap / AnchorGap / ChainBroken`。
- 这些层级没有稳定的一一映射，同一批缺包会在不同模块被重复升格。

### 2. 缺少统一的动作阶段

- 当前可以看到 `SkippedLowValue / Attempted / SkippedTooLate / SkippedChainBroken`。
- 当前也可以看到 `gap-repair-in-flight / gap-expired / transportAwaitRecoveryAnchor / RequestPli`。
- 这些动作和阶段混在同一批 reason label 中，导致日志里很难分辨“系统正在本地补”和“系统已经放弃本地补、开始等关键帧”。

### 3. 缺少统一的升级证据

- `timeline.gap.is_some()` 就足以进入 `ReferenceGap`。
- `gap-repair-in-flight` 会立刻写入 `AwaitingRecoveryKeyframe`。
- cloud 路径下，非 `disposable` 的 skipped/too-late 会直接记成 `UnrecoverableReferenceChain`。
- 这些规则把“值得修”与“证据足够升级恢复”压得太近。

## Proposed Model

### A. 统一四维语义

所有缺包、补包和恢复事件统一输出以下四个维度。

1. `value_tier`
   - `low`
   - `medium`
   - `high`

2. `risk_tier`
   - `none`
   - `repairable`
   - `reference`
   - `anchor`

3. `action_stage`
   - `drop`
   - `nack_pending`
   - `nack_missed`
   - `wait_keyframe`
   - `request_idr`

4. `evidence_scope`
   - `anonymous`
   - `frame_bound`
   - `chain_bound`
   - `anchor_bound`

约束：

- `value_tier` 只表达“这批缺包本身值多大”。
- `risk_tier` 只表达“现有证据表明它对恢复链的风险有多大”。
- `action_stage` 只表达“当前系统正在做什么”。
- `evidence_scope` 只表达“证据绑定到哪一层对象”。

### B. 统一三档价值

#### 1. 低价值包

- 定义：
  - 原始 `delta`
  - 未绑定明确参考帧
  - failure cost 为 `LocalDrop`
  - 当前没有 frame-bound / chain-bound 证据
- 合同：
  - `value_tier = low`
  - `risk_tier = none`
  - 允许 `drop` 或机会型 `nack_pending`
  - 失败后只记 `nack_missed`
  - 不允许直接进入 `wait_keyframe`
  - 不允许直接进入 `ReferenceGap`

#### 2. 中等价值包

- 定义：
  - 原始 `reference/supply`
  - 或 frame-bound 的 continuation / delta cluster
  - 当前有供给压力，但没有明确坏链证据
- 合同：
  - `value_tier = medium`
  - `risk_tier = repairable`
  - 优先 `nack_pending`
  - 失败先记 `nack_missed`
  - timeline 进入“可修补缺口”，不直接进入 `ReferenceGap`
  - 允许继续观察后续 frame completion / gap resolved

#### 3. 高价值包

- 定义：
  - anchor/keyframe 缺失
  - 已出现 frame-bound 或 chain-bound 的明确不可恢复证据
  - 或连续 `bootstrapMissingIdr + clean anchor 无推进`
- 合同：
  - `value_tier = high`
  - `risk_tier = reference` 或 `anchor`
  - `action_stage` 才允许进入 `wait_keyframe`
  - 只有高价值失败才允许进入 `request_idr`

### C. 统一阶段机

缺包在链路中的合法流转固定为：

1. `drop`
2. `nack_pending`
3. `nack_missed`
4. `wait_keyframe`
5. `request_idr`

约束：

- `drop -> request_idr` 禁止直跳。
- `nack_pending -> wait_keyframe` 需要明确升级证据。
- `nack_missed` 只表示“本地修补这次没有成功”，不自动表示坏链。
- `wait_keyframe` 只表示“已放弃继续赌本地修补，进入关键帧恢复语境”。

## Required Contract Changes

### 1. `GapSeverity` 需要新增中间层

当前：

- `MinorGap`
- `ReferenceGap`
- `AnchorGap`
- `ChainBroken`
- `RecoveryBlocked`

目标：

- `LowValueGap`
- `RepairableGap`
- `ReferenceGap`
- `AnchorGap`
- `ChainBroken`
- `RecoveryBlocked`

约束：

- `timeline.gap.is_some()` 只能推出 `RepairableGap`。
- 只有明确 `reference` 级证据，才能推出 `ReferenceGap`。

### 2. `gap-repair-in-flight` 不能再写成 `AwaitingRecoveryKeyframe`

目标改造：

- `gap-repair-in-flight` 只表示 `action_stage = nack_pending`
- 单独引入 `AwaitingGapRepair` 或 `LocalRepairPending` 语义
- `AwaitingRecoveryKeyframe` 只在升级到高价值恢复语境后才成立

### 3. `UnrecoverableReferenceChain` 需要拆分

当前 cloud 路径将非 `disposable` 的 skipped/too-late 统一记成 `UnrecoverableReferenceChain`。  
目标改造：

- `UnrecoverableLate`
- `UnrecoverableSupplyMiss`
- `UnrecoverableReferenceChain`

约束：

- `Supply miss` 不直接等价为 `reference chain broken`
- 只有 `frame_bound + chain evidence` 才能写 `UnrecoverableReferenceChain`

### 4. `NackDeadlineExpired` 需要携带原始预算上下文

当前 `NackDeadlineExpired` 只有 `missing_packets`。  
目标改造：

- 附带：
  - `value_tier`
  - `risk_tier`
  - `frame_importance`
  - `frame_rtp_timestamp`
  - `frame_unrecoverable_reason`
  - `evidence_scope`

约束：

- recovery 层只消费事件自身的升级上下文
- 不再依赖“最新全局 timeline 已经被升格后的状态”反推原始含义

## Decision Rules

### Rule 1: 低价值包

- RTT 紧张、高压、display starved 时优先 `drop`
- RTT 充裕时允许试一次 `nack_pending`
- deadline miss 后保持 `nack_missed`
- 绝不单独触发 `wait_keyframe`

### Rule 2: 中等价值包

- 优先 `nack_pending`
- deadline miss 后先落到 `nack_missed`
- 若后续出现 `gap_resolved / frame_complete / fresh continuation`，可回到正常供给
- 若同一 frame/chain 邻域持续失败，再升级 `risk_tier`

### Rule 3: 高价值包

- 一旦确认坏链证据成立，停止继续赌本地修补
- 进入 `wait_keyframe`
- 仅在远端仍无可用恢复关键帧时进入 `request_idr`

## Implementation Plan

1. Batch 1：语义合同拆层
   - 新增 `LowValueGap / RepairableGap`
   - 拆分 `UnrecoverableSupplyMiss`
   - 明确 `action_stage`

2. Batch 2：transport 与 timeline 对齐
   - `gap-repair-in-flight` 改成 `local repair pending`
   - `timeline.gap.is_some()` 改为 `RepairableGap`
   - `NackDeadlineExpired` 携带上下文

3. Batch 3：owner/coordinator 升级收口
   - 只让 `ReferenceGap / AnchorGap / ChainBroken` 进入 recovery 主线
   - `continuation-only` 改成辅助证据，不再单独推动重入恢复

4. Batch 4：trace 与 tests 收口
   - 所有日志统一输出四维语义
   - 增加 low/medium/high 三档矩阵测试

## Validation

- [ ] 新增 low-value gap 不进入 `ReferenceGap` 的测试
- [ ] 新增 medium-value gap deadline miss 只落到 `nack_missed` 的测试
- [ ] 新增 high-value gap 才允许进入 `wait_keyframe / request_idr` 的测试
- [ ] 新增 `gap-repair-in-flight` 不再写 `AwaitingRecoveryKeyframe` 的测试
- [ ] 新增 `NackDeadlineExpired` 携带原始预算上下文的测试
- [ ] 定向回放当前 trace，确认 `small gap` 不再直接把系统拖进恢复主线

## Risks

- 如果 `RepairableGap` 收得过宽，会让真坏链升级变慢。
- 如果 `ReferenceGap` 收得过严，会让部分真正需要等关键帧的场景延后恢复。
- 如果四维语义只在局部模块落地，owner/session/trace 继续保留旧字符串 reason，会出现双轨合同。

## Progress

- [x] Step 1: 完成根因分析，确认问题是语义漂移，不是单点网络指标
- [x] Step 2: 完成统一模型设计，固定三档价值与四维语义
- [ ] Step 3: 等待确认后进入实施拆分

## Execution Notes

- Date: 2026-05-12 | Status: planned
- Update: 根据 `runtime-trace-1778549415932-1.jsonl` 与 `transport/rtc` 主线代码，确认当前主问题是“包价值、修补状态、参考链风险、恢复锚点需求”被混用，导致 small gap 在 cloud 高 RTT 与远端 IDR 不积极的组合下被过早升级到恢复态。
- Decision: 本轮先冻结统一语义合同，再按 `budget -> transport -> timeline -> owner/coordinator -> trace` 顺序改造，不做单点阈值修补。
- Decision: 实施阶段以“低价值直接丢、中等价值先本地 NACK、高价值才升级恢复”为主线，不改远端行为假设。
- Risk/Blocker: 需要在实施前统一 `reason label` 与 trace 输出，否则旧日志和新语义会并存，验证结果会混淆。
