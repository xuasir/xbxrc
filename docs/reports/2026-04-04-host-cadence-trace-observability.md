# Host Cadence Trace Observability Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-04-host-cadence-trace-observability.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-04-host-cadence-trace-observability.md)
- 已把 host cadence epoch / phase 补进 runtime trace 观测链，后续可以直接用 trace 分析 `Starved -> Steady` 的行为。

## Delivered

- `XbxEngineStatsDto` 现在包含 `host_display_tick_epoch`、`video_present_epoch`、`host_cadence_phase`
- `statsSnapshot / observabilitySnapshot / hostPresentState` 已能输出这些字段
- trace projection 已把这三项纳入 host present 变化判定

## Changes

- [`crates/xbxengine/protocol/src/runtime.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/protocol/src/runtime.rs) 增加 host cadence trace 字段
- [`crates/xbxengine/core/src/diagnostics/stats.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/diagnostics/stats.rs) 投影 runtime stats 到 dto
- [`src-tauri/src/mods/xbxengine/trace_projection.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/xbxengine/trace_projection.rs) 输出 `displayTickEpoch / presentEpoch / cadencePhase`

## Validation

- `cargo test -p xbxengine build_stats_projects_host_cadence_epoch_fields -- --nocapture`
- `cargo test -p xbxrc host_present_state_projects_cadence_epoch_signals -- --nocapture`
- `cargo check -p xbxengine`
- `cargo check -p xbxrc`

## Risks

- 这一步只补齐观测，不替代真实 trace 验证
- 下一轮仍需实际采一份 trace，确认 phase-aware gate / queue pressure 在运行态没有恢复过冲
