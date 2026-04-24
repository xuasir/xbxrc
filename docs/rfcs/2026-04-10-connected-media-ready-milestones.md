# Connected / MediaReady Milestones RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: agent
- Last Updated: 2026-04-10

## Background

- 当前运行时同时暴露 `TransportConnectionStateChanged`、`MediaVideoReady`、`StatsVideoFrameRendered` 等事件，但它们分别代表 peer 连接、协商拿到分辨率、renderer 产生帧，并不等价于统一的“播放成功”。
- 前端和日志层仍容易把 `Connected` 或 `MediaVideoReady` 直接理解为画面成功，导致“已连接但黑屏/等待关键帧”与“画面已经稳定”混淆。

## Goal

- 将「连接成功」与「媒体可稳定展示」拆成两个明确里程碑，并提供统一的引擎判定口径。
- 让协议、Tauri bridge、前端状态与诊断面板都可以直接消费该里程碑，而不是继续拼接单点事件猜状态。

## Scope

- In scope:
  - `crates/xbxengine/protocol/src/runtime.rs`
  - `crates/xbxengine/core/src/api/runtime/*`
  - `crates/xbxengine/core/src/diagnostics/stats.rs`
  - `src-tauri/src/mods/xbxengine/events.rs`
  - `src-tauri/src/shell/bridge.rs`
  - `src/shared/rpc/xbxengine.ts`
  - `src/streaming/runtime/*`
  - `src/streaming/*`
  - `src/i18n/locales/*`
- Out of scope:
  - 全量替换历史 `MediaVideoReady` 事件命名
  - 细化音画同步阈值与音频里程碑

## Plan

1. 在协议层新增呈现里程碑事件与 stats 字段，保留现有 `MediaVideoReady` 作为“协商/尺寸 ready”语义。
2. 在 runtime 内基于 transport、control、track、packet、present freshness 与 stall 信号维护统一里程碑状态机，并输出阶段耗时。
3. 将新里程碑贯通到 Tauri bridge、前端 runtime contract、状态文案与诊断快照，并补充回归测试。

## Validation

- [x] `cargo test -p xbxengine api::runtime -- --nocapture`
- [x] `cargo test -p xbxengine diagnostics::stats -- --nocapture`
- [x] `pnpm exec tsc --noEmit`

## Risks

- `MediaVideoReady` 在现网含义已接近“协商就绪”，如果直接重用其名称做“稳定展示”，会破坏旧消费者。
- 里程碑判定阈值过紧会把短暂 ramp-up 误判成 degraded，阈值过松又会延后真实告警。

## Progress

- [x] Step 1: 明确里程碑语义与兼容策略，新增 RFC。
- [x] Step 2: 实现 runtime / protocol / bridge 的统一里程碑输出。
- [x] Step 3: 前端状态、文案、诊断与测试收口。

## Execution Notes

- Date: 2026-04-10 | Status: completed
- Update: 已完成协议、runtime、bridge、前端状态与文案联调；验证通过 runtime/diagnostics Rust 测试与 TypeScript 类型检查。
- Decision: 新增独立 `presentationMilestone` 事件与 stats 字段，而不是篡改现有 `TransportConnectionStateChanged` / `MediaVideoReady` 语义。
- Risk/Blocker: 首版仍以视频稳定展示为主，音频 playout / AV sync 作为后续增强项。
