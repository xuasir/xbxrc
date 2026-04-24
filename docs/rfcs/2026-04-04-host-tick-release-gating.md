# Host Tick Release Gating RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 当前 decode handoff 已经改为 pull-driven，但 release / present 链路仍未形成“host tick 直接参与 release eligibility”的闭环。
- 宿主 native presenter 已经在 display tick 上消费 `ScheduledFrameSlot`，并持续上报 `host_display_interval_ms`、`host_frame_age_budget_ms`、`latest_video_host_present_time_ms`。
- 但 engine 内 `pacer` 仍主要依据 `frame.pts`、catch-up 和 queue pressure 做 release 决策，host tick 只作为 budget/pressure 的旁路输入，没有直接卡住“过早 release”。

## Goal

- 让 host tick / 最近 host present cadence 直接参与 pacer release 时机判定。
- 保持现有 presenter / renderer / runtime 接口不变，只把 host cadence 收口为 pacer 的 release gate。
- 避免 host present 节奏暂时缺失时把 pipeline 锁死，保留超时退化路径。

## Scope

- In scope:
  - `crates/xbxengine/core/src/media/video/render/pacer.rs`
  - `crates/xbxengine/core/src/media/video/pacer/actor.rs`
  - `docs/project-task.md`
  - 本 RFC 的执行进度记录
- Out of scope:
  - native presenter 的 `ScheduledFrameSlot` / display link 主体逻辑重构
  - runtime tick / host bridge 协议形态调整
  - decode / renderer actor 接口变更

## Plan

1. 在 pacer policy 中补充 host release gate，让 `SubmitNow/Sleep` 能显式受 host cadence 窗口约束。
2. 在 pacer actor 中用 `latest_video_host_present_time_ms + host_display_interval_ms` 计算 gate wait，并在 host cadence 失活时安全退化。
3. 补齐回归测试，验证“host tick 未到时不提前 release、host tick 失活时不锁死”。

## Validation

- [ ] `cargo test -p xbxengine media::video::render::pacer -- --nocapture`
- [ ] `cargo test -p xbxengine media::video::pacer -- --nocapture`
- [ ] `cargo check -p xbxengine`

## Risks

- 如果 gate 过于刚性，host cadence 短时缺样可能让 pacer 额外等待，放大 backlog。
- 如果 gate 退化条件过松，又可能重新回到“host tick 只做旁路观测”的状态。

## Progress

- [x] Step 1: 确认当前 host tick 只直接影响 native presenter `take_ready_frame()`，还未进入 engine pacer 的 release eligibility。
- [x] Step 2: 已实现 host tick release gate，并收口到 pacer 主路径。
- [x] Step 3: 已完成验证并回写结果。

## Execution Notes

- Date: 2026-04-04 | Status: planned
- Update: 新建 RFC，收口“host tick 直接参与 release 时机”的独立改造范围。
- Decision: 优先把 host cadence 直接并入 pacer 的 release gate，不扩大到 runtime tick 或 native presenter 主结构改造。
- Risk/Blocker: 需要避免把 host cadence 缺样误判成强制等待，否则会把 pacer 锁死。
- Date: 2026-04-04 | Status: in-progress
- Update: 已在 `FramePacingPolicy` 增加 host release gate 输入；`pacer actor` 现使用 `latest_video_host_present_time_ms + host_display_interval_ms` 计算下一次 host release 窗口，并把等待并入 `next_wait_duration()` 与 `drive_ready_frames()`。
- Decision: 本轮先不引入新的 host tick epoch 协议，而是复用现有 host present metrics 做 release gating；当 host cadence 超过 `2.5x interval` 未更新时自动退化为不 gating，避免锁死 pacer。
- Risk/Blocker: 这轮属于“用 host present 窗口直接卡 release eligibility”，不是跨层单调 tick epoch；如果后续还要精确对齐 display-link tick 边界，再补 epoch 信号会更稳。
- Date: 2026-04-04 | Status: validated
- Update: 已运行 `cargo fmt --all`、`cargo test -p xbxengine media::video::render::pacer -- --nocapture`、`cargo test -p xbxengine media::video::pacer -- --nocapture`、`cargo check -p xbxengine`，均通过。
- Decision: 验证重点放在 pacer policy 和 pacer actor 单测，确保“host tick 未到时不提前 release、host cadence 失活时不持续等待”。
- Risk/Blocker: 当前还没有真实 trace 复核，本轮只能确认代码路径和单测闭环，下一步适合用新 trace 看 `videoPacerSubmitCountTotal / videoPresentFps / hostNoPendingStreak` 是否同步改善。
