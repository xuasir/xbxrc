# RFC：MediaSupply 生命周期单轨

**状态：** 实施中（2026-05-27）
**关联：** [2026-05-25 低延迟显示调度](2026-05-25-low-latency-display-scheduling-optimization.md)、[2026-05-26 Receive InsertGate 收敛](2026-05-26-receive-insert-gate-and-recovery-convergence.md)

## 问题

近期 trace 显示 Owner `supply-starved`、receiver `waiting-keyframe`、picture `PlaybackRecovered` 与 `decodeFps` 健康 **同 tick 并存**（`1779865309109`）。修洞末段 HoldRepair 与 PLI 无闭环（`1779863398229`）。非单点 bug，而是 **缺少全生命周期单一供给态**。

## 目标

- 对外唯一主叙事：`media_supply_phase`（`priming` | `steady` | `repairing` | `must_idr` | `supply_break`）。
- `RecoverySurfacePhase`、Owner 状态、receiver 字符串均为其 **投影**，禁止平行裁决。
- 动作不变式：`HoldRepair` ⇒ 可观测 receive-local PLI 尝试；Priming 阶段禁止误标 `SupplyStarved`。

## 阶段表

| media_supply_phase | 宽进 delta | 要 IDR | RecoverySurface 投影 |
|--------------------|------------|--------|----------------------|
| priming | steady continuation（metadata 齐） | follow-up only | steady |
| steady | 05-25 `steady_displayed_idr_delta_admits` | hard gap / corrupt | steady |
| repairing | 不喂 bootstrapMissingIdr | NACK→PLI→FIR | repairing |
| must_idr | 非 IDR Hold | hard PLI（免 session cooldown） | await-idr |
| supply_break | supply-break 窄路径 Emit | coordinator | supply-break |

## Priming 超相位（非补丁门控）

- **未完成前** `derive_media_supply_phase` 恒为 `priming`，吞掉 receiver repairing / gap / waiting 子态。
- **首显后 5s acquisition 窗**（`media_supply_host_first_present_at_ms`）：窗内恒 `priming`，即使当拍 decode/submit 已健康（对齐 MEDIA_SUPPLY_GATE）。
- **窗后完成条件**：`decode_age`≤200ms 且 `submit_age`≤500ms；picture `PlaybackRecovered` 不参与退出。
- Owner：`media_supply_phase == priming` → 表驱动 `Priming`；`keyframe_request_outcome_seq` 递增驱动 trace 事件（非 label 去重）。

## PS 严进

仅当 `displayed_idr_serving` 且 **非 priming** 时，`video_parameter_sets_changed_at_ms` 触发严进窗。

## Trace 门禁

| 场景 | 通过标准 |
|------|----------|
| 起播（5309109 类） | 首显后 5s 无 `supply-starved`；≥1 `keyframeRequestOutcome`；`waiting-keyframe` 簇 <3/60s |
| 稳态（177970） | `STEADY_SUPPLY_GATE`；`session_phase=steady` >95% |
| 修洞末段（339） | HoldRepair 后 3s 内 PLI；10s 内 submit P95<500ms 或 reset |
| 回归（177985） | 无长窗 supply-break + transport-await 叠层 |

## 层边界（延续 P2）

- L0：`derive_media_supply_phase_from_stats` @ `recovery/contract.rs`
- L1：Insert 读 `media_supply_phase`，禁止 Priming 走 PS 严进 Hold
- L3：Owner 读 `contract_snapshot.media_supply_phase` + `post_first_present_acquisition`
- L4：receive-local PLI 不经 session `suppress_session_picture_recovery_action` 吞掉
