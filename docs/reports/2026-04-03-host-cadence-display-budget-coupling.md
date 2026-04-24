# 基于宿主 cadence 的显示链预算联动 Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-03-host-cadence-display-budget-coupling.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-03-host-cadence-display-budget-coupling.md)
- 已完成将宿主 present cadence 真正接入显示链 pacing，`video_present_fps` / `display_fps` 不再只是观测值，而是会影响 `pacer` 的 drop target 和 sleep 行为。

## Delivered

- `pacer` 现在会在 display cadence 明显落后且 backlog 持续时更快收紧队列预算。
- `FramePacingPolicy` 增加了 sleep-guard 覆盖口，高 cadence 压力下更容易走 `SubmitNow` 而不是继续等待。
- `pacer` actor 已把 runtime stats 里的宿主 cadence 事实接入 pacing 决策，维持了原有 queue history / no-pending / overwrite 的收敛逻辑。

## Changes

- 在 [`crates/xbxengine/core/src/media/video/render/pacer.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/render/pacer.rs) 中让 `HostPacingPressure` 记录 present/display cadence，`QueueHistoryController` 结合 cadence gap 收紧 drop target，并补齐相关单测。
- 在 [`crates/xbxengine/core/src/media/video/pacer/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/pacer/actor.rs) 中把 `video_present_fps` 和 refresh cadence 送入 pacing 决策，按压力动态缩短 sleep guard。
- 在 [`docs/rfcs/2026-04-03-host-cadence-display-budget-coupling.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-03-host-cadence-display-budget-coupling.md) 和 [`docs/project-task.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/project-task.md) 中完成实施边界与任务收口。

## Validation

- `cargo fmt --all`
- `cargo test -p xbxengine media::video::render::pacer -- --nocapture`
- `cargo test -p xbxengine media::video::pacer -- --nocapture`
- `cargo check -p xbxengine`

## Risks

- present fps 信号如果短时抖动过大，仍有可能把正常短突发压成不必要的收紧。
- 当前 cadence 联动仍主要依赖既有 `video_present_fps` / `host_display_interval_ms` 事实，后续如要继续压低 decode-to-present，需要单独拆解 pull-model 改造。

## Follow-up

- 后续若要继续向 Moonlight 风格演进，建议单独立项做 `decode -> pacer` 的 pending-output / pull-model 收口。
- 如果后续真实 trace 证明 cadence 收紧过头，再回头微调 cadence gap 阈值和 sleep guard 公式。
