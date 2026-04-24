# NACK Value Semantics And High-Value Preservation Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-12-nack-value-semantics-and-high-value-preservation.md`](../rfcs/2026-04-12-nack-value-semantics-and-high-value-preservation.md)
- 已完成 NACK 准入层语义纠偏：低价值放弃与参考链断裂证据分离；高价值 supply/reference 在 steady/recovery 下的 near-deadline 策略与恢复触发白名单对齐 RFC。

## Delivered

- `nack.rs`：`with_cloud_latency_admission_policy` 低价值路径恒为 `SkippedLowValue`；`sample_loss` 仅 supply  tier 可抬 `SkippedChainBroken`；supply near-deadline 在 steady 保持尝试、在 recovery 记 `SkippedTooLate` + `estimatedArrivalNearDeadlineSupplyRecovery`；`maybe_handle_chain_broken` 按 disposition 与 `nack_reference_chain_recovery_evidence` 白名单收敛触发。
- `timeline.rs`：`is_local_low_value_gap_reason` 纳入 `estimatedArrivalNearDeadlineLowValue`。
- `backend.rs`：anchor candidate failure 枚举替换误名 `ChainBroken*` 为 `TransportLowValue*` 与 `TransportTimingNearDeadlineSupplyRecovery`，`as_str` 保持历史键兼容。
- 单测：`nack` / `nack_scheduler` / 新增 timeline 用例；RFC Validation/Progress 已勾选。

## Changes

- [`crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs`](../../crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs)
- [`crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.rs`](../../crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.rs)
- [`crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.test.rs`](../../crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.test.rs)
- [`crates/xbxengine/core/src/api/backend.rs`](../../crates/xbxengine/core/src/api/backend.rs)

## Validation

- `cargo test -p xbxengine transport::rtc::stream::video_source::nack`
- `cargo test -p xbxengine nack_scheduler`
- `cargo test -p xbxengine estimated_arrival_near_deadline_low_value`

## Risks

- 白名单过严时，少数依赖旧「near-deadline recovery supply → chain broken」路径的边缘场景可能更晚进入 keyframe 恢复；需靠 timeline 真坏链与 supply 低 repairability 路径兜底。
- steady 下 supply 先 `Attempted` 可能略增单次 gap 上的 NACK 尝试，依赖 scheduler deadline/max_age/retry 收口。

## Follow-up

- 若线上 trace 显示 steady supply 补包偏多，可再评估按 `refresh_boost` 微调 `SUPPLY_NEAR_DEADLINE_GUARD_MS` 或记录「首次 near-deadline 仍尝试」的观测计数。
- `budget.rs` 内 reason 前缀文档化（`lowValue:` / `timing:` / `referenceChain:`）可作为后续可读性小补。
