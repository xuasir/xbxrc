# 解码与 Pacer 的 Deadline/Budget 解耦 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: in-progress
- Owner: Codex
- Last Updated: 2026-04-21

## Background

- 新 trace 表明当前 `decode -> pacer -> render/host` 之间仍然存在较深的局部调度耦合：
  - decode 端通过输出队列状态决定何时停止继续输入；
  - pacer 端通过本地队列深度和 host pressure 动态收紧 drop target；
  - render/host 端仍保留最后一层局部纠偏，并把 display 侧局部拥塞继续反馈给上游恢复叙事；
  - recovery owner / session 仍会直接消费 `outputQueueOverflow`、`rendererQueueOverflow`、`queuePressure` 这类局部队列事实。
- 这种模型的问题是：
  - 低延迟目标不稳定：短时 recovery burst 会被 `outputQueueOverflow -> queuePressure` 连续放大。
  - 职责边界混乱：decode 负责产出帧，pacer 负责决定显示时机，render/host 负责最终提交与呈现，三层都在做局部节奏控制。
  - 恢复叙事被局部事实污染：display 层短时拥塞会被放大成上游 media recovery 升级信号。
  - 当前的“加深 recovery 队列”有短期止血价值，但长期仍是阈值补丁，系统模型还会继续漂移。
- 现在系统更需要的是：
  - decode 基于 frame deadline / frame age / local budget 产出与早期淘汰；
  - pacer 作为唯一 release governor，负责“哪些帧还能按时显示”。
  - render/host 只处理呈现域局部纠偏，不再回流改写上游调度主叙事；
  - recovery owner / session 只消费真正代表媒体供给失败的事实，不再把局部队列事件直接映射为恢复升级。

## Goal

- 将 `decode -> pacer` 从“队列状态双向驱动”收敛成“deadline/budget 单向驱动”。
- 让 decode 停止把“队列是否非空”作为主调度信号，只保留极少量本地缓冲保护。
- 让 pacer 成为唯一的消费调度器，统一负责 release cadence、late drop、catch-up 和 local drop。
- 让 render/host 退出上游调度闭环，只保留 presentation 域的最终提交与局部平滑。
- 让 recovery owner / session 的升级叙事与局部队列事实解耦，只由媒体供给失败、clean anchor 进度、display completion 分层事实驱动。
- 保持低延迟目标优先，局部缓冲只作为短时 burst 吸收器，不演化成高延迟蓄水池。

## Scope

