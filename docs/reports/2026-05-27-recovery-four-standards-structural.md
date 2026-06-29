# Report：恢复链四标准结构性收口

**日期：** 2026-05-27
**RFC：** [`2026-05-26-receive-insert-gate-and-recovery-convergence.md`](../rfcs/2026-05-26-receive-insert-gate-and-recovery-convergence.md)

## 目标

在 2026-05-27 四问题补丁之上，按四条验收标准做结构性收口（非事后 remap）：

1. **单轨** — Insert / Owner 只认 `RecoveryContractSnapshot` 与 `decodable_to_feed`
2. **短链** — Owner 信号 → `DelegatedToReceive` → receive PLI，不经 session keyframe epoch
3. **稀疏 IDR 独立节奏** — `SparseIdrRhythm` 与 session budget 解耦
4. **session 不污染 keyframe in-flight** — escalation 不产出可执行 `RequestPli`/`RequestFir`

## 变更摘要

| 区域 | 文件 | 内容 |
|------|------|------|
| Escalation | `recovery/escalation.rs` | `delegate_picture_recovery_to_receive`；图片级 PLI/FIR/coalesce → `DelegatedToReceive`，不 enter epoch |
| Coordinator | `recovery/coordinator.rs` | sync 前 suppress；`receive_local` 时 `CoalescedKeyframeInFlight` no-op；`recovery_picture_recovery_authority` |
| Policy | `session/policy.rs` | 首帧 coalesce → `DelegatedToReceive` + `recovery_receive_keyframe_hint_at_ms` |
| Insert | `receive/insert_gate.rs` | 首帧仅 fresh/bootstrap IDR；去掉生产 `resolve_inspection_admission` |
| Owner | `policy/video_scheduling_owner.rs` | 删除 `displayed_idr_serving_wide`，统一 `contract_snapshot.serving_wide` |
| 稀疏 IDR | `contract.rs` + receive | `SparseIdrRhythm`、`receive_keyframe_last_sent_at_ms`；NACK 加速条件收紧 |
| 可证伪 | `api/backend.rs`、`decode/actor.rs` | `insert_hold_decode_bypass_mismatch_total` |

## 验证

- `cargo test -p xbxengine --lib` — **983 passed**
- `cargo test -p xbxrc --lib trace_projection` — **66 passed**
- `cargo fmt -p xbxengine -p xbxrc`

## 后续（2026-05-27 补全）

- `recovery_session_keyframe_in_flight` 在 coordinator `sync` 后与 state machine 对齐写入
- `trace_projection`：`pictureRecoveryDelegated` / `pictureRecoveryAuthority` / `sessionKeyframeInFlight`；`insertGateDecision` 含 `holdDecodeBypassMismatchTotal`
- 单测：`first_frame_emits_only_fresh_or_bootstrap_idr`、`serving_wide_without_decoder_sync_does_not_release_to_stable_serving`

可选：trace `1779930355464` 离线复验。
