# Host Cadence Phase-Aware Gating RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 当前 `display_tick_epoch` 已经贯通到 pacer，并成为 host release gate 的首选信号。
- `present_epoch` 与 `cadence_phase` 虽然已经入 `runtime stats`，但目前主要用于诊断表达，没有直接参与 pacer 决策。
- 这会导致 pacer 仍然只能从“有没有新 tick / 时间窗是否到期”推断 host 状态，无法针对 `Priming / Steady / Starved` 做更细粒度的开窗与 pressure 策略。

## Goal

- 让 `cadence_phase` 成为 pacer 的直接输入，而不只是诊断字段。
- 在 `Priming / Steady / Starved` 三类 host 状态下，为 release gate 与 sleep guard 提供不同策略。
- 保持 `display_tick_epoch` 为首要单调开窗信号，避免 phase-aware 逻辑破坏已有 epoch-first 行为。

## Scope

- In scope:
  - `crates/xbxengine/core/src/media/video/pacer/actor.rs`
  - 相关 pacer / render pacer 测试
  - `docs/project-task.md`
- Out of scope:
  - native presenter phase 生成逻辑重写
  - `runtime stats` 协议字段再扩容
  - decode / ingress / session loop 的恢复策略调整

## Plan

1. 收口 pacer 内部对 `cadence_phase` 的解析表示，避免直接散落消费字符串。
2. 让 host release gate 与 cadence sleep guard 根据 phase 调整行为。
3. 补 phase-aware 回归测试，并完成文档回写。

## Validation

- [x] `cargo test -p xbxengine media::video::pacer -- --nocapture`
- [x] `cargo test -p xbxengine media::video::render::pacer -- --nocapture`
- [x] `cargo check -p xbxengine`
- [x] `cargo check -p xbxrc`

## Risks

- phase 语义若过度耦合当前 host telemetry 细节，后续 presenter 侧调相会影响 pacer 行为稳定性。
- `Starved` 下若策略过于激进，可能放大重复 submit / backpressure，而不是缓解 host 压力。

## Progress

- [x] Step 1: 定位 `cadence_phase` 已贯通但仍未进入 pacer 直接决策。
- [x] Step 2: 实现 phase-aware gating / pressure 策略。
- [x] Step 3: 完成验证并回写 RFC / task。

## Execution Notes

- Date: 2026-04-04 | Status: completed
- Update: `crates/xbxengine/core/src/media/video/pacer/actor.rs` 已把 `cadence_phase` 收口成 `HostCadencePhaseHint`，不再在 pacer 决策里散落消费字符串。
- Update: `resolve_host_release_wait_duration` 已加入 phase-aware 语义：`Priming` 且尚无首个 present 时，同一 tick 内会维持半帧等待；`Starved` 下允许同一 tick 直接放行补帧；`Steady` 仍保持 epoch-first + 时间窗 fallback。
- Update: `resolve_cadence_sleep_guard_override_ms` 已直接消费 phase：`Starved` 关闭长睡眠保护、`Priming` 收紧 sleep guard，避免 host 尚未进入稳定 cadence 时过长阻塞 pacer。
- Update: 已补并通过 phase-aware 回归：`host_release_gate_blocks_reusing_same_priming_tick_before_first_present`、`host_release_gate_releases_same_tick_immediately_when_host_is_starved`、`cadence_sleep_guard_override_shortens_sleep_during_priming`、`cadence_sleep_guard_override_disables_sleep_when_host_is_starved`。
- Decision: phase-aware 逻辑只调节“同一 tick 内是否继续等、是否允许更激进 submit、是否缩短 sleep guard”，不改变 `display_tick_epoch` 作为首要 release gate 的位置。
- Risk/Blocker: 当前 phase-aware 策略仍集中在 pacer 本地；`cadence_phase` 还没有进一步参与 queue history/drop target 等更细的 pressure 分层。
