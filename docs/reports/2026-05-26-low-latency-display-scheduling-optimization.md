# 低延迟显示调度优化 — 实施报告

关联 RFC：[2026-05-25-low-latency-display-scheduling-optimization.md](../rfcs/2026-05-25-low-latency-display-scheduling-optimization.md)

## 摘要

按 RFC Phase 0–5 完成代码与观测基础设施落地：三条 native 路径统一 host present tick rerun + `hostMailboxTakeDecision` trace；steady 龄期与 pending 持帧语义收紧；stats 增加 `submit_to_present_ms` / `inspection_pulse_active`；云 profile jitter/NACK 小步收紧；提供中后段 trace 门禁脚本。

## 代码变更

### Phase 1 — Host present

- `request_host_present_tick_dispatch` / `finish_host_present_tick_dispatch` 提升为全平台 API。
- **Windows wgpu**、**macOS layer**（含 display link / 16ms fallback）：`render_loop_rerun_requested`，Accepted 后 tick 不再因 pending 占用而静默丢失。
- `record_host_mailbox_take_decision`：macOS wgpu、Windows `wgpu-windows`、layer 同口径。
- `scheduling.rs`：steady 下 `frame_age_budget_ms` 以 `display_interval_ms` 托底；无 pending 时才 `record_present_refresh`。

### Phase 2 — 观测

- `stats.rs` / `protocol/runtime.rs`：`submit_to_present_ms`、`inspection_pulse_active`。
- `trace_projection.rs` 投影新字段。

### Phase 3 / 4 — 云 profile

- `compiler.rs`：Rust-owned cloud `jitter_buffer_min_delay_ms` 28→15，`max_delay` 48→36；`nack_retry_interval_ms` 90→80。

### Phase 5 — 脚本

- [`.agents/skills/analyze-runtime-logs/scripts/trace_midsegment_report.py`](../../.agents/skills/analyze-runtime-logs/scripts/trace_midsegment_report.py)：79–150s 窗口门禁（steady 占比、recovering 脉冲、submit/present P95、mailbox 异常）。

## 验证

```bash
cargo fmt
cargo test -p xbxrc --lib native_video
cargo test -p xbxengine --lib displayed_idr
python3 .agents/skills/analyze-runtime-logs/scripts/trace_midsegment_report.py runtime-logs/runtime-trace-<new>.jsonl
```

### 基线 trace 门禁（Phase 0 前采集 `1779701428966`）

对**已合入 Phase 0 之前**的基线 trace 跑脚本仍为 **FAIL**（steady 84.6%、recovering 信号 9、submit P95 315ms），与 RFC 问题描述一致。需在**本批改动合入后**复采同场景 trace 再跑脚本验收。

## 回归修复（2026-05-26，`runtime-trace-1779764321731`）

**现象**：decode ~30fps，84 次 `hostMailboxAccepted`（`hasPending=true`），但 `host_frame_present_epoch=0` 全程无画面。

**根因**：Phase 1 移除 `PendingFlagGuard` 后，`run_layer_present_tick` 在 `SkippedNoReadyFrame` 等路径提前 `return`，未调用 `finish_host_present_tick_dispatch`，`render_loop_pending` 永久为 true，display link 不再调度 take。

**修复**：`HostPresentTickGuard`（RAII）在 layer / Windows wgpu / macOS wgpu tick 入口创建，任意出口释放 pending 并处理 rerun。

## Phase 0 控制面修复（2026-05-26，`runtime-trace-1779766401223`）

**现象**：首帧 ~+58s 出现，`PlaybackRecovered` 已触发，但 `recovery_displayed_idr_at_ms` 长期为 None；79–150s steady 占比 0%，recovering 脉冲密集。

**根因**：latest-only mailbox 上屏的是 IDR 之后的 delta RTP；`update_host_video_present_metrics` 用 displayed delta 调 `record_displayed_idr_fact`，与 `recovery_pending_displayed_idr_rtp` 不匹配，fresh anchor commit 失败。ingress 侧 `displayed_idr_serving` 仅认 `recovery_displayed_idr_at_ms`，Phase 0 的 `steady_displayed_idr_delta_admits` / hold steady 不生效。

**修复**：

