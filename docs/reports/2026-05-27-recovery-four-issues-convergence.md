# Report：恢复链四问题收口（2026-05-27）

**RFC：** [`2026-05-26-receive-insert-gate-and-recovery-convergence.md`](../rfcs/2026-05-26-receive-insert-gate-and-recovery-convergence.md)

## 摘要

在 WebRTC 对齐基础上，收口自查 1–4：L0/L3 合同单轨、Insert↔Decode 可观测、Session 图片级恢复委托与 cooldown 分流、MustIdr 稀疏 IDR 下 receive-only PLI 加速（不放宽无参考 delta）。

## 变更要点

1. **降耦合：** `insert_gate` 删除 bootstrap/timed_fallback/repairing 旁路；Owner `displayed_idr_serving_release` 要求 `contract_snapshot.decoder_reference_synced`。
2. **可观测：** `latest_insert_decision` / `latest_insert_decision_reason` / `insert_decode_bypass_aligned`；trace `insertGateDecision`。
3. **Session：** Owner 提议 PLI/FIR 立即 `DelegatedToReceive`；receive 已发 PLI 时不 coalesce `keyframeInFlight`。
4. **稀疏 IDR：** `sparse_idr_pressure_active_from_stats`；`KeyframeRequester` 稀疏间隔；HoldRepair 源 `insert-gate-hold-repair-sparse`；NACK escalation 在稀疏压力下 `arm_immediate`。

## 验证

```bash
cargo fmt --all
cargo test -p xbxengine --lib   # 981 passed
cargo test -p xbxrc --lib trace_projection -- --test-threads=1  # 66 passed
```

## 后续

- 用 `runtime-trace-*.jsonl` 对照 `insertGateDecision` / `keyframeRequestOutcome`（尤其 `insert-gate-hold-repair-sparse`）做现场 P95 对比。
