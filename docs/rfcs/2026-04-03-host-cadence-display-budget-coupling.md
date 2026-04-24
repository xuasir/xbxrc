# 基于宿主 cadence 的显示链预算联动 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 上一轮 [`docs/rfcs/2026-04-03-queue-history-paced-display-supply.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-03-queue-history-paced-display-supply.md) 已完成，把显示侧从单帧 deadline 扩成了队列历史驱动的 pacing，并接回宿主供给压力。
- 现在 `pacer` 仍主要消费 `host_display_interval_ms`、`host_frame_age_budget_ms`、`no_pending_streak` 和 overwrite/drop 比率，但 `video_present_fps`、显示节奏和 present 历史还没有真正进入调度决策。
- 这导致当前显示链虽然能“收敛”，但对“宿主实际 present 跟不上显示 cadence”这类情况还不够敏感，仍偏向观测而不是联动控制。

## Goal

- 让 `pacer` 显式消费宿主 present cadence 事实，把 `video_present_fps` 和 display 侧节奏信号纳入 drop target / 追帧决策。
- 在持续积压时，让显示链更快回到低延迟水位，同时保持短突发的容忍度。
- 保持当前 Rust/Tauri 显示链架构不变，只增强 pacing 策略，不引入新的渲染栈或第二套调度链。

## Scope

- In scope:
  - [`crates/xbxengine/core/src/media/video/render/pacer.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/render/pacer.rs)
  - [`crates/xbxengine/core/src/media/video/pacer/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/pacer/actor.rs)
- Out of scope:
  - [`crates/xbxengine/core/src/transport/rtc/pipeline/session_loop.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/pipeline/session_loop.rs) 的完整 pull-model 改造
  - ingress / decode 的事件驱动重构
  - recovery / BWE 主线策略和阈值调整
  - native video presenter 行为改写
  - 前端 DTO / 展示层字段扩张

## Plan

1. 在 `pacer` 策略层加入对 present cadence 的统一判定，区分短突发和持续失配。
2. 在 `pacer` actor 中把 `video_present_fps` 等宿主事实送入 pacing 决策，并在必要时收紧 sleep / drop 行为。
3. 补齐定向测试，验证 display cadence 失配时会更快收敛，但短突发仍保留弹性。

## Validation

- [x] `cargo fmt --all`
- [x] `cargo test -p xbxengine media::video::render::pacer -- --nocapture`
- [x] `cargo check -p xbxengine`

## Risks

- 如果 present fps 信号短暂抖动就触发过强收紧，可能反而把正常短突发压成不必要丢帧。
- 如果 cadence 联动规则过于复杂，pacer 会再次退化成“多条件拼接”的难维护状态。
- 当前工作树可能还有并行修改，必须限制在显示链 pacing 相关文件内，避免和其他收口任务互相踩踏。

## Progress

- [x] Step 1: 已完成问题边界收敛，确认本轮只做显示链 cadence 联动，不并入 decode pull-model
- [x] Step 2: 已完成 `pacer` 策略与 actor 联动实现
- [x] Step 3: 已完成定向验证并收尾任务跟踪

## Execution Notes

- Date: 2026-04-03 | Status: planned
- Update: 新建本轮实施 RFC，目标是把宿主 present cadence 真正接入显示链 pacing，而不是只做观测。
- Decision: decode pull-model 另起后续 RFC，本轮只做显示链预算与 cadence 联动。
- Risk/Blocker: 需要在不破坏现有 queue history / no-pending 收敛规则的前提下，增加 present fps 相关联动。
- Date: 2026-04-03 | Status: completed
- Update: 已在 `render/pacer.rs` 引入 present/display cadence 感知，`QueueHistoryController` 在 present fps 明显落后 display fps 且持续 backlog 时会更快收紧 drop target；同时 `FramePacingPolicy` 增加 sleep guard 覆盖口，cadence 压力较高时更倾向 `SubmitNow`。
- Decision: 保持 decode pull-model 独立，当前只完成显示链预算与 cadence 联动这一轮。
- Risk/Blocker: 目前策略仍依赖现有 `video_present_fps` / `host_display_interval_ms` 事实，后续如要进一步压低 decode-to-present，再单独立项收口。
- Date: 2026-04-03 | Status: completed
- Update: 已完成格式化与定向验证，`cargo fmt --all`、`cargo test -p xbxengine media::video::render::pacer -- --nocapture`、`cargo check -p xbxengine` 均通过。
- Decision: 这一轮交付已完整收口并产出 Report。
