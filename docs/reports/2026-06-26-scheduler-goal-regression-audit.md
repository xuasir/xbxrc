# Scheduler Goal Regression Audit Report

## Summary

当前调度主线保持原目标：参考 WebRTC 的恢复责任边界，由 receive / recovery ledger 掌握 picture recovery 事实主权；decode 后由 pacer 统一做低延迟 latest-only 价值决策；renderer / native host 执行 bounded mailbox 与本地自愈。

本轮代码级回归覆盖 receive / InsertGate、session reconnect、ingress backpressure、decode output mailbox、pacer、native scheduling。结论是主线方向成立，已修正两个局部偏差：continuous reference 下过度 `activeRepairHold`，以及 PS/config strict 对 `DisplayStable` 的反向依赖。

## Scope

- `crates/xbxengine/core/src/transport/rtc/recovery/contract/insert_control.rs`
- `crates/xbxengine/core/src/transport/rtc/receive/insert_gate.rs`
- `crates/xbxengine/core/src/api/runtime/lifecycle.rs`
- `crates/xbxengine/core/src/transport/rtc/session/expensive_recovery_gate.rs`
- `crates/xbxengine/core/src/transport/rtc/receive/rtx_sink.rs`
- `crates/xbxengine/core/src/transport/rtc/receive/ingress_state/mod.rs`
- `crates/xbxengine/core/src/media/video/decode/actor.rs`
- `crates/xbxengine/core/src/media/video/decode/video_decode.rs`
- `crates/xbxengine/core/src/media/video/pacer/actor.rs`
- `src-tauri/src/mods/native_video/scheduling.rs`

## Findings

- receive / InsertGate：`Continuous + decoder_reference_synced` 时，`NackPending` / `NackMissed` / `WaitKeyframe` / `RequestIdr` 归一为 `Steady`，continuation 可继续低延迟入 decode；真实 `Repairing` active gap 继续触发 `activeRepairHold`，保护修复窗口。
- PS/config strict：当前由 fresh IDR admission、decoder reference sync、parameter set change window 驱动，控制事实回到 packet / reference / decoder evidence。
- reconnect：session policy 与 runtime lifecycle 都保留 domain gate；`receiverWaitingKeyframe` / `livenessNoProgressTimeout` 在 connected healthy transport 与 serviceable media 下被 `connectedHealthyNoProgress` gate 吸收。
- display 自愈：`hostPresentStalled` / `displaySupplyCritical` / `displaySupplyDegraded` 只触发 runtime 本地 presenter recovery，且受 present loop、no-pending host mailbox、fresh post-decode evidence guard 限制。
- ingress：生产 video RTP ingress 固定 64 包通道；`RtcVideoSourceSink` 对 oversized sender 继续用 64 作为有效上限；best-effort 在 48 包软水位进入本地队列；priority / repair / best-effort backlog 保新丢旧。
- decode：decode actor input mailbox 为 2，decode output mailbox 为 current + latest；present pipeline stressed 时提供短突发四槽；pacer 满时 decoded output 在边界 coalesce/drop，decode 继续拉 ingress。
- pacer：pacer 是 decode 后唯一价值决策层；内部 `current_release + latest_release_candidate`，render queue 为 1，recovery / priming 为 2；Ready 提交，Drop 丢弃，Hold 等待。
- native scheduling：host mailbox 保持 latest pending；media epoch 切换清理旧 displayed / pending；stale retained displayed 停止刷新 present freshness。

## Validation

- `cargo test -p xbxengine reconnect_lifecycle --lib -- --nocapture`
- `cargo test -p xbxengine reconnect_recovery_matrix --lib -- --nocapture`
- `cargo test -p xbxengine transport::rtc::receive::rtx_sink --lib -- --nocapture`
- `cargo test -p xbxengine transport::rtc::receive --lib -- --nocapture`
- `cargo test -p xbxengine media::video::decode --lib -- --nocapture`
- `cargo test -p xbxengine media::video::pacer::actor --lib -- --nocapture`
- `cargo test -p xbxrc --lib native_video::scheduling -- --nocapture`
- `cargo check -p xbxengine`
- `cargo check -p xbxrc`
- `git diff --check`

## Runtime Evidence

最新 runtime trace 仍是旧样本：`runtime-logs/runtime-trace-1782364737067-1.jsonl`。

`trace_webrtc_acceptance_gate.py --latest --max-age-seconds 900 --require-lifecycle-reconnect-gate --require-ingress-queue-gate` 输出：

- `traceFreshness.freshnessGate=FAIL`
- `acceptanceGate=FAIL`
- `receiveFeedbackGate=PASS`
- `midsegment.globalLatencyGate=FAIL`
- `ingressQueueGate=FAIL`
- `localBackpressureBestEffortOverflow streak=466`
- `maxSenderQueueLimit=64`
- `maxSenderQueueDepth=64`
- `maxTotalQueueDepth=86`
- `runtimeLocalReconnectConsumed=0`
- `rebuildPeerConnectionClosureCount=0`

该旧样本证明修复前故障形态：receive recovery 已闭合，剩余压力集中在 display / local backpressure / latency gate。

## Fresh Trace Acceptance

当前构建实机验收建议使用：

```bash
python3 .agents/skills/analyze-runtime-logs/scripts/trace_webrtc_acceptance_gate.py --latest --max-age-seconds 900 --require-lifecycle-reconnect-gate --require-ingress-queue-gate
```

验收信号：

- `traceFreshness.freshnessGate=PASS`
- `acceptanceGate=PASS`
- `activeRepairHold` 在 `referenceState=continuous` 下归零或显著下降
- `insertGateDecision emit/decodableToFeed` 上升
- `keyframeRequestOutcome coalesced reason=insert-gate-supply-break` 下降
- `hostMailboxEnqueue` / `hostFramePresent` 持续推进
- `localBackpressureBestEffortOverflow` 连续段收敛
- `midsegment.globalLatencyGate=PASS`
- `runtimeLocalReconnectConsumed=0`
- `rebuildPeerConnectionClosureCount=0`

## Recommendation

当前代码继续沿原目标推进。下一步验收以当前构建 fresh trace 为准，重点看 continuous reference 插入、ingress overflow streak、submit / present P95、host mailbox 连续性与 reconnect 消费计数。
