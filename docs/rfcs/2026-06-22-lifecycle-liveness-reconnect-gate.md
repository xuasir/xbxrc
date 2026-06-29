# Lifecycle Liveness Reconnect Gate RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成（代码与单测已完成；fresh trace 运行时验收待跑）
- Current State: implemented
- Owner: agent
- Last Updated: 2026-06-23

## Background

最新 trace [`runtime-trace-1782114876554-1.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1782114876554-1.jsonl) 表明：

- +42.348s transport 进入 `Connected`，+43.385s 进入 `DisplayStable`，+46.849s stats 达到 `present_fps=60.79 / decode_fps=60.79`。
- ICE probe 多次给出 `hasSelectedOrNominatedPair=true`、`failedPairCount=0`、`directChecksWithoutResponse=false`。
- TWCC/remote track 在可用窗口里健康，`twcc_loss=0.0`、`twcc_delivery=1.0`、`remoteTrackAttached` 持续推进。
- +53.197s、+68.693s、+76.820s、+84.114s 出现 `RtcVideoFrameSource rx closed cause=rebuildPeerConnection`。
- +82.732s 出现 `videoEscalation reason=livenessNoProgressTimeout`，+90.051s runtime 消费 `transportReconnectCandidate:livenessNoProgressTimeout`。
- 连续 rebuild 后 stats 反复回到 `inbound_video_frame_count_total=0`、`decode_fps=0`、`present_fps=0`，native host 只保留旧帧刷新，最终在退出前后进入 `displaySupplyStarved`。

当前代码路径：

- [`expensive_recovery_gate.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/expensive_recovery_gate.rs) 已为 `TransportAwaitRecoveryKeyframe` 增加 `mediaGate:connectedHealthyTransportAwait`，能挡住健康网络下的 `receiverWaitingKeyframe` reconnect。
- 同文件 `reconnect_block_reason()` 对 `VideoEscalationReason::LifecycleRecovering` 直接返回 `None`，`livenessNoProgressTimeout` 绕过健康网络 gate。
- [`recovery.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/recovery.rs) 将 `LifecycleRecovering` 的 reconnect proposal 解析为 `ConnectivityTransport`。
- [`lifecycle.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/runtime/lifecycle.rs) 只按 `reason_domain.allows_runtime_reconnect_candidate()` 放行，`ConnectivityTransport` 会触发实际 `request_reconnect(... Policy)`。

## Goal

- `livenessNoProgressTimeout` 在连接健康、已有 clean anchor / fresh media output / DisplayStable / continuous ReferenceChain / recent success edge 时进入 cooldown/local recovery。
- `livenessNoProgressTimeout` 只有在硬连接失败证据成立时进入 `ConnectivityTransport` 并触发 PeerConnection rebuild。
- trace 能直接看到阻断原因，例如 `reconnectBlocked:lifecycleGate:connectedHealthyNoProgress` 或复用健康连接 gate 语义。
- runtime domain gate 保持最后防线，上游调度负责给出正确 reason domain 与 gate detail。

## Scope

- In scope:
  - [`crates/xbxengine/core/src/transport/rtc/session/expensive_recovery_gate.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/expensive_recovery_gate.rs)
  - [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs)
  - [`crates/xbxengine/core/src/transport/rtc/policy/recovery.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/recovery.rs)
  - [`crates/xbxengine/core/src/transport/rtc/session/policy_tests/reconnect_lifecycle.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy_tests/reconnect_lifecycle.rs)
  - [`crates/xbxengine/core/src/api/runtime/runtime_tests/reconnect_recovery_matrix.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/runtime/runtime_tests/reconnect_recovery_matrix.rs)
- Out of scope:
  - 新增 transport 路线
  - 改远端编码器行为
  - 重做 receive feedback arbiter、NACK、PLI/FIR 主线
  - 改 runtime 最终 `ConnectivityTransport` domain gate 的基本职责

## Plan

