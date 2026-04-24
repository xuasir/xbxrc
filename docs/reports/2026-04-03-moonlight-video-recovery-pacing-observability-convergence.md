# Moonlight 视频恢复 / Pacing / 观测收口 Report

## Summary

- Related RFC: [`docs/rfcs/2026-04-03-moonlight-video-recovery-pacing-observability-convergence.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-03-moonlight-video-recovery-pacing-observability-convergence.md)
- 本轮按 1 -> 2 -> 3 顺序完整落地了 Moonlight 借鉴链路：恢复态 FSM、pacer 双队列/历史窗口和 decoder 自愈/观测闭环都已经进入主线实现。

## Delivered

- 恢复态从布尔/计数组合收成显式 FSM，并把 reset、bootstrap keyframe 恢复、恢复收敛和失败升级统一进 `video_decode.rs`。
- pacer 主循环增加 render 队列与短背压重试，配合时间窗口化的 queue history，形成更接近 Moonlight 的双层调度语义。
- decoder/pacer 的恢复状态、事件和背压观察已经投到 stats、trace projection、runtime snapshot、前端性能面板和诊断面板。

## Changes

- `crates/xbxengine/core/src/media/video/decode/video_decode.rs` 新增恢复状态枚举、状态转移快照和失败升级逻辑。
- `crates/xbxengine/core/src/media/video/decode/actor.rs`、`crates/xbxengine/core/src/api/backend.rs`、`crates/xbxengine/core/src/diagnostics/stats.rs`、`crates/xbxengine/protocol/src/runtime.rs` 串起恢复状态的运行态投影。
- `crates/xbxengine/core/src/media/video/pacer/actor.rs`、`crates/xbxengine/core/src/media/video/render/pacer.rs` 固化了 pacing/render 双队列和 500ms 历史窗口。
- `src-tauri/src/mods/xbxengine/trace_projection.rs`、`src/streaming/runtime/xbxengine-runtime.ts`、`src/streaming/diagnostics.ts`、`src/components/stream/StreamPerformancePanel.vue`、`src/components/stream/StreamDiagnosticsPanel.vue`、`src/i18n/locales/en.json`、`src/i18n/locales/zh.json` 补齐了前端和 trace 可见性。

## Validation

- `cargo fmt --all`
- `cargo test -p xbxengine media::video::decode::video_decode -- --nocapture`
- `cargo test -p xbxengine media::video::decode::actor -- --nocapture`
- `cargo test -p xbxengine media::video::pacer::actor -- --nocapture`
- `cargo test -p xbxengine media::video::render::pacer -- --nocapture`
- `cargo check -p xbxengine`
- `pnpm exec vue-tsc --noEmit --pretty false`

## Risks

- 目前还保留仓库里其他并行改动，后续合并时需要继续留意 stats / RPC 字段是否保持同口径。
- render 背压现在已经不再直接丢帧，但若实际设备侧长期背压，仍可能触发更激进的 queue 压缩策略，需要后续运行态继续观察。

## Follow-up

- 继续用真实运行 trace 观察恢复态转换、render 背压和 queue history 是否符合预期。
- 如果后续需要更细的性能诊断，再考虑把 render queue depth / backpressure 持续时间补成独立指标。
