# Report：Receive Feedback Arbiter 与 WebRTC 接收侧对齐

**RFC：** [`rfcs/2026-05-28-receive-feedback-arbiter-webrtc-alignment.md`](../rfcs/2026-05-28-receive-feedback-arbiter-webrtc-alignment.md)

## 摘要

图片级 NACK / PLI / FIR 已收口到 receive 层 `ReceiveFeedbackArbiter`；Insert/Decode 准入由 `ReferenceChainState` 主导（`ReceiverTraceLedger` 为事实源，stats 为 fallback）；trace 提供 `receiveFeedbackDecision`、`referenceChainStateChanged`、`receivePictureRecoveryTerminal` 与验收脚本 rates/gate。

## Phase 0–5 交付

| 阶段 | 交付 |
|------|------|
| 0–2 | `ReceiveFeedbackArbiter`、`plan/execute_receive_feedback`、NACK/Insert 发送路径 |
| 3 | `ReferenceChainState` 驱动 `decodable_to_feed` / Insert Hold |
| 4 | 初版 trace + `trace_receive_feedback_report.py` |
| 5 | Insert 与 arbiter 同源 `reference_chain_observation`；`MediaSupplyPhase`/`displayed_idr_*` 仅 projection；trace 扩展字段 + terminal `remote-no-usable-idr`（`sent≥5` 或 `3×effective_rtt`）；violation 计数；脚本 `rates` / `receiveFeedbackGate` |

## 验证

```bash
cargo fmt
cargo test -p xbxengine --lib          # 1000 passed
cargo test -p xbxrc --lib trace_projection -- --test-threads=1  # 68 passed
python3 .agents/skills/analyze-runtime-logs/scripts/trace_receive_feedback_report.py runtime-logs/<trace>.jsonl
```

### 脚本样本（改前 trace，`receiveFeedbackDecisionEvents==0` 预期）

`runtime-logs/runtime-trace-1779953007765-1.jsonl`：历史样本无 `receiveFeedbackDecision`；gate 可能因 `sent==0` 跳过 response 阈值。新采 trace 应含扩展字段与 terminal 事件。

## 后续

- 用含 `receiveFeedbackDecision` 的新 trace 填 gate PASS 样本进本 report
- `nackEffectiveRate` 可继续委托 `summarize_runtime_trace.py` 的 `recovery_audit.nackEffectiveness`
