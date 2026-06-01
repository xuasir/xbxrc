# Report：Receive Feedback Arbiter 简化落地修复

**Checklist：** [`rfcs/2026-05-29-receive-feedback-arbiter-landing-fix-checklist.md`](../rfcs/2026-05-29-receive-feedback-arbiter-landing-fix-checklist.md)  
**接续 RFC：** [`rfcs/2026-05-28-receive-feedback-arbiter-webrtc-alignment.md`](../rfcs/2026-05-28-receive-feedback-arbiter-webrtc-alignment.md)

## 摘要

落地 receive-local `ReceiveRecoveryLedger`（`keyframe_required` / `response_state` / terminal），`ReferenceChainState` 改为 ledger-first 投影；`ReceiveFeedbackArbiter` 主判断读 `keyframe_required`；分层 `receivePictureRecoveryTerminal`（`remote-continuation-only` / `remote-no-usable-idr` 等）；coordinator / owner 改读 ledger 投影；trace 与验收脚本拆分 `arbiterMismatchTotal` / `sparseMustIdrMismatchTotal` / `referenceStatsFallbackTotal`，并增加 `silentStuck` gate（有 terminal 时不因低 response rate FAIL）。

## 交付

| 步骤 | 交付 |
|------|------|
| Step 1 | [`recovery_ledger.rs`](../../crates/xbxengine/core/src/transport/rtc/receive/recovery_ledger.rs)；`ReceiverTraceLedger.recovery`；ledger-first `reference_chain_observation` |
| Step 2 | `ReceiveFeedbackArbiterInput/Decision` 增加 ledger 字段；`decide()` 读 `keyframe_required`；`sparse_idr_rhythm_from_recovery_ledger` |
| Step 3 | insert/decode 回写：first delta、non-IDR continuation、usable IDR、IDR unusable、clean anchor committed |
| Step 4 | `maybe_emit_picture_recovery_terminal` 分层 reason；coordinator `check_idr_completed` / transport-await 读 ledger |
| Step 5 | `VideoSchedulingOwnerInput` 读 `receive_keyframe_required`；transport-await hard rebuild 降级 |
| Step 6 | trace 扩展字段；`trace_receive_feedback_report.py` gate 更新 |

## 验证

```bash
cargo fmt
cargo test -p xbxengine --lib          # 1006 passed
cargo test -p xbxrc --lib trace_projection -- --test-threads=1  # 68 passed
python3 .agents/skills/analyze-runtime-logs/scripts/trace_receive_feedback_report.py runtime-logs/<trace>.jsonl
```

本地 trace 验收见 [`2026-05-29-receive-feedback-arbiter-trace-validation.md`](2026-05-29-receive-feedback-arbiter-trace-validation.md)（脚本 JSON：`trace-validation/`）。

## Trace 验收摘要（2026-05-29）

| 样本 | Gate | 要点 |
|------|------|------|
| `runtime-trace-1779961935840-1.jsonl`（replay 基线） | FAIL `silentStuck` | 改前画像；作回归对照 |
| `runtime-trace-1780024427780-1.jsonl`（continuation） | FAIL `arbiterMismatch` 等；**terminal=38**，无 `silentStuck` | 终端策略运营闭环 |
| `runtime-trace-1780017393821-1.jsonl`（healthy 代理） | FAIL；anchor=1 | 待**当前构建**新采 healthy |

脚本：`FreshAnchorRecovered`/`PlaybackRecovered` 计入 chain；`sparseMustIdrMismatch` 仅报告；trace 投影新增 `episodeProjectionState` / `displaySupplyStarvedBlocker`。

## 后续

- 用**当前 workspace 构建**新采 healthy trace，目标 `receiveFeedbackGate=PASS` 且全链非零
- 新采 continuation 复核 `arbiterMismatchTotal=0`（旧 trace 累计 mismatch=46 来自改前录制）

## 审查修复（2026-05-29）

- `note_usable_idr_packet_accepted`：仅记 response，不清 `keyframe_required`、不标 decoder synced；`Continuous` 仅由 decode sync / clean anchor 闭合
- `latest_transport_await_response_observed_at_ms`：不再把 `receive_keyframe_last_sent_at_ms` 当作 response observed
- `apply_decoder_facts_from_stats` + `refresh_recovery_ledger_decoder_facts`：decoder waiting / invalid / no-output / synced 回写 ledger
- `sparse_idr_rhythm_from_recovery_ledger`：去掉 `MediaSupplyPhase::MustIdr` 参与 active
- `has_transport_await_hard_rebuild_evidence`：owner 不再读 `receive_keyframe_required`
- `write_keyframe_request_outcome_stats`：去掉 stats 侧重复 `sent_count_unresolved` 递增

## 审查修复（第二轮）

- `maybe_emit_picture_recovery_terminal`：`UsableIdr` 分支接入 `decoder-rejected-idr` / `no-clean-anchor-after-decode`（`terminal_reason_after_usable_idr`）
- `latest_transport_await_response_observed_at_ms`：无 episode 时读 ledger `usable-idr` + H264 inspection 时间
- `trace_receive_feedback_report.py`：`chain_key` 优先 `ledgerGeneration`；事件流携带 `current_ledger_generation`

## 审查修复（第四轮 · Recovery Epoch 闭环）

- `apply_clear_receive_recovery_projection`：epoch 切换同步清空 `latest_h264_inspection_observation` / `receive_keyframe_last_sent_at_ms`
- `ReceiveRecoveryLedger::sync_to_stats`：reset 后显式写回 `None` sent 投影；ingress `recv_frame_inner` / `queue_transport_observation` 立即 align ledger
- 新增 `recovery/contract/completion.rs`：`receive_picture_recovery_complete*` 供 coordinator / owner 共用；删除 coordinator「receiver 离开 waiting 即完成」fallback
- `latest_transport_await_response_observed_at_ms`：H264 须 `bound_recovery_epoch == transport_recovery_epoch`
- `apply_host_display_facts_from_stats`：仅当前 epoch clean anchor 才回写 display-stable
- `build_insert_context`：`action_stage` 改读 ledger `derive_packet_recovery_action_stage`
- `trace_receive_feedback_report.py`：新增 `epochIsolationViolations` / `sameLedgerGenerationClosure` gate

## 审查修复（第五轮 · decode-sync 与 owner release）

- `apply_clear_receive_recovery_projection`：epoch 切换同步清空 `recovery_decoder_reference_synced_at_ms` / `latest_video_decode_ok_*`
- `completion.rs`：`picture_recovery_evaluation_now_ms` 不再用旧 decode 时间作 freshness `now`；`decoder_reference_synced_for_recovery_epoch` 要求 sync 不早于当前 episode 或绑定当前 epoch H264
- `has_current_clean_anchor_release_evidence`：移除 `clean_anchor_epoch` 单独短路，统一走 `receive_picture_recovery_complete_from_fields`
