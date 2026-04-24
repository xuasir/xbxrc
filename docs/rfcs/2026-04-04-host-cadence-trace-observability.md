# Host Cadence Trace Observability RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 目前 runtime stats 已经有 `host_display_tick_epoch`、`video_present_epoch`、`host_cadence_phase`。
- 但 runtime trace 里的 `statsSnapshot / observabilitySnapshot / hostPresentState` 还没有投影这些字段。
- 结果是我们无法用真实 trace 直接验证 `Starved -> Steady` 恢复段的 phase-aware gate / queue pressure 是否按预期工作。

## Goal

- 让 trace 直接包含 host cadence epoch / phase 关键信号。
- 让后续 runtime trace 分析可以直接观察 `display_tick_epoch`、`present_epoch`、`cadence_phase` 的推进。

## Scope

- In scope:
  - `crates/xbxengine/protocol/src/runtime.rs`
  - `crates/xbxengine/core/src/diagnostics/stats.rs`
  - `src-tauri/src/mods/xbxengine/trace_projection.rs`
  - 相关 diagnostics / trace projection 测试
- Out of scope:
  - runtime core scheduling 行为改造
  - 新 trace 文件采集与人工分析结论

## Plan

1. 把 host cadence epoch / phase 从 runtime stats 投影进 `XbxEngineStatsDto`。
2. 把这些字段写入 `observabilitySnapshot.video` 与 `hostPresentState`。
3. 补回归测试并完成验证。

## Validation

- [x] `cargo test -p xbxengine build_stats_projects_host_cadence_epoch_fields -- --nocapture`
- [x] `cargo test -p xbxrc host_present_state_projects_cadence_epoch_signals -- --nocapture`
- [x] `cargo check -p xbxengine`
- [x] `cargo check -p xbxrc`

## Risks

- trace 字段补齐后，后续如果 host cadence 命名再变化，需要同步维护 diagnostics / projection 层。
- 这一步只补可观测性，不直接证明调度行为正确，还需要下一轮采真实 trace。

## Progress

- [x] Step 1: 确认 trace 现状无法直接观测 host cadence epoch / phase。
- [x] Step 2: 完成 stats dto 与 trace projection 字段补齐。
- [x] Step 3: 完成验证并回写文档。

## Execution Notes

- Date: 2026-04-04 | Status: completed
- Update: `XbxEngineStatsDto` 已新增 `host_display_tick_epoch`、`video_present_epoch`、`host_cadence_phase`。
- Update: `diagnostics/stats.rs` 已把 runtime stats 中的 host cadence epoch / phase 投影进 dto。
- Update: `trace_projection.rs` 已把三项字段写进 `observabilitySnapshot.video` 与 `hostPresentState`，并纳入 host present 观测去重判定。
- Update: 已新增 `build_stats_projects_host_cadence_epoch_fields` 与 `host_present_state_projects_cadence_epoch_signals` 回归测试。