1. 抽出可复用的健康连接 + 服务化输出判定：连接 fresh connected、ICE selected/nominated 且无 direct no-response、TWCC stable/local-feedback/loss 健康，且存在 clean anchor、fresh media output、DisplayStable、ReferenceChain continuous/repairing 或 recent success edge。
2. 在 `reconnect_block_reason()` 为 `LifecycleRecovering` 增加 `lifecycle_liveness_reconnect_block_reason()`：
   - `reason_label == "livenessNoProgressTimeout"` 时读取健康连接判定。
   - 健康连接 + 服务化输出成立时返回 `lifecycleGate:connectedHealthyNoProgress`。
   - lifecycle disconnected、fresh direct ICE no-response、recovering + connectivity failure evidence、transport severe/expired deadline 保持 reconnect 出口。
3. 在 proposal/domain 解析处保留硬连接证据的 `ConnectivityTransport`，对被 gate 降级的 proposal 记录 `CooldownSuppressed` 与 `reconnect_gate_detail`。
4. 补 trace-like 回归：
   - 已有 frame/output 成功，ICE/TWCC 健康，后续 no-progress 到 liveness timeout，期望无 `RequestReconnectCandidate` 命令，ledger 为 cooldown + lifecycle gate detail。
   - Disconnected / fresh direct ICE no-response / remote terminal 仍可触发 reconnect。
5. 运行定向验证并用当前 trace 的期望字段定义 fresh trace 验收。

## Validation

- [x] `cargo fmt`
- [x] `cargo test -p xbxengine lifecycle_liveness --lib -- --nocapture`
- [x] `cargo test -p xbxengine connected_healthy_twcc --lib -- --nocapture`
- [x] `cargo test -p xbxengine reconnect_lifecycle --lib -- --nocapture`
- [x] `cargo test -p xbxengine reconnect_recovery_matrix --lib -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::session::policy --lib`
- [x] `cargo check -p xbxengine`
- [x] `git diff --check`
- [x] `git diff --cached --check`
- [x] `PYTHONPYCACHEPREFIX=/private/tmp/codex_pycache python3 -m py_compile .agents/skills/analyze-runtime-logs/scripts/trace_lifecycle_reconnect_gate.py .agents/skills/analyze-runtime-logs/scripts/trace_webrtc_acceptance_gate.py`
- [x] `PYTHONPYCACHEPREFIX=/private/tmp/codex_pycache python3 -B -m unittest discover .agents/skills/analyze-runtime-logs/tests`（16 tests OK）
- [x] 负样本 gate：`python3 .agents/skills/analyze-runtime-logs/scripts/trace_lifecycle_reconnect_gate.py runtime-logs/runtime-trace-1782114876554-1.jsonl --require-lifecycle-block` 返回 `FAIL`，失败项为 `localRecoveryReconnectConsumedAfterHealthy`、`rebuildPeerConnectionAfterHealthy`、`missingLifecycleConnectedHealthyBlock`。
- [x] 负样本总验收：`python3 .agents/skills/analyze-runtime-logs/scripts/trace_webrtc_acceptance_gate.py runtime-logs/runtime-trace-1782114876554-1.jsonl --require-lifecycle-reconnect-gate` 返回 `FAIL`，receive/midsegment 通过但 lifecycle reconnect gate 失败。
- [ ] Fresh trace 期望：健康网络下出现 `reconnectBlocked:lifecycleGate:connectedHealthyNoProgress`，同窗口无 `rx closed cause=rebuildPeerConnection` storm。

## Risks

- gate 过宽会延迟真实硬断链恢复；缓解方式是只承认 fresh ICE/TWCC 健康证据，并保留 direct no-response、Disconnected、remote terminal 出口。
- `livenessNoProgressTimeout` 同时覆盖首帧前和重连后窗口；修正需分开处理 pre-first-frame progress token 与已服务化媒体输出。
- 当前工作区已有 staged transport/recovery 改动；实现需要在现有 diff 上增量修改，保持已有 receiverWaitingKeyframe gate 行为。

