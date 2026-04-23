# RFC: RTC Recovery Boundary Convergence

Completion: 未完成
State: planned
Owner: Codex
Created: 2026-04-22

## Background

当前 RTC 恢复链路已经形成清晰的分层趋势：

- 局部模块负责短链执行与事实采样，例如 `video_source`、decode、pacer、transport 执行层。
- 全局模块负责恢复归因、预算、阶段判断与动作选择，例如 `session::policy`、`recovery::escalation`。

这套方向整体正确，但代码中仍存在三类复杂度累积：

1. 边界仍不够锋利：全局层仍承接了较多局部语义解释，局部层也残留少量历史状态机影子。
2. 解释链条偏长：同一异常会在 observation、owner signal、policy、escalation、command bridge 多次翻译。
3. 高置信场景不够强裁决：系统整体偏保守，容易给人“能解释但不够果断”的体感。

当前问题不在于“是否需要全局恢复调度”，而在于需要让全局层只处理真正的全局事，同时为少数高置信故障路径提供更短、更硬的升级通道。

## Goals

1. 收边界：明确哪些判断属于局部闭环，哪些判断必须进入全局恢复编排。
2. 减解释：减少同一信号在多层中的重复翻译与重复语义包装。
3. 强化少数高置信决策路径：为少数确定性强、恢复收益高的故障场景提供更直接的动作升级。
4. 保持当前“局部短链执行 + 全局策略裁决”的总体方向，不回退到连接层自带完整恢复状态机。

## Non-Goals

1. 不重写整个 RTC 恢复系统。
2. 不改变固定技术栈或跨层主边界（Tauri + Rust / Vue 3 + TypeScript）。
3. 不把成熟的 Rust 侧恢复逻辑迁回前端或其他运行时。
4. 不追求把所有恢复决策都下沉到局部层。
5. 不在本 RFC 中直接处理所有现存调参问题或所有诊断字段补充问题。

## Problem Statement

### 1. Global policy currently knows too much local detail

`session::policy` 目前同时处理：

- fault domain
- recovery stage
- display / decode / transport 交叠抑制
- first-frame acquisition 优先级
- transport-await 局部 probe 特判
- display supply 与恢复动作抑制

这导致全局层既像“编排层”，又像“局部例外仲裁器”。

### 2. Multiple layers reinterpret the same signal

一个局部异常从采样到执行，往往要经历：

- source / pacer / decode 产生 observation
- session facts / owner signal 再表达
- `session::policy` 再归因
- `recovery::escalation` 再升级
- `policy::scheduling` 再映射 command
- `transport_session` 再包装执行语义

这提高了可观测性，但也提高了维护成本与误判概率。

### 3. High-confidence scenarios are diluted by the generic ladder

当前统一梯子为：

- Absorb
- LocalRecover
- TransportRecover

该梯子适合多数抖动场景，但对少数高置信故障链（例如长期 `transportAwaitRecoveryAnchor`、明确 bootstrap blockage、明显 display starvation 且伴随 transport/decode 证据）仍然过于依赖通用抑制与冷却路径，导致决策显得保守。

## Proposed Direction

### A. Boundary tightening

将判断拆成三类，并在代码中建立明确契约：

1. Local-closed judgments
   - 明确由局部模块闭环处理，不再上浮做二次解释。
   - 示例：
     - `video_source` 内已确认的包级恢复状态、head-missing / OOS 样本统计、局部 admission 结果
     - pacer 本地队列预算、局部 pressure 吸收
     - transport 执行层对 command 成败的纯执行记录

2. Global-only judgments
   - 仅全局层负责，不允许下层重复实现。
   - 示例：
     - fault domain
     - cost ceiling
     - recovery budget
     - reconnect 资格
     - `RequestPli` / `RequestFir` / `RequestDecoderReset` / `RequestReconnectCandidate` 的最终选择

3. Contracted handoff facts
   - 局部层提供结构化事实，全局层只消费这些事实，不重算底层语义。
   - 示例：
     - “局部恢复已失败且具备硬证据”
     - “显示供给退化，但缺少 transport 证据”
     - “bootstrap 阻塞仍在持续，且非短暂抖动”

### B. Explanation reduction

围绕“减少重复翻译”做三项收敛：

1. 收缩 `session::policy` 中的局部例外判断
   - 把能在 source / pacer / decode 明确定义的局部状态，改为以事实字段输入，而不是在 policy 中二次推理。

2. 收缩 bridge 层语义拼装
   - `transport_session` 保留执行结果与 ledger 回写，但避免再追加与 policy 重叠的判断语义。

3. 清理历史状态机影子
   - 对已经上移到策略层的动作选择，移除连接层遗留的历史阶段机辅助逻辑、测试命名和注释影子，避免“谁说了算”不清晰。

### C. High-confidence fast paths

引入“少数强裁决路径”，但仅限高收益、高置信的故障家族：

