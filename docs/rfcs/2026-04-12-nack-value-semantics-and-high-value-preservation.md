# NACK Value Semantics And High-Value Preservation RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。  
> 完成报告：[`docs/reports/2026-04-12-nack-value-semantics-and-high-value-preservation.md`](../reports/2026-04-12-nack-value-semantics-and-high-value-preservation.md)

## Status

- Completion: 已完成
- Current State: completed
- Owner: Codex / rtc transport scheduling
- Last Updated: 2026-04-12

## Background

- 当前 `video_source/nack.rs` 已完成 `Startup / Recovery / Steady` 三态收口，并把 `window_source / frame_value / repairability` 接入统一预算。
- 这轮继续按“首帧 -> priming -> steady”回看实现时，发现两类偏移：
  - `SkippedLowValue` 在 `Recovery` 压力下会被直接提升成 `SkippedChainBroken`，再进入 `maybe_trigger_reference_chain_recovery()`；这会把“低价值放弃”误写成“参考链已坏”。
  - `Supply / Reference` 这类高价值帧在 `Steady` 期的 near-deadline 路径仍偏硬，当前实现更偏低延迟洁癖，不够偏保活，容易把本可放宽一点继续保链的问题过早收口为 `SkippedTooLate`，并通过后续恢复面继续放大。
- 这两个偏移都落在 `NACK admission` 语义层，不应通过改 owner/session 主恢复状态机来兜。

## Goal