## Progress

- [x] Step 1: 完成最新 trace 分析与重建链路复盘。
- [x] Step 2: 定位 `LifecycleRecovering` 绕过 `connectedHealthyTransportAwait` 的代码路径。
- [x] Step 3: 已实现 lifecycle liveness gate。
- [x] Step 4: 已运行代码验证并更新任务/RFC。
- [ ] Step 5: 等待 fresh trace 做运行时验收。

## Completion Audit

- Requirement: 解释为什么健康网络下发生 `rebuildPeerConnection`。
  - Evidence: `runtime-trace-1782114876554-1.jsonl` 中 `seq=434` 显示 `present_fps=60.79 / decode_fps=60.79 / twccDelivery=1.0 / twccLoss=0.0`，后续 `runtimeReconnectConsumed` 将 `receiverWaitingKeyframe` / `livenessNoProgressTimeout` 消费成 `connectivity-transport`，并伴随 `rx closed cause=rebuildPeerConnection`。
  - Status: Proven.
- Requirement: 找到根本调度问题。
  - Evidence: `LifecycleRecovering/livenessNoProgressTimeout` 曾绕过 expensive gate；`RecoveryPolicyProposal` 对真实 reconnect candidate 保持 `ConnectivityTransport`，runtime domain gate 会消费并触发 reconnect。
  - Status: Proven.
- Requirement: 本地恢复/冷却信号保留在本地恢复层。
  - Evidence: `lifecycle_liveness_reconnect_block_reason()` 已覆盖健康连接 + 服务化输出、await success edge、control replay backlog、本地恢复活跃、连接失败证据缺口；`liveness_reconnect_attempts_without_progress` 只在 gate 后仍为 reconnect 时计数；`connected_healthy_lifecycle_liveness_reconnect_is_blocked` 等 Rust 回归通过。
  - Status: Code-level proven.
- Requirement: 保留真实硬连接失败出口。
  - Evidence: `lifecycle_liveness_direct_ice_no_response_still_reconnects`、`direct_ice_zero_response_probe_accelerates_pre_first_frame_reconnect`、runtime reconnect matrix 均通过。
  - Status: Code-level proven.
- Requirement: 运行时证明健康网络窗口不再出现 rebuild storm。
  - Evidence: `trace_webrtc_acceptance_gate.py --latest --max-age-seconds 900 --require-lifecycle-reconnect-gate` 已能选中新 trace `runtime-trace-1782180584210-1.jsonl` 且 `traceFreshness=PASS`，但该 trace 停留在 `transportState=New` / `startup:priming`，`sent/decoded/DisplayStable=0`，`healthyStatsCount=0`，缺少 WebRTC/media 窗口。
  - Status: Missing fresh trace evidence.

## Execution Notes

- Date: 2026-06-22 | Status: planned
- Update: 复盘 trace `runtime-trace-1782114876554-1.jsonl`，确认网络健康窗口后仍因 `livenessNoProgressTimeout` 被解析为 `ConnectivityTransport`，runtime 消费后触发 PeerConnection rebuild。
- Decision: 修正放在 session policy / expensive recovery gate；runtime domain gate 继续作为最后防线。
- Risk/Blocker: 需要用户确认后进入实现，符合 transport/recovery 边界复杂任务流程。

- Date: 2026-06-22 | Status: implemented
- Update: `LifecycleRecovering/livenessNoProgressTimeout` 已接入 expensive recovery gate。健康连接 + 服务化输出时降级为 cooldown，连续 reconnect 后缺少 success edge 时阻断 rebuild storm；fresh direct ICE no-response 仍保持 reconnect。
- Decision: liveness reconnect attempt 只在 gate 放行后计数，避免被阻断的候选累计成“已尝试重连”。
- Risk/Blocker: fresh trace 仍需验证运行时不再出现 `rx closed cause=rebuildPeerConnection` storm。

