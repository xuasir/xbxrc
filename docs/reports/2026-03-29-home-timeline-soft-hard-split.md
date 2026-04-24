# Home Timeline Soft/Hard Split Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-03-27-timeline-value-runtime-upgrade.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-27-timeline-value-runtime-upgrade.md)
- 已完成 Home 场景 timeline 的软/硬恢复分层与小预算收口，clean anchor 短窗内的 delta gap 重入不再直接把链路打回恢复态。

## Delivered

- clean anchor 后的短窗 delta gap 重入现在优先留在 `Healthy`，避免恢复债务被短抖动反复点燃。
- 软重入预算耗尽或保护窗过期后，timeline 自动恢复原来的硬破链语义。
- reference/keyframe 级别的硬破链保持不变。

## Changes

- [`crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.rs) 新增 clean-anchor 软重入保护窗与预算。
- `mark_gap_reorder_pending`、`mark_gap_nack_candidate`、`mark_gap_repair_in_flight`、`mark_gap_resolved`、`mark_gap_expired` 统一接入软/硬分层。
- 新增单测覆盖软重入、预算耗尽与 reference 硬破链不变。

## Validation

- `cargo test -p xbxengine transport::rtc::stream::video_source::timeline -- --nocapture`

## Risks

- 仍需要继续用最新 Home/Cloud trace 验证，确认 `stable-serving` 是否真正获得足够长的连续稳定窗。
- 如果后续 trace 仍出现大量 `referenceChainUnrecoverable`，还需要继续审视上游 gap 生成与 recovery coordinator 的重入节奏。

## Follow-up

- 继续拿最新 Home trace 看 `stable-serving` 是否能站住。
- 若仍反复回到 `rebuilding-supply`，下一步再收 `recovery/coordinator.rs` 的重入节流与预算上限。
