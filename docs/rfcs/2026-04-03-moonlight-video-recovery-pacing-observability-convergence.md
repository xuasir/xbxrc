# Moonlight 视频恢复 / Pacing / 观测收口 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 当前视频链路已经借鉴了 Moonlight Qt 的部分思路，但还没有按“恢复态 -> pacing 语义 -> 自愈与观测”顺序完整收口。
- 现有实现里，恢复相关状态仍偏分散，pacer 仍以局部预算与历史控制为主，观测面也还没有把恢复阶段和背压状态完整暴露出来。

## Goal

- 按 1 -> 2 -> 3 的顺序完备落地 Moonlight 风格的视频链路收口。
- 最终结果要做到：
  - 恢复态是显式状态机，而不是若干布尔标志位。
  - pacing 语义有明确队列边界、历史窗口和背压收敛规则。
  - decoder 自愈、重置和观测面可追踪、可回归、可定位。

## Scope

- In scope:
  - `crates/xbxengine/core/src/media/video/decode/video_decode.rs`
  - `crates/xbxengine/core/src/media/video/decode/actor.rs`
  - `crates/xbxengine/core/src/media/video/render/pacer.rs`
  - `crates/xbxengine/core/src/media/video/pacer/actor.rs`
  - `crates/xbxengine/core/src/media/video/types.rs`
  - `crates/xbxengine/core/src/diagnostics/stats.rs`
  - `src-tauri/src/mods/xbxengine/trace_projection.rs`
  - `src/streaming/runtime/xbxengine-runtime.ts`
  - `src/components/stream/StreamDiagnosticsPanel.vue`
  - `docs/project-task.md`
- Out of scope:
  - 不引入新的前端/运行时栈
  - 不改 RTC / transport 主架构
  - 不新增并行媒体管线

## Plan

1. 将恢复态收成显式 FSM，并把关键帧等待、重置和失败升级路径统一起来。
2. 继续固化 pacing 语义，让 decode -> pacer -> render 的队列与历史预算边界更明确。
3. 补齐自愈与观测闭环，把恢复阶段、背压和重置轨迹完整暴露到 stats / trace / UI。

## Validation

- [x] `cargo fmt --all`
- [x] `cargo test -p xbxengine media::video::decode::video_decode -- --nocapture`
- [x] `cargo test -p xbxengine media::video::decode::actor -- --nocapture`
- [x] `cargo test -p xbxengine media::video::pacer::actor -- --nocapture`
- [x] `cargo test -p xbxengine media::video::render::pacer -- --nocapture`
- [x] `cargo check -p xbxengine`
- [x] `pnpm exec vue-tsc --noEmit --pretty false`

## Risks

- 恢复态与现有失败计数、reset 逻辑如果拆得过细，容易引入状态不同步。
- pacing 预算过于激进会放大尾延迟或误丢帧。
- 观测字段增加后，如果前后端 DTO 没同步，容易出现展示层空字段或类型不匹配。

## Progress

- [x] Step 1: 已确认现有恢复状态与入口，并收成显式 FSM。
- [x] Step 2: 已把 pacing 队列 / render 队列 / 历史窗口语义固化。
- [x] Step 3: 已补齐自愈与观测闭环并完成验证。

## Execution Notes

- Date: 2026-04-03 | Status: completed
- Update: 已按 1 -> 2 -> 3 顺序完成 Moonlight 借鉴链路收口，恢复态 FSM、pacer 双队列/历史窗口和 decoder 自愈/观测都已落地。
- Decision: 不再按“最小可用”收口，改为完整落地 Moonlight 借鉴链路。
- Risk/Blocker: 无功能性阻塞；剩余风险主要是与仓库里既有并行改动合并时保持字段与观测口径一致。
