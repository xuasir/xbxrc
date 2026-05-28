# RFC：恢复链减法与 libwebrtc 对齐

**状态：** 已实施（2026-05-27）  
**范围：** `xbxengine` receive / recovery / session policy  
**Supersedes（部分）：** [2026-05-26 Receive InsertGate 与恢复链收敛](2026-05-26-receive-insert-gate-and-recovery-convergence.md) 中的 fast-path / timed-fallback decoder unstick 分支

## 问题

在 displayed-IDR serving + `waiting-keyframe` 场景，session 第二层恢复（5s fast-path、timed-fallback promote `RequestDecoderReset`）与 receive 单轨 PLI 打架，trace 上出现 200+ `requestDecoderReset` 风暴，而 hopeless delta 仍 `decodableToFeed`。

## 目标形态

```text
InsertGate (Emit/Hold/Drop) → KeyframeRequester / NackRequester
                              ↑ hint only
Session: DelegatedToReceive | RequestDecoderReset(仅 backend/reconfigure)
```

## 落地摘要

| 项 | 动作 |
|----|------|
| Fast path | 删除 `displayed_idr_fast_path.rs`；policy 改为 `maybe_receive_keyframe_delegation_owner_signal` |
| Reset promote | 删除 `maybe_promote_timed_fallback_decoder_unstick` / supply-break reset promote |
| Policy 硬盖 | 删除 `should_force_timed_fallback_decoder_unstick` |
| Insert | `InsertDecisionReason` 收敛；删除 `timedFallbackDecodable`；`MustIdr` 下非 bootstrap delta 一律 Hold |
| Contract | `waiting-keyframe` 优先 `MustIdr`；`derive_media_supply_phase` 不再因 timed-fallback unstick 进 SupplyBreak；`decoder_reset_permitted` 收窄 |
| IDR 谓词 | 唯一入口 `idr_recovery_active_from_stats`（`MustIdr`）；已删 `reference_chain_*` |
| Contract 结构 | `recovery/contract/` 多模块：`insert` / `decode_sync` / `gap` / `sparse_idr` / `supply` / `snapshot` / `display` / `transport_await` / `exit` + `tests.rs` |
| 诊断外迁 | `session/facts/recovery_episode.rs`、`gap_severity.rs`；contract **不** re-export episode/ledger |
| Session PLI | `action_coordinator` 出口 `suppress_session_picture_recovery_action` |
| Ledger 叙事 | `resolve_recovery_state` 读 `recovery_surface_phase=await-idr` → `ActiveRecovery` |

## RequestDecoderReset 纪律

- **允许：** `DecoderBackendFailure`、`Reconfigure`、显式本地 maintenance
- **禁止：** reference chain / supply-break / timed-fallback 证据链上的 session promote reset；IDR 链一律 `DelegatedToReceive` + receive hint

## 验收

见 [report](../reports/2026-05-27-recovery-subtraction-validation.md)。