- `contract.rs`：`displayed_idr_serving_from_stats()`、`resolve_host_display_idr_anchor_rtp()`。
- `runtime_port.rs`：present 提交 anchor 时优先 pending IDR RTP。
- `ingress_state/decode.rs`、`startup.rs`、`runtime_state.rs`：统一用 serving helper。

**验证**：`cargo test -p xbxengine --lib displayed_idr` + `runtime_port::` 全通过；待同场景复采 trace 跑门禁脚本。

## P1 / P3 跟进（2026-05-26，`1779773137501` 复验后）

**P1 — displayed IDR 已 serving 时抑制 `receiverWaitingKeyframe` 短脉冲**

- `should_collapse_receiver_waiting_keyframe_to_repairing()`：gap repair 期间将 receiver 投影为 `repairing/gapRepairInFlight`，不再闪 `waiting-keyframe`。
- `resolve_recovery_keyframe_action(..., displayed_idr_serving)`：serving 下 blocking 时强制 `Submit`，避免 `WaitKeyframe` 空窗。
- `enter_recovery_wait_from_source`：serving + 上述 timeline reason 时不再 `set_is_blocking_non_keyframe_admission(true)`。

**P3 — 观测去噪**

- `inspection_pulse_active`：仅 `!admission_accepted` + `submit_age≥200ms`（不再因 Accept 的 `bootstrapMissingIdr` 元数据误报）。
- trace：`bootstrapRejectObserved` 仅在 **实际拒绝** 时发出（`admission_accepted == false`）。

**验证**：`cargo test -p xbxengine --lib ingress_loop` 60 passed；trace_projection 66 passed。

## P2 — decode 输出邮箱丢帧调度（2026-05-26）

**问题**：`coalescedAfterDecode` 在 20ms 固定窗内对 `supply|supply` 丢弃**新入帧**，与 latest-only「保留最新候选」相反，trace 上约 ~10/s 的 decode output drop 多由此产生。

**修复**（`video_decode.rs` + `decode/actor.rs`）：

1. **取消** steady `supply/disposable` 的「时间窗内丢 incoming」coalesce；steady 统一走 **value-aware supersede**（保留更高价值/更新帧）。
2. **保留**恢复窗 **anchor/keyframe 保护**：仅当 mailbox 已有 keyframe/anchor 且新帧非 keyframe、且非 `FreshAnchor/RecoveryContinuation` 升级时，在 **host 显示间隔对齐** 的短窗内丢弃突发 continuation（`coalescedAfterDecode`）。
3. `set_mailbox_host_cadence(host_display_interval_ms)`：coalesce/保护窗口与 host tick 对齐（12–50ms，默认 33ms），不再用固定 20ms。

**验证**：`cargo test -p xbxengine --lib media::video::decode::video_decode::tests` 46 passed。

**待复采**：同场景 trace 对比 `video_decode_output_drop` 速率是否下降、present 是否略升（仍不追求 present≈decode）。

## Phase 6 — WebRTC 式恢复五点（`runtime-trace-1779785704665`）

**事故摘要**：末段 ~51s `sessionPhase=recovering` + `stallKind=waitingKeyframe` + `awaitKeyframe:hostIdrOrCleanAnchor`；`submit_age_ms` 单调涨至 ~51s；`hostMailboxEnqueue` 停更；TWCC 仍 healthy；仅 4 次 `h264IdrAccessUnitObserved` 且末次后为零；~500ms 周期 `requestDecoderReset`；gap repair-in-flight 与 waiting-keyframe 并行。

**落地**（见 RFC Phase 6）：

- 合同 `recovery_exit_path_from_stats` + owner `ServingReady` / `recoverySustaining` 降级。
- `decoder_reset_permitted_from_stats` 抑制无 IDR 的 reset 环。
- ingress / RTP 保护 IDR；gap↔keyframe 仲裁；PathD PLI；trace 注记 `awaitKeyframe:decodeOutput|timedFallback`。

**验收**：合入后复采同场景；`trace_midsegment_report.py` 增加 recovering 连续 >5s、waiting-keyframe 下 reset 爆发门禁。

## 后续

1. 同场景复采 ≥120s trace（非 minimal），确认 Phase 0+1+6 门禁通过。
2. 若 submit 尖峰仍与 inspection 对齐，用 `inspectionPulseActive` 对照 `h264InspectionRejected` 时间线。
3. Open Q2：面板区分 decode 供给 / present 节拍（可单独 UI 任务）。
