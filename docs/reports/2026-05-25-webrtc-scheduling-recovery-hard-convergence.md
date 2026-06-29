# WebRTC 调度/恢复硬收口 — 实施报告

**日期**：2026-05-25
**关联计划**：`.cursor/plans/webrtc_调度硬收口_412bf341.plan.md`（未改 plan 文件）
**关联 RFC**：`docs/rfcs/2026-04-29-playback-recovery-single-line-convergence.md`、`docs/rfcs/2026-05-20-receive-side-loop-rearchitecture.md`

## 摘要

将 post-decode / stats / owner / session 控制面从「episode + CleanAnchorSubmitted/Committed + bridge」硬切为浏览器 式双事实：

1. **Pre-decode**：`ReceiverState`（receive / decode_gate）
2. **Post-decode**：`DisplayedIdrFact` + `PlaybackRecovered`（host present 单点写入）

## 已完成

### 阶段 1 — RecoveryDisplayFacts

- `record_pending_displayed_idr_rtp` / `record_displayed_idr_fact` / `record_playback_recovered_fact`
- 删除 `record_transport_clean_anchor_submission*`、`record_transport_clean_anchor_bridge_with_rtp` 及 unsolicited 旁路
- `runtime_port::update_host_video_present_metrics` 为唯一 fresh-anchor / playback 写入点；host stall **不再** invalidate 已显示 IDR
- `apply_transport_clean_anchor` 固定 `displayed-idr`；transition phase `FreshAnchorRecovered`

### 阶段 2 — VideoSchedulingOwner

- `VideoSchedulingOwnerInput` 增加 `recovery_displayed_idr_at_ms` / `recovery_playback_recovered_at_ms` / `recovery_fresh_anchor_recovered_at_ms`
- `has_established_displayed_idr_fact`：`bootstrapMissingIdr` 不再否定已显示 IDR
- `has_current_clean_anchor_release_evidence` 仅认 displayed-idr 事实（移除 bridge 控制路径）
- 测试：`rebuilding_supply_allows_displayed_idr_fact` 替代 bridge 合同

### 阶段 3 — facts / coordinator / policy

- `compute_recovery_facts` 使用 `has_current_clean_anchor_from_stats`
- `reconcile_recovery_progress_from_current_bootstrap`：已显示 IDR 时固定 `CleanAnchorRecovered`；`playback_recovered` 优先于 continuation 回写
- `recovery_anchor_evidence_trace_code`：优先 `displayedIdr` / `playbackRecovered`
- `check_idr_completed`（**生产**）：仅 `recovery_displayed_idr_at_ms` 或 receiver 离开 `waiting-keyframe`；`update_inflight_status` 用其清 PLI in-flight
- `has_current_clean_anchor_from_stats` / `current_clean_anchor_observed_at_ms`：硬切仅认 `displayed-idr` + `recovery_*` 字段（不再认 `chain-clean-anchor-submitted` / bridge）

### 阶段 4 — Decode 解耦

- decode drain 仅 `record_pending_displayed_idr_rtp`（无 submission / commit epoch 写 stats）
- `recovery_chain_unsettled` 不再依赖 `clean_anchor_commit_recovery_epoch`
- 邮箱字段 `clean_anchor_commit_recovery_epoch` 仍保留于 frame 类型（仅传播/测试），**不**再驱动 stats 控制

### 阶段 5 — Trace / 前端

- Trace：`displayedIdrObserved`、`playbackRecovered`、`receiverStateChanged`、`freshAnchorRecovered`；`h264IdrAccessUnitObserved` 要求 `admission_accepted`
- DTO：`recovery_displayed_idr_rtp` / `recovery_displayed_idr_at_ms` 导出到 stats snapshot
- 前端 diagnostics 已沿 `waitingKeyframe` / `playing` / stallKind 口径（无 `chain-clean-anchor-submitted` 主因）

### 阶段 6 — 验收

- `rg`：`record_transport_clean_anchor_submission` / `cleanAnchorEpisodeUnbound` 生产路径 **0**（仅 `docs/project-task.md` 历史条目）
- 回归（抽样全绿）：`runtime_port` 13、`video_scheduling_owner` 65、`receive::` 126+、`clean_anchor` 相关 55+、`displayed_idr` 单测

## 9080323 类场景验证

| 检查项 | 收口后 | 自动化 |
|--------|--------|--------|
| 首包 IDR 后 | trace `displayedIdrObserved` / `freshAnchorRecovered` | `trace_projection.test`: `displayedIdrObserved` + `legacy_chain_clean_anchor_submitted_timeline_does_not_emit_clean_anchor_submitted` |
| 同会话 | 无 `CleanAnchorSubmitted` 由旧 submission 投影；无 episode 驱动 PLI 完成 | coordinator: `legacy_chain_clean_anchor_submission_does_not_complete_idr` |
| present 空洞 | owner 不因 `bootstrapMissingIdr` 推翻 displayed IDR | `video_scheduling_owner.test`: `rebuilding_supply_allows_displayed_idr_fact` |
| owner / coordinator | `displayed-idr` 或 receiver `receiving` 完成 PLI in-flight | `check_idr_completed_when_*` |

仓库内无 `runtime-trace-*9080323*` 样本文件。已用 `src-tauri/src/mods/xbxengine/fixtures/webrtc_recovery_9080323_contract.json` + `trace_projection::webrtc_recovery_9080323_contract_validation_matrix` 固化上表 5 行契约；上线后请在 `runtime-logs/` 复采同类会话与 fixture 对照。

## 后续（人工）

- Cloud/Home 复采 `runtime-trace-*9080323*` 同类会话：与 fixture 5 行 + 自动化矩阵结果对照（实机 golden 仍建议保留一份 JSONL）
- 全量 `cargo test -p xbxengine --lib`（CI）

## 2026-05-25 host present 持帧误报修复

- **根因**：`RetainedDisplayedFrame` 误调 `record_no_pending_take()` → `cadencePhase=starved`，且未刷新 `latest_present_time_ms`，导致 `presentAge` 虚高与 owner `hostPresentStalled`。
- **修复**：`record_display_hold` + `record_present_refresh`；持帧期放宽 `stale_frame_age_budget`；recovery keyframe 放宽 `RejectedAlreadyPresented`；owner/runtime 在 `steady`+displayed-idr 时不报 host present stall / rebuildingSupplySuspect。
- **测试**：`retained_displayed_frame_does_not_enter_starved_or_no_pending_streak`；`host_mailbox_state_steady_cadence_does_not_project_retained_old_frame_risk`。

## 2026-05-25 补强（五项建议）

| 项 | 落地 |
|----|------|
| P0 9080323 验收 | fixture JSON + `webrtc_recovery_9080323_contract_validation_matrix` |
| P1 Owner 四态 | `VideoSchedulingOwnerContractState` + `video_owner_contract_state` / `recovery_owner_contract_state` |
| P2 Coordinator 硬证据 | 去掉 episode terminal/progress；`transport_await_has_hard_bootstrap_evidence_from_stats` |
| P3 Owner 诊断 | `build_temporary_diagnostic_summary` 仅认 displayed-idr / fresh-anchor |
| RecoveryDisplayFacts | `contract::RecoveryDisplayFacts::from_stats`；facts 读路径统一投影 |
