# runtime-trace-1775106411793 视频卡死 follow-up Report

## Summary

- Related RFC: [`docs/rfcs/2026-04-02-runtime-trace-1775106411793-video-freeze-followup.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-02-runtime-trace-1775106411793-video-freeze-followup.md)
- 本次已完整收口 trace 暴露的“拉起后卡死”问题：owner 侧的 critical `noPending` + `waitKeyframe` 不再被压成纯 `SupplyStarved`，coordinator 侧的 `explicitHealthyCleanAnchor` 也不再把 hard-fallback 内部起点彻底清零。

## Delivered

- 修正 `VideoSchedulingOwner` 对 `critical noPending + waitKeyframe` 的判定，优先回到 `RebuildingSupply`。
- 修正 `RecoveryCoordinator` 的 clean anchor 语义，保留 hard-fallback 内部起点，避免短暂健康帧把坏窗从头打散。
- 补齐并通过对应回归测试与 `transport_await_` 全量验证。

## Changes

- 在 [`crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs) 增加 waitKeyframe 优先重建分支，避免 `clean anchor + chain healthy` 误压掉明确的 recovery noise。
- 在 [`crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 调整 `acknowledge_clean_anchor()` / `reset_transport_await_hard_fallback()`，只清 runtime stats 可见字段，不清内部 hard-fallback 起点。
- 更新 [`docs/project-task.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/project-task.md) 和对应 RFC，确保追踪状态与最终语义一致。

## Validation

- `cargo fmt --all`
- `cargo test -p xbxengine transport_await_ -- --nocapture`
- `cargo test -p xbxengine critical_wait_keyframe_noise_prefers_rebuilding_over_supply_starved_even_with_clean_anchor -- --nocapture`
- `cargo test -p xbxengine connected_lingering_no_pending_with_clean_anchor_can_return_to_stable_serving -- --nocapture`
- `cargo test -p xbxengine transport_await_hard_fallback_uses_connected_ingress_when_decoder_reset_path_exhausted -- --nocapture`
- `cargo test -p xbxengine transport_await_hard_fallback_timer_resets_on_healthy_clean_anchor -- --nocapture`
- `cargo check -p xbxengine`

## Risks

- 保留 hard-fallback 内部起点后，同一 recovery epoch 内如果链路短暂恢复又快速退化，可能更快触发 reconnect 候选。
- 如果后续 trace 继续表现出抖动型误触发，再给“持续健康”补一层更严格的 disarm 条件。

## Follow-up

- 用下一份真实 runtime trace 验证 `RebuildingSupply` 与 hard-fallback reconnect 的节奏是否仍然过激。
- 如需要，再补一个“持续健康后真正 disarm hard-fallback”的条件，避免抖动链路频繁重连。