1. Persistent transport-await fast path
   - 条件：
     - `transportAwaitRecoveryAnchor` 持续超过固定窗口
     - 已具备硬恢复证据
     - 当前回合局部修复无实效
   - 行为：
     - 直接跳过部分通用吸收逻辑，进入更强局部恢复或连接恢复。

2. Bootstrap blockage fast path
   - 条件：
     - 重复出现 bootstrap blockage
     - 非 IDR/无效 bootstrap 明确阻塞首帧建立
   - 行为：
     - 减少通用 suppress / cooldown 的往返，直接走固定升级梯子。

3. Display starvation with corroborating evidence
   - 条件：
     - display supply 严重退化
     - 同时缺少 fresh decode / present
     - 并且伴随 transport 或 decode 侧硬证据
   - 行为：
     - 避免长期停留在 display-only 吸收态，允许更快抬升到对应动作。

这些 fast path 不应泛化为新的“大而全分支系统”，而应当收敛成少数显式命名的策略入口。

## Design Principles

1. 事实优先，不重复推导。
2. 局部层负责短链执行与局部闭环，全局层负责裁决与预算。
3. 默认保守，但对高置信故障路径允许更快升级。
4. 新增策略必须可测试、可观测、可解释。
5. 任何上提或下沉都必须减少重复语义，而不是制造新的双重所有权。

## Impacted Modules

- `crates/xbxengine/core/src/transport/rtc/stream/video_source/*`
- `crates/xbxengine/core/src/media/video/pacer/*`
- `crates/xbxengine/core/src/media/video/decode/*`
- `crates/xbxengine/core/src/transport/rtc/session/control_model.rs`
- `crates/xbxengine/core/src/transport/rtc/session/policy.rs`
- `crates/xbxengine/core/src/transport/rtc/policy/scheduling.rs`
- `crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs`
- `crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`
- `crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs`
- `crates/xbxengine/core/src/transport/rtc/connection/service.rs`
- 相关 policy / recovery / connection / source tests

## Implementation Plan

### Phase 1: Boundary inventory and contract extraction

1. 盘点当前局部模块向全局层输出的 observation / fact。
2. 划分 local-closed judgments、global-only judgments、contracted handoff facts。
3. 为 handoff facts 建立统一命名与字段约定。
4. 标记连接层和 bridge 层中已不应继续拥有的历史策略逻辑。

### Phase 2: Policy slimming

1. 收缩 `session::policy` 中局部例外判断数量。
2. 将可局部闭环的判断前移到 source / decode / pacer，并以事实字段上送。
3. 简化 `transport_session` 中与 policy 重叠的语义拼装。
4. 清理连接层遗留状态机影子与误导性测试命名。

### Phase 3: High-confidence fast paths

1. 为 `transportAwaitRecoveryAnchor` 持续失败路径引入显式 fast path。
2. 为 bootstrap blockage 引入显式 fast path。
3. 为 display starvation + corroborating evidence 引入显式 fast path。
4. 为每条 fast path 补齐独立测试与诊断字段。

### Phase 4: Verification and rollout hardening

1. 回归现有 recovery integration / policy tests。
2. 为局部闭环与全局裁决交接点新增契约测试。
3. 使用 runtime trace 对比改造前后的：
   - 动作数量
   - 平均升级层级
   - 高置信路径恢复耗时
   - 无效 suppress / cooldown 次数

## Validation Plan

1. 单元测试
   - `session::policy`
   - `recovery::escalation`
   - `transport_session`
   - `connection::service`
   - `video_source`

2. 集成测试
   - recovery integration 现有矩阵
   - reconnect lifecycle
   - display owner ledger

3. 运行时日志验证
   - 选取典型 `transportAwaitRecoveryAnchor`
   - bootstrap blockage
   - display starvation trace
   - 对比动作路径是否更短、解释链是否更少、升级是否更直接

## Risks

1. 过度下沉判断会让局部层重新长成新的“迷你 policy”。
2. fast path 过多会重新引入难以维护的特判森林。
3. 如果 handoff facts 定义不清，可能只是把重复解释从一层搬到另一层。
4. 过度强调果断升级可能牺牲短时抖动场景下的稳定性。

## Mitigations

1. 仅允许少数显式命名的 fast path。
2. 所有边界调整必须附带契约测试。
3. 对每个被下沉的判断写清所有权与消费者。
4. 用 runtime trace 对升级速度与误触发率做双向验证。

## Progress Checkpoints

- [ ] 完成边界盘点与 handoff facts 清单
- [ ] 完成 `session::policy` 瘦身方案
- [ ] 完成连接层 / bridge 层历史策略影子清理方案
- [ ] 完成三条高置信 fast path 设计
- [ ] 完成测试与 trace 验证方案

## Open Questions

1. `RequestFir` 的能力门控最终契约应完全由策略层承担，还是保留执行层兜底校验？
2. display supply 信号应继续作为全局恢复输入，还是进一步收缩为“仅在有 corroborating evidence 时才进入恢复编排”？
3. 当前 recovery ledger 的语义字段中，哪些属于调试冗余，哪些必须保留给 trace 分析？
