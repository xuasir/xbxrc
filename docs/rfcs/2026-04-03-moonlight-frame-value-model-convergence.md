# Moonlight 帧价值模型落地 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- `docs/frame.md` 已经把问题说得很清楚：预算不能只看 `delta/reference/keyframe` 的粗粒度标签，而要同时覆盖恢复阶段、链路价值、RTT 余量、失败代价和时间窗来源这 5 个维度。
- 现有实现并不是没有帧价值，而是把相关信号分散在多层里：
  - [`crates/xbxengine/core/src/media/video/types.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/types.rs) 里有 `FrameValue` / `FrameRecoveryDisposition`
  - [`crates/xbxengine/core/src/media/video/ingress/budget.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/ingress/budget.rs) 和 [`crates/xbxengine/core/src/media/video/ingress/scheduler.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/ingress/scheduler.rs) 里有准入、晚到、backlog 的局部预算
  - [`crates/xbxengine/core/src/transport/rtc/stream/nack_scheduler.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/nack_scheduler.rs) 与 [`crates/xbxengine/core/src/transport/rtc/stream/video_source/nack_policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/nack_policy.rs) 已在做 RTT / 价值 / deadline 的 admission 分档
  - [`crates/xbxengine/core/src/transport/rtc/recovery/runtime_state.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/runtime_state.rs) 与 [`crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 里已经能识别恢复阶段和主恢复视图
- 最近的实机 trace 也暴露出同类失配：表面上是 `delta` 的晚到或恢复 keyframe 等待，但实际演化成整条参考链失活和 `Reconnecting/no-signal` 风暴。说明现有局部规则虽然各自合理，但还没有形成统一的“这一帧当前到底值不值得救”的合同。

## Goal

- 把 Moonlight 的帧价值思路落成一套统一、可执行、可观测的 Rust 预算模型。
- 保留现有 `FrameValue` 作为“帧本身的内在价值”，再增加独立的运行时预算上下文，承载恢复阶段、链路价值、RTT 余量、失败代价和时间窗来源。
- 让 `ingress / nack / recovery` 三条主线消费同一份价值合同，避免不同层各自判断“该不该救、该不该等、该不该升级”。
- 最终目标不是再加阈值，而是让每一次 drop / retry / escalate 都能回答：为什么是这帧、为什么是现在、为什么用这个窗口。

## Scope

- In scope:
  - [`crates/xbxengine/core/src/media/video/types.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/types.rs)
  - [`crates/xbxengine/core/src/media/video/ingress/budget.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/ingress/budget.rs)
  - [`crates/xbxengine/core/src/media/video/ingress/scheduler.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/ingress/scheduler.rs)
  - [`crates/xbxengine/core/src/transport/rtc/stream/frame_cadence.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/frame_cadence.rs)
  - [`crates/xbxengine/core/src/transport/rtc/stream/nack_scheduler.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/nack_scheduler.rs)
  - [`crates/xbxengine/core/src/transport/rtc/stream/video_source/nack_policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/nack_policy.rs)
  - [`crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs)
  - [`crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.rs)
  - [`crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs)
  - [`crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs)
  - [`crates/xbxengine/core/src/transport/rtc/recovery/hard_stall.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/hard_stall.rs)
  - runtime stats / diagnostics / trace projection 里与帧价值、恢复阶段、deadline 来源相关的字段
- Out of scope:
  - 引入第二条 transport / signaling / media pipeline
  - 直接照搬 Moonlight 的协议实现或 SDL/FFmpeg 线程模型
  - 把 `FrameValue` 改成一个只会扩大的“全能大对象”
  - 只做前端展示、不改后端预算合同

## Plan

1. 固化一层独立的 `FrameBudgetContext`。
   - `FrameValue` 继续表示帧本身的内在价值，只负责依赖关系、刷新加权、payload 尺寸等静态属性。
   - 新增一层运行时预算上下文，承载恢复阶段、链路价值、RTT 余量、失败代价和时间窗来源。
   - 让 `FrameBudgetContext` 成为 NACK、Ingress、Recovery 的共同输入，而不是每层各写一套隐式判断。
