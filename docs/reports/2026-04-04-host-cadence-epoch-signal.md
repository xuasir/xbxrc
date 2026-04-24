# Host Cadence Epoch Signal Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-04-host-cadence-epoch-signal.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-04-host-cadence-epoch-signal.md)
- 已完成 host cadence 显式 epoch / phase 信号贯通，并让 pacer 优先消费新的 cadence signal，旧时间窗逻辑退居 fallback。

## Delivered

- native host telemetry 现在显式维护 `display_tick_epoch`、`present_epoch`、`cadence_phase`
- viewport snapshot / runtime host feedback / core runtime stats 已透传新的 cadence signal
- pacer 已优先按新的 `display_tick_epoch` 开窗 release，并补齐调度链路回归测试

## Changes

- [`src-tauri/src/mods/native_video/mod.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/native_video/mod.rs) 与 [`src-tauri/src/mods/native_video/presenters.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/native_video/presenters.rs) 将 host cadence epoch / phase 收口到 viewport diagnostics
- [`src-tauri/src/mods/xbxengine/runtime_state.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/xbxengine/runtime_state.rs)、[`crates/xbxengine/core/src/api/backend.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/backend.rs)、[`crates/xbxengine/core/src/transport/rtc/stack/runtime_port.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack/runtime_port.rs) 已把新字段同步进 runtime stats
- [`crates/xbxengine/core/src/media/video/pacer/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/pacer/actor.rs) 以 `display_tick_epoch` 作为优先 gate，避免重复消费同一 host tick，并保留时间窗 fallback

## Validation

- `cargo test -p xbxengine media::video::pacer -- --nocapture`
- `cargo test -p xbxengine media::video::render::pacer -- --nocapture`
- `cargo test -p xbxengine api::runtime -- --nocapture`
- `cargo check -p xbxengine`
- `cargo check -p xbxrc`

## Risks

- `present_epoch` / `cadence_phase` 目前已入 stats，但主要用于观测和后续扩展，还没有成为 phase-aware policy 的直接决策输入
- 现有验证覆盖了 host gate 与 runtime recovery 主链路，但真实窗口切换、前后台切换下的 cadence phase 漂移仍需继续靠运行态 trace 观察

## Follow-up

- 继续评估是否让 `cadence_phase` 直接参与 host pressure / release gating 的细分策略
- 结合真实 runtime trace 观察 `display_tick_epoch` 与 `present_epoch` 在窗口抖动和 starvation 下的推进形态
