# RFC：Receive InsertGate 与恢复链收敛

**状态：** 已实施（2026-05-26）  
**范围：** 仅客户端（`xbxengine`、`src-tauri`）  
**关联：** [2026-05-25 低延迟显示调度](2026-05-25-low-latency-display-scheduling-optimization.md)、[2026-05-12 传输修复语义统一](2026-05-12-transport-repair-and-recovery-semantic-unification.md)

## 问题

同一 AU 在 ingress Accept 与 decode `bootstrapGateRejected` 分叉；`CleanAnchorCommitted` 后 ledger 长期 `suppress`；TimedFallback 控制面已激活但 decode 供给未恢复。

## 目标单轨

- **InsertGate**（`receive/insert_gate.rs`）为 pre-decode 唯一裁决：`Emit` / `HoldRepair` / `DropCorrupt`。
- **PLI/FIR** 仅 receive（`KeyframeRequester` + nack escalation）；session `RequestPli` 映射为 `CooldownSuppressed`。
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