2. 把预算合同接到 ingress / NACK 准入。
   - ingress 侧不再只按固定 `min/max delay` 判断，而是结合恢复阶段和窗口来源来决定 `DropLate / DropBacklog / Reconfigure / WaitKeyframe`。
   - NACK admission 侧继续保留现有 `estimated_recovery_arrival_ms / frame_playout_deadline_at_ms`，但把它们提升为合同中的显式字段，和 RTT 余量、失败代价一起参与 skip / retry / chain-broken 判定。
3. 把同一份合同接入恢复升级。
   - `rebuilding-supply / reconnecting / priming / steady` 等恢复阶段要影响 keyframe / decoder reset / reconnect 的优先级。
   - 参考链关键帧、恢复关键帧、普通 delta 以及低价值帧不再只靠编码标签区分，而要结合失败代价来决定是否升级。
4. 补齐观测与回归。
   - 在 trace / runtime stats 中显式记录五个维度与最终预算结果，保证实机可以直接解释“为什么救 / 为什么丢 / 为什么升级”。
   - 用近期真实 trace 回放验证：delta 晚到不再轻易放大成整链漂白，恢复风暴能更早在预算层收敛。

## Validation

- [x] 为 `FrameBudgetContext` / `FrameValue` 的组合规则补单测，覆盖恢复阶段、链路价值、RTT 余量、失败代价和时间窗来源五个维度
- [x] 为 `nack_scheduler` / `video_source::nack_policy` / `ingress::scheduler` 补跨层一致性测试，确认同一帧在不同入口不会产生冲突结论
- [x] 回放 `runtime-trace-1775197534489.jsonl`，确认第一次 reconnect 与后续 `no-signal` 风暴都能被新的预算合同解释
- [x] 变更完成后执行 `cargo fmt --all`、`cargo test -p xbxengine ...`、`cargo check -p xbxengine`

## Risks

- 如果把恢复阶段和链路价值直接塞进 `FrameValue`，会把 media 基础类型做得过重，后续维护成本会很高。
- 如果 RTT 余量和失败代价算得过于保守，Cloud 启动和高 RTT 场景会被过度放缓，低延迟收益反而下降。
- 如果只在 ingress 或只在 recovery 做这层预算，而不让 NACK admission 共享同一合同，仍然会出现“上游觉得该救、下游觉得该丢”的失配。

## Progress

- [x] Step 1: 已完成 `docs/frame.md`、近期 trace 以及现有 `FrameValue` / NACK / recovery 入口的对齐分析
- [x] Step 2: 已固化 `FrameBudgetContext` 与五维预算合同
- [x] Step 3: 已贯通 ingress / NACK / recovery 的统一消费路径
- [x] Step 4: 已补齐 trace / stats / 回归验证
- [x] 当前 recovery / observability 子项已完成 `recovery_stage / chain_value / failure_cost / window_source` 的 stats / trace 投影，并在 recovery coordinator / hard stall 里接入 stage-aware 优先级偏置
- [x] 当前验证已完成，仓库里其他并行改动不影响本轮交付

## Execution Notes

- Date: 2026-04-03 | Status: completed
- Update: 已完成从 `FrameValue` 到 `FrameBudgetContext` 的两层模型落地，并将其贯通到 ingress / NACK / recovery / trace / frontend 的完整消费链路。
- Decision: 采用“两层模型”而不是“全能大对象”路线：`FrameValue` 只保留帧的内在价值，`FrameBudgetContext` 承载恢复阶段、链路价值、RTT 余量、失败代价、时间窗来源。
- Risk/Blocker: 目前没有阻塞项；仓库里仍存在与本任务无关的少量 dead-code warnings，但不影响本轮交付。
