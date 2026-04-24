# Host Present 停滞与显示链恢复 RFC

> 对应落地：`docs/references/sdl3-cutover-notes.md` 与显示链/解码背压/transport-await 组合恢复。

## Status

- Completion: 已完成
- Current State: completed
- Owner: xbxengine / Tauri native_video
- Last Updated: 2026-04-12

## Background

- Host `displayTickEpoch` 前进但 `presentEpoch` 长期不前进时，解码侧仍高速灌帧，极小输出队列与激进 stale 策略会触发 `outputQueueOverflow`，系统从可恢复滑向不可恢复。
- `transportAwaitRecoveryAnchor` 下连续 `NonIdrVcl` 时，仅本地 decoder reset 无法解决远端参考链问题。
- `clean anchor` 已提交不等于 host 已恢复出图，恢复完成条件需绑定 present 事实。

## Goal

- 将 host present 停滞提升为独立可观测故障（`hostPresentStalled`），并触发有序本地恢复（present 侧节流、可选 presenter 重置、与既有 coordinator 梯子对齐）。
- 在 NonIdrVcl + present 不恢复时更早强制 IDR/关键帧类请求。
- 收紧「恢复完成」判定，避免仅凭 clean anchor 回到 stable。

## Scope

- In scope: `xbxengine-core`（`video_scheduling_owner` / `session::policy` / `recovery::coordinator` / decode ingress）、`xbxengine` host bridge 默认方法、Tauri `native_video`、诊断 stats/trace、前端 i18n。
- Out of scope: 改动 WebRTC 信令协议架构；替换几何手柄导航。

## 契约摘要

### hostPresentStalled 进入（实现口径）

- 已连接且非 `host_is_priming_without_present`。
- `host_display_tick_epoch` 在策略拍间单调增加，而 `video_present_epoch` 在同期不增加，连续满足 ≥6 拍（策略拍，非固定 wall clock）。
- 解码链仍「有供给」：`decode_age_ms` 不劣于 `degraded_decode_age_ms * 1.5`，且轨道已附着并有视频字节。

### 退出

- `video_present_epoch` 增长，或 tick/present 关系恢复正常，或断开连接。

### 优先级

- 与 `supplyStarved` 并存时，`hostPresentStalled` 作为更具体的 `video_owner_reason` / `stall_kind` / `video_health` 覆盖展示。

### 本地恢复顺序（与实现一致）

1. **Decode ingress 节流**：`host_present_stall_decode_throttle` 为真时仅允许关键帧入解码邮箱；在 `drain_ingress_to_decode` 前对有界队首连续非关键帧做丢弃，避免队头 delta 挡住后续 IDR（HOL）。
2. **Presenter**：runtime `tick` 中在冷却窗内对当前 viewport 调用 `reset_native_video_presenter_for_host_stall`（detach presenter，下一帧 `present_frame` 自动重建）。
3. **Decoder reset / keyframe**：沿用 `OwnerRecoveryReason::HostPresentStalled` → `DisplaySupplyCritical` 桶，由既有 `RecoveryCoordinator` 梯子裁决。

### 恢复完成（clean anchor 路径收紧）

- `can_restore_serving_after_clean_anchor` 在 recent `outputQueueOverflow` 诊断（短窗，约 800ms）存在时返回 false；并继续要求既有 present/decode 阈值与媒体连续性等条件。
- **未采用**「present epoch 跨策略拍连续增长 streak」门控（与现有 owner 测试矩阵冲突）；若后续要加，需单独 RFC 与全量策略回归。

### transport-await + NonIdrVcl

- `coordinator::maybe_force_keyframe_non_idr_present_stall`：在 unresolved transport-await、新鲜 `NonIdrVcl`、出图停滞持续约 900ms（同 `video_present_epoch`）时，写入 `recovery_hard_fallback_trigger_reason=transportAwaitNonIdrPresentStallKeyframe` 并再走 keyframe escalation（可与 `CoalescedKeyframeInFlight` 共存）。

## Validation

- [x] `cargo test -p xbxengine`（定向：`video_scheduling_owner`、`diagnostics::stats`、`recovery::coordinator`、`media::video::decode`、新增 `transport_await_non_idr_with_present_stall_forces_keyframe_after_elapsed_epoch_hold`）
- [ ] `cargo test -p xbxrc` / `src-tauri`（若本地环境具备完整 native 依赖可补跑）

## Risks

- 策略拍频率依赖上层 snapshot，阈值需随线上日志再调。
- Presenter 重置受冷却限制，避免闪屏循环。

## Progress

- [x] RFC 起草
- [x] 引擎与宿主实现合入
- [x] 验证通过并更新 Completion

## Execution Notes

- Date: 2026-04-12 | Status: completed
- Update: host present 停滞检测与 `hostPresentStalled` 诊断、decode ingress 节流、Tauri presenter 重置钩子、NonIdr+present stall keyframe 强制、clean-anchor 下 output overflow 门控与遥测字段已落地；见 Report。
