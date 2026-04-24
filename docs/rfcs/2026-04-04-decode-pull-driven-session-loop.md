# Decode Pull-Driven Session Loop RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 上一轮已经完成显示侧 pacing 改造，trace 也确认主要问题集中在显示侧供给，而不是 decode 吞吐本身。
- 但当前 `MediaSessionLoop` 仍通过固定 `decode_drain_tick` 和 mailbox slot 推进 ingress -> decode handoff，decode actor 对外 contract 仍偏“被动 mailbox”，没有形成清晰的 demand / output 优先状态机。
- 继续沿用 tick/mailbox 模式，会让 decode 推进与 frame arrival、pending output、pacer backpressure 的真实状态脱节，也不利于后续把 decode 作为 pipeline 中的显式 owner 来观测和调参。

## Goal

- 将 decode 推进改为更接近 pull-driven 的状态机，由 decode 侧显式暴露 demand contract。
- 移除 `MediaSessionLoop` 中固定频率的 `decode_drain_tick`，让 ingress -> decode handoff 由 frame arrival 与 decode 状态推进。
- 保持现有 pacer / renderer 接口不变，同时补齐必要的 runtime observation，避免推进链路失压或悬空。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/pipeline/session_loop.rs`
  - `crates/xbxengine/core/src/transport/rtc/pipeline/ingress.rs`
  - `crates/xbxengine/core/src/media/video/decode/actor.rs`
  - `crates/xbxengine/core/src/media/video/decode/video_decode.rs`
  - decode demand contract、ingress handoff 策略、session loop 触发点与相关测试/观测
- Out of scope:
  - pacer / renderer 外部 API 形态调整
  - 新一轮显示策略或 owner policy 重构
  - transport 层 jitter buffer / packet assembly 语义变更

## Plan

1. 为 decode state / actor 增加显式 demand contract，区分“还可接收输入”与“应优先 drain output”。
2. 重写 ingress -> decode handoff，使其基于 decode demand 持续推进，而不是 mailbox slot 计数。
3. 移除 `session_loop` 的固定 decode tick，并用 frame arrival / observation 驱动 handoff 与状态更新。

## Validation

- [ ] `cargo test -p xbxengine media::video::decode -- --nocapture`
- [ ] `cargo test -p xbxengine transport::rtc::pipeline -- --nocapture`
- [ ] `cargo check -p xbxengine`

## Risks

- decode actor 从 slot 驱动切到 demand contract 后，如果状态同步不完整，可能出现 ingress 不再推进或重复推进。
- 去掉固定 tick 后，若 frame arrival 和 decode/pacer backpressure 的边界条件处理不严，可能让 pending output 长时间滞留。
- 现有测试更多覆盖 decode 内部恢复逻辑，pipeline handoff 行为可能还需要补强。

## Progress

- [x] Step 1: 建立独立 RFC，明确本轮 scope 不再混入显示侧 pacing 改造。
- [x] Step 2: 已实现 decode pull-driven contract，`session_loop` 改为 demand 变化驱动，不再依赖固定 decode tick。
- [x] Step 3: 已完成针对性验证并回写 RFC / task 进度。

## Execution Notes

- Date: 2026-04-04 | Status: planned
- Update: 新建 RFC，确认本轮聚焦 decode pull-driven state machine，不复用上一轮 display pacing RFC。
- Decision: `session_loop` 去掉固定 `decode_drain_tick`，改由 decode demand contract 驱动 ingress handoff。
- Risk/Blocker: 当前 worktree 较脏，修改时需要避免覆盖并行任务；`docs/project-task.md` 已被其他工作更新，需按最新内容插入记录。
- Date: 2026-04-04 | Status: in-progress
- Update: 已在 `decode actor / video_decode / pipeline ingress / session_loop` 落地 pull-driven handoff：decode actor 通过 `available_input_slots + pending_output_backpressure + demand_epoch/Notify` 暴露 demand snapshot，`session_loop` 通过 `wait_for_demand_change_since(...)` 等待状态变化，不再保留 4ms `decode_drain_tick`。
- Decision: 这轮 demand 通知采用 `epoch + Notify`，不改动 pacer / renderer 外部接口；`ingress` 仍保留显式 requeue 语义，但停止以 mailbox slot 轮询作为主 contract。
- Risk/Blocker: `transport::rtc::pipeline` 过滤当前没有命中现成测试用例，本轮只能通过编译与 decode 相关回归证明改造闭环；后续适合补 session loop 级别的 pipeline 集成测试。
- Date: 2026-04-04 | Status: validated
- Update: 已运行 `cargo fmt --all`、`cargo test -p xbxengine media::video::decode -- --nocapture`、`cargo test -p xbxengine transport::rtc::pipeline -- --nocapture`、`cargo check -p xbxengine`，均通过。
- Decision: 保留 `transport::rtc::pipeline` 这条命令作为本轮 validation checklist 的一部分，尽管当前命中过滤数为 0；它仍能保证目标模块在 test profile 下完成编译。
- Risk/Blocker: 当前仍有仓库既有 `unused_*` warning，与本轮改造无直接关系，暂不扩大处理范围。