- Date: 2026-06-23 | Status: implemented
- Update: 复验通过：`cargo fmt`、`cargo test -p xbxengine lifecycle_liveness --lib -- --nocapture`、`cargo test -p xbxengine reconnect_lifecycle --lib -- --nocapture`、`cargo test -p xbxengine connected_healthy_twcc --lib -- --nocapture`、`cargo test -p xbxengine transport::rtc::session::policy --lib`、`cargo test -p xbxengine reconnect_recovery_matrix --lib -- --nocapture`、`cargo check -p xbxengine`、`git diff --check`、`git diff --cached --check`。
- Decision: 代码层验收覆盖健康 TWCC/ICE、lifecycle liveness cooldown、reconnect 后等待 success edge、direct ICE no-response 出口与 runtime domain gate。
- Risk/Blocker: goal 完成仍等待 fresh trace，要求健康网络窗口出现 lifecycle gate block 诊断，且无 `rx closed cause=rebuildPeerConnection` storm。

- Date: 2026-06-23 | Status: implemented
- Update: 新增 `trace_lifecycle_reconnect_gate.py`，用于 fresh trace 验收健康网络下的 rebuild regression。脚本会定位健康 TWCC/output 窗口，检查后续本地恢复原因是否被 runtime 消费为 reconnect，并统计 `rebuildPeerConnection` closure。
- Decision: 使用旧失败 trace `runtime-trace-1782114876554-1.jsonl` 做负样本，gate 返回 `FAIL`，证明它能抓住本次根因：健康窗口后 `receiverWaitingKeyframe/livenessNoProgressTimeout` 被消费成连接层 reconnect，并伴随 rebuild closure。
- Date: 2026-06-23 | Status: implemented
- Update: `trace_webrtc_acceptance_gate.py` 已接入 `--require-lifecycle-reconnect-gate`，总验收报告会同时输出 receive、midsegment 与 lifecycle reconnect gate。旧失败 trace 下 receive/midsegment 通过，但 lifecycle reconnect gate 失败，总 `acceptanceGate=FAIL`。
- Decision: fresh trace 最终使用总入口验收：`python3 .agents/skills/analyze-runtime-logs/scripts/trace_webrtc_acceptance_gate.py --latest --max-age-seconds 900 --require-lifecycle-reconnect-gate`。
- Risk/Blocker: 仍等待 fresh desktop streaming trace 证明运行时已经不再把健康网络下的本地恢复信号升级为连接层 rebuild。

- Date: 2026-06-23 | Status: implemented
- Update: 为 lifecycle reconnect gate 与总验收接入补单元测试：合成 PASS trace 要求 `reconnectBlocked:lifecycleGate:connectedHealthyNoProgress` 后总验收通过；合成 regression trace 包含健康 TWCC/output 后的 `runtimeReconnectConsumed:livenessNoProgressTimeout` 与 `rebuildPeerConnection`，总验收失败并报告三项 lifecycle failure。
- Decision: trace gate 的完成证据纳入 `python3 -B -m unittest discover .agents/skills/analyze-runtime-logs/tests`，当前 16 tests OK。
- Risk/Blocker: fresh trace 仍是 goal 完成的唯一缺口。

- Date: 2026-06-23 | Status: implemented
- Update: 启动 `pnpm tauri dev` 生成 fresh trace `runtime-trace-1782180584210-1.jsonl`，总验收 freshness 通过，但 trace 只包含启动采样与 `startup:priming`，没有 ICE/TWCC、receive feedback、decode/present、DisplayStable 或 rebuild 事件。
- Decision: 该 fresh trace 只能证明新 build 可启动与日志可写，不能作为健康网络 rebuild regression 的验收证据。
- Risk/Blocker: 需要一次真实 desktop streaming 会话，维持到出现健康 TWCC/output 窗口后再运行总 gate。
