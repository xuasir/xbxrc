# 统一云游戏 BWE/码率观测口径并修复错误显示 Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-03-24-cloud-bwe-bitrate-observability-unification.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-24-cloud-bwe-bitrate-observability-unification.md)
- 本任务已完成 Rust 观测聚合、协议 DTO、trace 投影、前端运行时映射与性能面板展示的统一收口，修复了“总下行/BWE 显示语义错误”问题。

## Delivered

- 将 `inbound_bitrate_kbps` 统一为“总下行码率（video + audio）”口径，并补充稳定兜底推导。
- 新增显式 `video_bwe_*` 与 `video_twcc_*` 字段，避免继续复用含义模糊的旧字段。
- 在 trace snapshot 与前端性能面板中同步展示 `BWE Target / Observed REMB / Actual Video / TWCC Recv/Loss/Delivery`。

## Changes

- `crates/xbxengine/core/src/diagnostics/stats.rs` 改为优先按 video+audio 组件推导 total bitrate，并把 BWE/TWCC 结构化观测投影到 `XbxEngineStatsDto`。
- `crates/xbxengine/protocol/src/runtime.rs`、`src/shared/rpc/xbxengine.ts`、`src/streaming/runtime/xbxengine-runtime.ts`、`src/player/domain/media.ts`、`src/streaming/types.ts` 完成新增字段的跨层透传与命名对齐。
- `src-tauri/src/mods/xbxengine/trace_projection.rs` 与 `src/components/stream/StreamPerformancePanel.vue` 改为输出/展示显式语义字段，中英文文案同步更新。

## Validation

- `cargo test -p xbxengine session::policy --lib`
- `cargo test -p xbxengine transport::rtc::stack::transport_session --lib`
- `cargo test -p xbxengine diagnostics::stats --lib`
- `cargo check -p xbxengine`
- `cargo check -p xbxrc`

## Risks

- `br` / `video_remb_bps` 仍作为兼容字段保留，外部若继续只消费旧字段，仍可能忽略新增语义字段；后续新增面板/脚本时应优先使用显式字段。
- TWCC/BWE/bitrate 来自不同采样周期，短时仍可能出现数值不同步，但现在是时间基线差异，不再是字段语义错误。

## Follow-up

- 若后续扩展更完整的性能诊断页或图表组件，继续以本次定义的 total/video/audio/BWE/TWCC 口径为唯一语义来源。
- 后续复盘真实 trace 时，优先读取 `bitrate.totalKbps`、`bwe.*`、`twcc.*`，逐步减少对兼容字段的依赖。
