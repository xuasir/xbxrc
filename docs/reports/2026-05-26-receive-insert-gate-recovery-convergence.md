# Report：Receive InsertGate 与恢复链收敛（2026-05-26）

## 交付摘要

- **InsertGate**：`crates/xbxengine/core/src/transport/rtc/receive/insert_gate.rs`，ingress `evaluate_decode_gate` 单点裁决。
- **TimedFallback 止血**：decode bypass、`displayed_idr_fast_path` 绕过 PathD cooldown、decoder unstick、nominal reset 在 bypass 下进入 `Recovering`。
- **Receive-only PLI**：`recovery/suppress.rs`；session proposal PLI→`CooldownSuppressed`；`recovery_receive_keyframe_hint_at_ms` + ingress 消费。
- **RecoverySurface / DerivedDecoderHealth**：`recovery/contract.rs` + `sync_derived_recovery_contract_fields`；trace `recoverySurface` / `derivedDecoderHealth`。
- **Owner**：TimedFallback ingress-waiting 不再默认 `recoverySustaining` 标签。

## 验证

```bash
cargo fmt
cargo test -p xbxengine --lib
```

trace 门禁（同类 `1779845409388`）：`timedFallback` 后 5–10s 内 `submit_age` &lt; 500ms 或 `requestDecoderReset` / receive PLI episode。

## P2 层边界收敛（2026-05-27）

- **L0**：`RecoveryContractSnapshot` + 模块顶边界表；`recovery_supply_break` / surface phase 唯一实现于 `contract.rs`。
- **时序**：`build_rtc_session_policy_orchestration_input` 先 `sync_derived_recovery_contract_fields`，Owner 输入携带 `recovery_surface_phase` / `derived_decoder_health`。
- **Owner 瘦身**：`SupplyBreak` 表驱动 intent（`build_supply_break_recovery_intent`）；禁止 supply-break 叠 transport-await；surface 强制 `SupplyStarved` 状态。
- **Ingress**：`HoldRepair` 走 `insert-gate-hold-repair`，不再进 `frame-inspection-rejected-await-anchor`。
- **Decode**：`insert_emit_bootstrap_bypass` 与 bootstrap 闸冲突时 `insertDecodeContractViolation` 注记。

验证：`cargo test -p xbxengine --lib`（971 passed，含 P2 收口补测）。

## P0–P2 收口（同日报后续）

- **Insert ↔ decode**：`insert_emit_bootstrap_bypass` + `recovery_keyframe_action_for_insert_decision`；ingress Emit 不再被 `resolve_recovery_keyframe_action` 打回 WaitKeyframe。
- **Session PLI**：生产 `transport_session` 仅 `receiveOnlyPictureRecovery` defer；`#[cfg(test)]` 保留 command/ledger 回归。
- **Suppress**：`recovery/suppress.rs` 合并 DisplaySupply + receive-only 单出口 `finalize_session_picture_recovery_action`。
- **Owner**：移除 `recoverySustaining` 意图标签；`derived_decoder_health` 用于 suspect_anchor / fast-path PathD / policy 门控。
- **遗留删除**：`receive/picture_recovery.rs`；`startup_compat` 不再认 `recoverySustaining` reason。