- In scope:
  - [`crates/xbxengine/core/src/media/video/decode/video_decode.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/video_decode.rs)
  - [`crates/xbxengine/core/src/media/video/decode/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/actor.rs)
  - [`crates/xbxengine/core/src/media/video/pacer/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/pacer/actor.rs)
  - [`crates/xbxengine/core/src/media/video/render/pacer.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/render/pacer.rs)
  - [`crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs)
  - [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs)
  - 相关 `video_decode` / `media::video::pacer` 测试
  - runtime trace / diagnostics / recovery narrative 字段补充
- Out of scope:
  - renderer / host scheduler 全量重写
  - recovery owner / session / value 模型整体推翻重写
  - ingress / NACK / transport 主链改造

## Design

### 1. 职责重新划分

- decode:
  - 职责是“尽快产出可用帧”
  - 允许做早期本地淘汰
  - 不再根据 `pacer` 的即时队列状态频繁切换节奏
- pacer:
  - 唯一 release governor
  - 统一负责 submit now / sleep / catch-up / drop
  - 统一裁决“当前帧是否还有显示价值”

### 2. 主判据从 queue depth 改成 deadline/budget

- decode 不再以“输出队列非空”作为 `PullOutputFirst` 主判据。
- decode 只在以下情况下进入强背压：
  - 本地缓冲达到 hard cap
  - 最老帧 age 已超过 decode-side local budget
  - renderer/pacer 明确返回持续不可消费信号
- pacer 的主判据改为：
  - frame deadline
  - frame age
  - host cadence
  - catch-up 状态
  - burst budget 是否已耗尽
- queue depth 保留为次级信号，只用于：
  - hard cap 保护
  - diagnostics
  - 极端 pressure 下的降级

### 3. 两类预算

- steady budget:
  - 常态路径
  - 浅缓冲
  - 快速 late drop
- recovery burst budget:
  - 仅在 `window_source=recovery` 生效
  - 允许短时更深缓冲
  - 预算是时间窗，不是长期队列特权
  - 一旦回到 steady，预算快速收缩

### 4. 控制点收敛

- decode 侧只保留一个 hard backpressure gate
- pacer 侧只保留一个 release/drop gate
- 不允许再出现“decode 因局部队列轻微非空就主动停，pacer 因普通压力又快速清队”这种双重调节

### 5. render/host 退出上游调度闭环

- render/host 继续负责最终提交、present cadence 对齐、bounded queue 保护。
- render/host 的局部纠偏只作用在 display 域：
  - latest-slot 替换
  - bounded queue 裁剪
  - present 节奏微调
- render/host 不再反向影响 decode ingress demand 与 pacer release 主判据。
- host pressure 继续保留为 pacer 的次级降级信号，用于极端场景保护提交链，平稳场景不再主导 release cadence。

### 6. 恢复叙事隔离

- owner / session 侧继续观测 decode、pacer、renderer 的局部 drop 与 queue 事件。
- 这些事件进入两类通道：
  - display diagnostics：用于解释局部拥塞、丢帧、present 抖动；
  - media recovery narrative：仅在能证明媒体供给失败时升级恢复。
- 进入 media recovery narrative 的核心事实收敛为：
  - clean anchor 缺失或回退
  - transport await / ingress wait keyframe 持续
  - decode 无有效输出且超过 budget
  - pacer 无可发布有效帧且 display completion 无进展
- `outputQueueOverflow`、`rendererQueueOverflow`、`queuePressure` 这类局部事实默认只记入 diagnostics，只有与媒体供给失败合同同时满足时才作为佐证进入恢复升级。

### 7. 观测与诊断

新增或强化这些诊断量：

- decode oldest frame age
- decode buffered playout budget ms
- pacer oldest frame age
- pacer buffered playout budget ms
- render/host bounded queue age / depth
- recovery burst active / remaining budget
- drop reason 按阶段区分：
  - decodeHardBackpressure
  - decodeLateEvict
  - pacerLateDrop
  - pacerCatchUpDrop
  - pacerPressureDrop
  - renderBoundedQueueTrim
  - hostPresentCadenceCorrection
- recovery narrative evidence source 区分：
  - mediaSupplyFailure
  - displayLocalPressure
  - displayCompletionStall

## Plan

1. 先收敛判据：把 decode 的 `ingress_demand` 从 queue-non-empty 模型改成 budget/hard-cap 模型。
2. 再收敛 pacer：让 pacer 只按 deadline/age/cadence 做消费与丢帧，queue depth 降为次判据。
3. 把 recovery burst 从“更深队列”收敛成“有限时间预算”，避免 recovery 特权长期残留。
4. 收敛 render/host 与 owner/session 边界，让 display 局部纠偏退出上游调度闭环。
5. 收敛 owner/session 的恢复输入事实，只让真正媒体供给失败驱动升级，局部队列事实退回 diagnostics。
6. 补 trace 与测试，验证 steady 和 recovery 两条路径的丢帧语义稳定。
7. 用新 runtime trace 复核 `outputQueueOverflow`、`queuePressure`、present age、frame age、recovery narrative evidence 的变化。

## Validation

- [ ] `cargo test -p xbxengine video_decode -- --nocapture`
- [ ] `cargo test -p xbxengine media::video::pacer -- --nocapture`
- [ ] 新增 decode-side budget gate 定向测试
- [ ] 新增 pacer deadline/budget drop 定向测试
- [ ] 新增 owner/session 恢复叙事隔离定向测试
- [ ] 用新 trace 验证：
  - `decode:drop:outputQueueOverflow` 频次下降
  - `pacer:drop:queuePressure` 从主掉帧原因退到次要原因
  - steady 路径 `presentFps` 保持稳定
  - recovery 路径不会把短 burst 立即放大成局部风暴
  - `outputQueueOverflow` / `rendererQueueOverflow` / `queuePressure` 默认落在 display diagnostics
  - recovery upgrade ledger 只由 media supply failure / display completion stall 这类合同事实驱动

## Risks

- 如果 decode 退得过松，局部积压会被推迟到 pacer 端集中爆发。
- 如果 pacer 的 budget 过宽，会牺牲低延迟目标，变成隐藏蓄水池。
- 如果 recovery burst budget 退出过慢，steady 路径会被恢复期策略污染。
- 如果 render/host 与 owner/session 的边界收得不干净，display 层局部抖动仍会继续污染 recovery 升级。
- 如果 diagnostics 不同步补齐，后续 trace 很难区分“预算驱动 drop”和“单纯队列保护 drop”。

## Progress

- [ ] Step 1: 定义 decode-side hard cap 与 budget gate
- [ ] Step 2: 定义 pacer-side deadline/budget governor
- [ ] Step 3: 收敛 recovery burst budget 语义
- [ ] Step 4: 收敛 render/host 与 owner/session 边界
- [ ] Step 5: 补测试与 trace 字段
- [ ] Step 6: 新 trace 复核

## Execution Notes

- Date: 2026-04-21 | Status: in-progress
- Update: 当前系统已确认 `decode -> pacer` 存在较深的 queue-driven coupling。
- Decision: 新 RFC 采用“decode 产出、pacer 唯一调度”的职责边界。
- Decision: queue depth 从主判据降为次判据，deadline/budget 升为主判据。
- Decision: render/host 保留 display 域局部纠偏职责，退出上游调度闭环。
- Decision: owner/session 对局部 queue/drop 事实采用 diagnostics 与 recovery narrative 双通道分流。
- Update: 已修复 review 暴露的三处偏差：1）owner/session 不再把 decode/renderer 局部 queue 事实接入 owner 输入；2）decode 在浅队列打满且预算未耗尽时进入背压，预算耗尽后重新放行输入；3）pacer recovery 窗口改为按最近 recovery 帧的剩余时间预算判定，过期 recovery 帧不会持续维持 recovery 模式。
- Risk/Blocker: 需要先定义统一的 budget 语义，否则实现会重新退化成阈值堆叠。
