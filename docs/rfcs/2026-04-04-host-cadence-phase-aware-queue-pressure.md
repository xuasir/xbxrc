# Host Cadence Phase-Aware Queue Pressure RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 上一阶段已经让 `cadence_phase` 进入 pacer 的 release gate 与 sleep guard，但 queue pressure / drop aggressiveness 仍只依赖 no-pending、overwrite 和 cadence lag。
- 这会让 `Priming` 和 `Starved` 两类状态在 backlog 策略上仍然被当作普通 steady cadence 对待。
- 特别是 `Priming` 阶段的低 `present_fps` 容易被误判为 cadence lag，而 `Starved` 阶段又需要更积极地收紧队列以尽快补帧。

## Goal

- 让 `cadence_phase` 直接参与 queue pressure / drop aggressiveness。
- 避免 `Priming` 被低 `present_fps` 误伤。
- 让 `Starved` 在 queue pressure 上直接进入 aggressive 模式。

## Scope

- In scope:
  - `crates/xbxengine/core/src/media/video/render/pacer.rs`
  - `crates/xbxengine/core/src/media/video/pacer/actor.rs`
  - 相关 queue pressure 测试
- Out of scope:
  - native presenter cadence 生成逻辑调整
  - runtime stats 协议扩容
  - release gate / sleep guard 的进一步重构

## Plan

1. 把 `cadence_phase` 纳入 `HostPacingPressure`。
2. 调整 `QueueHistoryController::decide_drop_target`，让 `Priming / Starved` 影响收紧策略。
3. 补回归测试并完成验证。

## Validation

- [x] `cargo test -p xbxengine media::video::render::pacer -- --nocapture`
- [x] `cargo test -p xbxengine media::video::pacer -- --nocapture`
- [x] `cargo check -p xbxengine`
- [x] `cargo check -p xbxrc`

## Risks

- 如果 `Starved` 过于激进，可能会过早丢弃还可用的 backlog。
- 如果 `Priming` 放得太松，可能掩盖真实的早期堆积。

## Progress

- [x] Step 1: 识别 queue pressure 尚未消费 `cadence_phase`。
- [x] Step 2: 完成 `Priming / Starved` 的 phase-aware queue pressure 策略。
- [x] Step 3: 完成验证并回写文档。

## Execution Notes

- Date: 2026-04-04 | Status: completed
- Update: `HostPacingPressure` 已新增 `cadence_phase`，由 pacer actor 从 runtime stats 透传给 queue pressure 决策。
- Update: `QueueHistoryController::decide_drop_target` 现在会对 `Priming` 屏蔽单纯由低 `present_fps` 造成的 cadence lag 收紧；对 `Starved` 直接提升为 aggressive queue pressure。
- Update: 已新增 `queue_history_keeps_relaxed_target_during_priming_without_real_backlog_pressure` 与 `queue_history_tightens_aggressively_when_host_phase_is_starved` 两个回归测试。
- Decision: 这一轮只把 phase 接入 queue pressure 现有判据，不单独新增第三档 drop target。