- 纠正 `NACK` 层的语义边界：`SkippedLowValue` 只表示“不值得补”，不再冒充坏链证据。
- 重排 `NACK` 对不同价值帧的策略优先级：高价值帧在 `Startup / Priming / Steady / Recovery` 均保持同等语义地位，只允许时效窗口和升级门槛不同，不允许价值语义漂移。
- 在不破坏当前首帧、priming、steady、recovery 主线调度的前提下，提升 `Supply / Reference` 帧的保活能力，减少由于 near-deadline 过硬导致的过早恢复升级。
- 保持“真实坏链 -> keyframe recovery”主线不变，只收紧坏链证据来源。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/nack_scheduler.rs`
  - `crates/xbxengine/core/src/media/video/ingress/budget.rs`
  - 相关 `policy_tests / recovery_integration / nack` 单测补齐
- Out of scope:
  - `video_scheduling_owner` 主状态机改写
  - `session::policy` / `expensive_recovery_gate` / reconnect budget 规则改写
  - 首帧 bootstrap / invalid bootstrap / startup 保护窗语义改写
  - decoder reset / reconnect / failed-terminal 主线策略变更

## Non-Goals

- 不把 `NACK` 升级为连接级恢复主权。
- 不取消 `Startup / Recovery / Steady` 三态。
- 不把 steady 放宽成“无限补包”或“弱化 deadline”。
- 不修改“真坏链 -> recovery keyframe”已有主线。

## Design Principles

1. 先纠正语义，再调参数，不做跨层混改。
2. 先按帧价值决定 repair 优先级，再由 phase 微调 guard/slack/retry，而不是先由 phase 决定命运。
3. `NACK` 层只能产出 transport repair 事实，不能凭 low-value admission 伪造 reference-chain broken 事实。
4. 高价值帧的意义跨阶段一致；不同阶段只应影响“愿意等多久、重试几次、何时升级”，不应影响“它值不值得保活”。
5. 所有放宽都限于 `Anchor / Supply / Reference`，不得顺带放宽 `LowValue / Disposable`。

## Current Problems To Fix

### 1. Low-Value Skip 语义过重

- `should_promote_recovery_pressure_low_value_skip(...)` 当前在 `Recovery` 期会把以下 low-value 理由直接升级为 `SkippedChainBroken`：
  - `cloudHighRttLowValueAdmission`
  - `displayStarvedLowValueAdmission`
  - `estimatedArrivalNearDeadlineLowValue`
  - `sampleLoss` 的一部分低 repairability 分支
- 上述理由最多只能说明“这次补包性价比低 / 时效差 / 显示压力高”，并不等于“参考链已经不可承接”。
- 这会导致 `maybe_handle_chain_broken(...)` 与 `maybe_trigger_reference_chain_recovery(...)` 被低价值 admission 误触发。

### 2. 高价值帧 near-deadline 策略偏硬

- 当前 `Supply / Reference` 在 `Steady` 期 near-deadline 会直接落到 `SkippedTooLate`；在 `Recovery` 期则会更激进地走 `SkippedChainBroken`。
- 这会让高价值包在 steady 下缺少“保活优先”的缓冲空间，阶段切换后容易从“还可以试一次”直接滑到“放弃并继续放大恢复信号”。
- 当前 `FrameBudgetContext` 已经会把部分恢复语义下的帧抬成 `Supply`，但上层 near-deadline 分支没有与之匹配的更保活 admission。

## Proposed Changes

### A. 语义纠偏：Low-Value Skip 不再自动升级坏链

1. 收紧 `should_promote_recovery_pressure_low_value_skip(...)`：
   - 禁止仅因 `cloudHighRttLowValueAdmission`、`displayStarvedLowValueAdmission`、`estimatedArrivalNearDeadlineLowValue` 就返回“坏链”。
   - 若保留 `sampleLoss` 的升级能力，也必须要求其明确影响 `Reference / Supply continuity`，而不是只因 `delta + low repairability`。
2. `SkippedLowValue` 保持为“局部 repair 放弃”终态，不再直接驱动 `maybe_trigger_reference_chain_recovery(...)`。
3. `maybe_handle_chain_broken(...)` 改为只消费 reference-chain 级证据：
   - `referenceChainUnrecoverable`
   - `waiting_for_recovery_keyframe`
   - 已有 chain-broken timeline 事实
   - 明确影响 reference continuity 的 sample loss / supply deadline miss
4. 将 `frame_unrecoverable_reason` 显式分层：
   - `lowValue:*`
   - `timing:*`
   - `referenceChain:*`
   避免上层把 low-value skip 和坏链证据混读。

### B. 策略重排：按帧价值优先，phase 只做微调

1. 统一三档价值：
   - `Anchor`
   - `Supply / Reference`
   - `LowValue / Disposable`
2. 各阶段共同语义：
   - `Anchor`：尽最大可能补
   - `Supply / Reference`：保活优先，允许更宽的 deadline slack
   - `LowValue / Disposable`：优先低延迟，可较早放弃
3. `Startup / Recovery / Steady` 只影响：
   - retry budget
   - near-deadline guard
   - RTT slack
   - 升级到 keyframe recovery 的门槛
4. 不再允许“同样的高价值包，仅因 phase 从 startup 变 steady，就从继续尝试直接滑到严格放弃”的硬切换。

### C. 高价值帧保活优化

1. `Supply / Reference` near-deadline 不再只用单一硬阈值处理。
2. steady 期对 `Supply / Reference` 优先：
   - 再尝试一次 NACK
   - 或进入保活型 late repair
   - 只有确认赶不上且会污染 playout 时，才 `SkippedTooLate`
3. recovery 期对 `Supply / Reference` 不再因 near-deadline 自动记为 `SkippedChainBroken`；
   必须先证明“它会破坏当前参考链承接”。
4. `refresh_boost` / clean-anchor 短窗内提升出的 `Supply` 语义，优先转化为更宽 repair window，而不是更快触发 keyframe 恢复。

## Invariants

1. `NACK` 仍是局部 transport repair，不接管 owner/session/coordinator 的恢复主权。
2. `chain broken -> keyframe recovery` 主线保留。
3. `Startup / Recovery / Steady` 三态保留。
4. 首帧到 priming 的主线不变：
   - invalid bootstrap 仍拒绝
   - 首帧保护窗仍生效
   - priming 期对 supply/reference 仍较 steady 宽容
5. reconnect / decoder reset / failed-terminal 规则不在本 RFC 内变更。

## Plan

1. 重构 `NACK` 语义层：
   - low-value skip 与 chain-broken 证据彻底拆分
   - `maybe_handle_chain_broken(...)` 只认 reference-chain 级证据
2. 重排高价值帧 admission：
   - 将 `Supply / Reference` 近 deadline 路径改为保活优先
   - phase 仅影响 guard/slack/retry，不改价值语义
3. 补齐回归矩阵并基于现有首帧/priming/steady 流程做定向验证

## Validation

- [x] 现有“真坏链 -> recovery keyframe”相关测试保持通过（定向：`cargo test -p xbxengine transport::rtc::stream::video_source::nack`、`nack_scheduler`、`timeline::estimated_arrival_near_deadline_low_value`）
- [x] 现有首帧 / priming / startup 保护相关测试保持通过（同上 + `startup_supply_near_deadline_keeps_nack_attempt`）
- [x] 新增：`displayStarvedLowValueAdmission` 在 recovery 压力下仍保持 `SkippedLowValue`
- [x] 新增：`cloudHighRttLowValueAdmission` 不再直接升级 `SkippedChainBroken`（低价值 delta sample loss 亦不再单独抬链断）
- [x] 新增：steady 下 `Supply / Reference` near-deadline 允许额外保活尝试（与 startup 一致保持 `Attempted`，由 scheduler 收口）
- [x] 新增：steady 放宽后，`LowValue` 不会借机长期占用 repair budget（仍走 `SkippedLowValue` + scheduler 抑制/预算，未改 `LowValue` 准入）
- [x] 新增：只有明确 reference-chain 证据时才触发 `maybe_trigger_reference_chain_recovery()`（`SkippedChainBroken` 白名单 + timeline `chain_broken`；`SkippedLowValue` 仅 timeline 真坏链）

## Risks

- 如果 reference-chain 证据收得过严，可能导致真坏链场景恢复变慢。
- 如果高价值帧 near-deadline 放宽过头，可能损伤 steady 期低延迟目标。
- 如果只改 disposition，不同步 reason 分类，owner/session 仍可能误读历史观测。

## Progress

- [x] Step 1: 拆分 low-value skip 与 chain-broken 证据
- [x] Step 2: 重排 `Supply / Reference` near-deadline admission
- [x] Step 3: 补齐并跑通回归矩阵

## Execution Notes

- Date: 2026-04-12 | Status: completed
- Update: 根据首帧 -> priming -> steady 审查结果，新增 RFC 固化 `NACK` 语义纠偏与高价值帧保活优化方向。
- Decision: 这轮只改 `NACK admission / budget` 语义，不改 owner/session 主恢复状态机。
- Decision: 第一类问题定性为“语义偏差”；第二类问题定性为“高价值帧策略过硬”，按帧价值优先重排，不按 phase 先行重写。
- Risk/Blocker: 若缺少“reference continuity 真断裂”与“只是 low-value 放弃”的分层测试，实施时容易重新引入语义漂移。
