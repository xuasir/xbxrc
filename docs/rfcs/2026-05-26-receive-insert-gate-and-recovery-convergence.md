# RFC：Receive InsertGate 与恢复链收敛

**状态：** 已实施（2026-05-26）；fast-path / timed-fallback reset 分支已由 [2026-05-27 恢复链减法](2026-05-27-recovery-subtraction-libwebrtc-alignment.md) supersede
**范围：** 仅客户端（`xbxengine`、`src-tauri`）
**关联：** [2026-05-25 低延迟显示调度](2026-05-25-low-latency-display-scheduling-optimization.md)、[2026-05-12 传输修复语义统一](2026-05-12-transport-repair-and-recovery-semantic-unification.md)

## 问题

同一 AU 在 ingress Accept 与 decode `bootstrapGateRejected` 分叉；`CleanAnchorCommitted` 后 ledger 长期 `suppress`；TimedFallback 控制面已激活但 decode 供给未恢复。

## 目标单轨

- **InsertGate**（`receive/insert_gate.rs`）为 pre-decode 唯一裁决：`Emit` / `HoldRepair` / `DropCorrupt`。
- **PLI/FIR** 仅 receive（`KeyframeRequester` + nack escalation）；session `RequestPli`/`RequestFir` 映射为 `DelegatedToReceive`（真抑制仍用 `CooldownSuppressed`）。
- **RecoverySurfacePhase** 四态：`steady` | `repairing` | `await-idr` | `supply-break`（写入 `recovery_surface_phase`）。
- **DerivedDecoderHealth** 供 owner/facts；裸 `video_decoder_recovery_state` 仅 diagnostics/trace 深链。

## 与 libwebrtc 差异（保留）

| 项 | libwebrtc | xbxrc |
|----|-----------|-------|
| Post-decode | 通常直接渲染 | latest-only mailbox + pacer |
| 帧率 | 尽量对齐 | decode_fps ≠ present_fps（云游戏低延迟） |
| Insert 花屏 | StandardWebRtc 允许 repairing delta | 同，且 displayed-idr 续播窄路径 |

## 层边界与禁止并行裁决（P2）

| 层 | 模块 | 唯一问题 | 禁止 |
|----|------|----------|------|
| L0 | `recovery/contract.rs` | `RecoverySurfacePhase` / supply-break / serving 宽窄 | Owner 内平行 supply-break 阈值 |
| L1 | `receive/insert_gate.rs` | 本 AU Emit/Hold/Drop | 用 `allows_relaxed` 填 Insert `displayed_idr_serving` |
| L2 | `media/video/decode` | 后端无输出 / reset | Insert 已 Emit 时主路径 `bootstrapGateRejected` |
| L3 | `policy/video_scheduling_owner` | 呈现 supply / host stall intent | supply-break 时再发 transport-await |
| L4 | receive + session | PLI/reset/hint | session 生产执行 RequestPli |

`RecoveryContractSnapshot::from_stats` 为单 tick 合同快照；`build_rtc_session_policy_orchestration_input` **先** `sync_derived_recovery_contract_fields` 再组装 Owner 输入。

## 验收

1. 同 AU 不长期 ingress Accept + decode reject（TimedFallback 窗除外且须跟 reset/PLI）。
2. gap 超阈值或 submit_age≥4s + waiting-keyframe → receive PLI 或 decoder reset，&lt;10s 恢复 submit 或终止。
3. trace `recoverySurface` 可解释会话；session 无生产 `RequestPli` 执行。

## WebRTC alignment（可解码性 / 丢包 / 关键帧纪律）

在保留 latest-only mailbox 与 decode/display 解耦的前提下，L0–L2 统一以下合同：

| 符号 | 层 | 职责 |
|------|-----|------|
| `PacketRecoveryActionStage` | L0 `contract.rs` | `drop → nack_pending → nack_missed → wait_keyframe → request_idr` |
| `decodable_to_feed` | L0 + L1 | Insert 与 decode bypass 共用：无 `decoder_reference_synced` 不 Emit continuation |
| `decoder_reference_synced_from_stats` | L0 | bootstrap_ready IDR decode 或新鲜 `recovery_decoder_reference_synced_at_ms` |
| `displayed_idr_serving` | L0 宽 | Priming / 控制面放松；**不**单独驱动 Insert Emit |
| `displayed_idr_decoder_synced` | L0 窄 | steady displayed-IDR delta 续播 |
| `RecoveryAction::DelegatedToReceive` | L4 session | owner 提议 PLI/FIR 委托 receive，与真 `CooldownSuppressed` 分离 |

**禁止：** 仅凭 host `displayed-idr` 向解码器灌无参考 delta；`nack_pending` 不得直跳 `request_idr`（须 gap_stale / decoder_waiting / no_output 证据）。HoldRepair/hint 走 receive soft PLI（`request_if_due`），不每 AU `force`。

**Trace：** `keyframeRequestOutcome` 使用 `latest_keyframe_request_source` / `latest_keyframe_request_outcome`，不再绑 `latest_observation_summary`。

## 四问题收口（2026-05-27）

| # | 问题 | 落地 |
|---|------|------|
| 1 | 新旧耦合 | `RecoveryContractSnapshot` 增 `decoder_reference_synced` / `action_stage`；Insert 单出口；Owner release 须 decoder sync |
| 2 | 新逻辑可证 | stats/trace：`insertGateDecision`、`insert_decode_bypass_aligned` |
| 3 | 决策链/cooldown | Session PLI 先 `DelegatedToReceive`；receive 在途时不 `CoalescedKeyframeInFlight`；MustIdr 未 sync 不 fresh-output suppress |
| 4 | 稀疏 IDR PLI | `sparse_idr_pressure_active_from_stats` + 缩短 `KeyframeRequester` 间隔 + NACK escalation `arm_immediate`；**不**放宽 delta Emit |

**稀疏 IDR：** 见 [`2026-05-14-dynamic-rtt-aware-recovery-timing.md`](2026-05-14-dynamic-rtt-aware-recovery-timing.md)（本子集仅 receive PLI/NACK dwell）。

Report：[`reports/2026-05-27-recovery-four-issues-convergence.md`](../reports/2026-05-27-recovery-four-issues-convergence.md)。

## 四标准结构性收口（2026-05-27）

| 标准 | 落地 |
|------|------|
| **单轨** | 生产 Insert 仅 `resolve_insert_decision`（首帧 `firstFrameFreshOrBootstrapIdr`）；`resolve_inspection_admission` 仅 `#[cfg(test)]`；Owner 仅读 `RecoveryContractSnapshot.serving_wide` |
| **短链** | `VideoEscalationController` 图片级一律 `DelegatedToReceive`，不 `try_enter_keyframe_epoch`；coordinator `sync` 前 `suppress_session_picture_recovery_action` |
| **稀疏 IDR 节奏** | `SparseIdrRhythm`（`active` / `pli_due` / `action_stage`）；`receive_keyframe_last_sent_at_ms`；NACK `arm_immediate` 须 `WaitKeyframe`+`pli_due` |
| **session 不污染 in-flight** | `CoalescedKeyframeInFlight` 在 `receive_local_keyframe_request_recent` 时 no-op；policy 首帧 coalesce 改 `DelegatedToReceive`+hint |

**可证伪：** `recovery_picture_recovery_authority`、`insert_hold_decode_bypass_mismatch_total`、`pictureRecoveryDelegated` 观测。

Report：[`reports/2026-05-27-recovery-four-standards-structural.md`](../reports/2026-05-27-recovery-four-standards-structural.md)。
