# Host Present 停滞与显示链恢复 Report

> 复杂任务收尾总结；过程跟踪见 RFC。

## Summary

- Related RFC: [`docs/rfcs/rfc-2026-04-12-host-present-stall-recovery.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/rfc-2026-04-12-host-present-stall-recovery.md)
- 将「tick 前进、present 卡住」提升为独立故障 `hostPresentStalled`，串联 decode 背压、本地 presenter 重置与 transport-await 下 NonIdr+出图停滞的 keyframe 强化，并收紧 clean anchor 恢复门控中的输出队列溢出条件。

## Delivered

- `VideoSchedulingOwner`：`HostPresentStallTracker`、`reason_label=hostPresentStalled`、与 `supply-starved` 组合的 stats/stall 映射。
- `session::policy`：`host_present_stall_decode_throttle`；控制面 `DisplaySupplyCritical` 与 escalation 梯子对齐。
- Decode：`ingress`/`scheduler` 在节流时仅关键帧入队；decode 队列容量与 stale 窗口适度放宽（配合背压）。
- `RecoveryCoordinator::maybe_force_keyframe_non_idr_present_stall` 与 stats 标记 `transportAwaitNonIdrPresentStallKeyframe`；回归测试 `transport_await_non_idr_with_present_stall_forces_keyframe_after_elapsed_epoch_hold`。
- Runtime：`reset_native_video_presenter_for_host_stall`、lifecycle 冷却调用；`sync` / protocol / trace / 前端 i18n 字段。
- `can_restore_serving_after_clean_anchor`：近期 `outputQueueOverflow` 时禁止借 clean anchor 过早回到 Ready（未引入 present-epoch streak，见 RFC 说明）。

## Changes

- 核心：`video_scheduling_owner`、`display_supply` 相关输入、`stats`、`coordinator`、`ingress`、`video_decode`/`decode actor`、`api/runtime`、`src-tauri` `native_video` / `runtime_state` / `trace_projection`、`xbxengine/protocol` runtime DTO、i18n JSON。

## Validation

- `cargo test -p xbxengine transport_await_non_idr_with_present_stall_forces_keyframe_after_elapsed_epoch_hold -- --nocapture`
- 建议全量：`cargo test -p xbxengine`（CI/本地完整环境）

## Risks

- 策略拍频率与阈值依赖上层 snapshot，线上需用 trace 再调。
- Presenter 重置受冷却约束，极端情况下仍可能需远端 IDR。

## Follow-up

- 若要坚持「present epoch streak」类完成条件，需独立 RFC 与 owner/policy 全矩阵回归。
- 在具备完整 native 依赖的环境中补跑 `cargo test -p xbxrc` / `src-tauri`。
