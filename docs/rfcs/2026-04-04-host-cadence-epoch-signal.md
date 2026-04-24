# Host Cadence Epoch Signal RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 当前 pacer 已经会使用 `latest_video_host_present_time_ms + host_display_interval_ms` 做 host release gate，但这仍然是时间窗推断，不是明确的 cadence phase / epoch 信号。
- native presenter 内部已经天然拥有 display tick / present 的单调推进事实，但这份事实没有被完整同步到 runtime stats，也没有成为 engine scheduling 的一等输入。
- 在 host cadence 抖动、长帧、窗口切换等边界场景下，只靠时间窗推断容易出现“漏消费窗口”或“无法区分新 tick 与旧时间窗重复读取”的问题。

## Goal

- 为 host cadence 建立明确的 epoch / phase 信号，并把它同步到 runtime stats。
- 让 pacer gating 优先消费新的 cadence signal；现有时间窗逻辑保留为 fallback。
- 不改变 presenter / renderer 的主职责，只补齐信号表达与调度消费方式。

## Scope

- In scope:
  - `src-tauri/src/mods/native_video/scheduling.rs`
  - `src-tauri/src/mods/native_video/mod.rs`
  - `src-tauri/src/mods/native_video/presenters.rs`
  - `src-tauri/src/mods/xbxengine/runtime_state.rs`
  - `crates/xbxengine/core/src/api/backend.rs`
  - `crates/xbxengine/core/src/runtime_stats_sink.rs`
  - `crates/xbxengine/core/src/media/video/pacer/actor.rs`
  - 相关测试
- Out of scope:
  - decode / ingress / session loop 行为调整
  - runtime host bridge 协议形态大改
  - native presenter 拓扑或线程模型改造

## Plan

1. 在 native host telemetry 中补充 cadence epoch / phase，并同步到 viewport snapshot。
2. 将新的 cadence signal 透传到 runtime stats / host present metrics。
3. 在 pacer actor 中优先消费 cadence epoch / phase，旧时间窗逻辑作为 fallback，并补回归测试。

## Validation

- [x] `cargo test -p xbxengine media::video::pacer -- --nocapture`
- [x] `cargo test -p xbxengine media::video::render::pacer -- --nocapture`
- [x] `cargo test -p xbxengine api::runtime -- --nocapture`
- [x] `cargo check -p xbxengine`
- [x] `cargo check -p xbxrc`

## Risks

- cadence epoch 若同步时机不一致，可能出现 tick / present 两条 epoch 漂移，反而增加歧义。
- 过早让 pacer 强依赖 epoch 可能在 host 反馈缺失时放大 stall，需要明确 fallback。

## Progress

- [x] Step 1: 确认当前 host cadence 只有 time/fps/count，没有显式 epoch / phase 信号。
- [x] Step 2: 实现 host cadence epoch / phase 信号并接到 runtime stats。
- [x] Step 3: 让 pacer 优先消费新信号并完成验证。

## Execution Notes

- Date: 2026-04-04 | Status: completed
- Update: native host telemetry 已把 `display_tick_epoch`、`present_epoch`、`cadence_phase` 写入 viewport snapshot，并经 `runtime_state -> host present metrics -> runtime stats` 贯通到 core。
- Update: pacer actor 已优先消费新的 `display_tick_epoch`，同一个 host tick 不会被重复消费；旧的 `latest_video_host_present_time_ms + host_display_interval_ms` 仅在 epoch 缺失或 host cadence 失活时作为 fallback。
- Update: 补齐 `api::runtime` / `media::video::pacer` / `media::video::render::pacer` 回归，确认显式 cadence signal 与恢复链路兼容。
- Decision: `display_tick_epoch` 作为 pacer release gate 的首选单调信号，`present_epoch` / `cadence_phase` 先用于 runtime stats 对齐与诊断表达。
- Risk/Blocker: 当前 `cadence_phase` 仍主要服务观测与后续策略扩展，尚未直接参与更细粒度 phase-aware gating。
