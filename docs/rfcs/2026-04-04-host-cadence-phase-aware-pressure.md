# Host Cadence Phase-Aware Pressure RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 当前 pacer 已经让 `cadence_phase` 参与 release gate 与 sleep guard。
- 但 queue pressure / drop aggressiveness 仍然只看 `no_pending` 压力、overwrite 比例、cadence lag 和 backlog 历史，没有把 `Priming / Starved` 本身作为一等输入。
- 这会导致 `Priming` 阶段的低 `present_fps` 被误判成严重 lag，也会让 `Starved` 虽然在 gate 上放宽补帧，但 queue budget 不一定同步收紧。

## Goal

- 让 `cadence_phase` 继续进入 queue pressure / drop aggressiveness 决策。
- 在 `Priming` 时抑制由未稳定 present cadence 引起的误 tightening。
- 在 `Starved` 时让 queue budget 更快进入 aggressive pressure。

## Scope

- In scope:
  - `crates/xbxengine/core/src/media/video/render/pacer.rs`
  - `crates/xbxengine/core/src/media/video/pacer/actor.rs`
  - 相关 pacer / render pacer 测试
- Out of scope:
  - host telemetry phase 生成逻辑调整
  - runtime stats 新字段扩容
  - decode / recovery 策略调整

## Plan

1. 把 `cadence_phase` 提升成 queue pressure 可直接消费的强类型输入。
2. 在 `QueueHistoryController::decide_drop_target` 中接入 `Priming / Starved` 特例。
3. 补回归测试并完成文档回写。

## Validation

- [x] `cargo test -p xbxengine media::video::pacer -- --nocapture`
- [x] `cargo test -p xbxengine media::video::render::pacer -- --nocapture`
- [x] `cargo check -p xbxengine`
- [x] `cargo check -p xbxrc`

## Risks

- 若 `Priming` 放松过头，可能掩盖真实 backlog 压力。
- 若 `Starved` 收紧过快，可能导致丢帧更激进，需要确认不会和补帧优先策略互相抵消。

## Progress

- [x] Step 1: 明确当前 queue pressure 仍未消费 `cadence_phase`。
- [x] Step 2: 实现 phase-aware drop aggressiveness。
- [x] Step 3: 完成验证并回写文档。

## Execution Notes

- Date: 2026-04-04 | Status: completed
- Update: 本轮目标聚焦到 queue pressure / drop aggressiveness，不再扩大到其他恢复链路。
- Update: `crates/xbxengine/core/src/media/video/render/pacer.rs` 已把 `cadence_phase` 从 `Option<String>` 收口为 `HostCadencePhaseHint`，queue pressure 不再散落字符串分支。
- Update: `QueueHistoryController::decide_drop_target` 已接入 phase-aware 语义：`Priming` 会抑制由低 `present_fps` 导致的 cadence lag tightening，`Starved` 会直接进入 aggressive queue pressure。
- Update: 已补并通过 render/pacer 回归：`queue_history_keeps_relaxed_target_during_priming_without_real_backlog_pressure`、`queue_history_tightens_aggressively_when_host_phase_is_starved`。
- Decision: `Priming` 只豁免“由 cadence lag 推出来的 tightening”，不会绕过 sustained backlog / overwrite / no-pending 压力；`Starved` 则直接提升 aggressive 等级。
- Risk/Blocker: phase-aware pressure 目前仍集中在 queue drop target，尚未继续参与更细的 drop detail 分类或 trace 观测聚合。
