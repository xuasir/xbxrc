---
name: analyze-runtime-logs
description: Analyze project-specific runtime trace logs written as JSONL. Use when Codex needs to inspect `runtime-logs/runtime-trace-*.jsonl` for incident triage, regression analysis, startup or streaming failures, cross-layer timeline reconstruction, receive feedback arbiter / ReferenceChain validation (`trace_receive_feedback_report.py`), or performance and stall evaluation in this Tauri + Vue 3 + TypeScript + Rust codebase.
---

# Analyze Runtime Logs

Use this skill for `runtime-logs/runtime-trace-*.jsonl` analysis in this repository.

## Quick Start

这些 `scripts/` 和 `references/` 路径都相对于当前 `SKILL.md` 所在目录。

1. Identify the target trace file or log directory.
2. Run [`scripts/summarize_runtime_trace.py`](scripts/summarize_runtime_trace.py) `<trace.jsonl>` to get phase anchors, long gaps, anomaly windows, and schema v3 profile/dimension/importance counts first.
3. If you need to isolate one session, subsystem, stage window, or metric, add `--session-id`, `--domain`, `--time-window`, `--phase`, and/or `--metric`.
4. To drop noisy rows before summarizing, add `--exclude-categories log` or `--categories state,decision,snapshot,event`.
5. To drill down around a row anchor, use `--anchor-seq <n>` with optional `--context-before` / `--context-after` (prints JSONL lines to stdout).
6. Use `--compare <other-trace.jsonl>` when you need a before/after regression check for the same phase window.
7. For long-session steady-state acceptance gates (e.g. low-latency display scheduling), run [`scripts/trace_midsegment_report.py`](scripts/trace_midsegment_report.py) on the trace; default window auto-anchors at the first `statsSnapshot sessionPhase=steady` after +79s and keeps the historical 71s duration (`--start-s` / `--end-s` force a manual window). Exit code `0` = heuristic PASS, `2` = GATE FAIL.
8. For receive feedback arbiter / ReferenceChain regressions (NACK·PLI·FIR 收口、`receiveFeedbackDecision`), run [`scripts/trace_receive_feedback_report.py`](scripts/trace_receive_feedback_report.py) on the trace. Prints JSON with `feedbackActionCounts`, `keyframeOutcomeCounts`, `referenceStateCounts`, `arbiterMismatchTotal`, `needKeyframeNonIdrFeedViolations`, projection-layer gates (`displayStableWithoutLedgerClosure`, `insertSurfacePhaseActionStage`, `insertControlProjectionMismatch`), and `keyframeChain` (sent → displayStable). Use `--fail-on-gate --require-media-recovered --require-display-stable` for fresh-trace acceptance. Exit `0` on success and `2` when an enabled gate fails; old traces without `receiveFeedbackDecision` still run but `summary.receiveFeedbackDecisionEvents` may be `0`.
9. For final WebRTC acceptance, run [`scripts/trace_webrtc_acceptance_gate.py`](scripts/trace_webrtc_acceptance_gate.py) `<trace.jsonl>`. It combines strict receive recovery (`receiveFeedbackGate + mediaRecovered + DisplayStable`) and midsegment low-latency steady-state gates. Add `--require-lifecycle-reconnect-gate` when validating healthy-network `rebuildPeerConnection` regressions. Exit `0` = full acceptance PASS; exit `2` = receive, midsegment, or lifecycle reconnect gate failed. Use `--latest --max-age-seconds <seconds>` after a new desktop session to select the newest `runtime-logs/runtime-trace-*.jsonl` and fail stale evidence with `traceFreshness.freshnessGate=FAIL`.
9a. For Rust H264 browser-profile fallback validation, run [`scripts/trace_h264_profile_fallback_gate.py`](scripts/trace_h264_profile_fallback_gate.py) `<trace.jsonl>` with `python3 -B`. Add `--require-fallback` to require the `4d/64 -> 42e` path; without it, the gate also accepts high-profile startup success. Use `--latest --max-age-seconds <seconds>` after a fresh desktop run.
9b. For browser WebRTC behavior sampling, run [`scripts/trace_browser_webrtc_behavior_report.py`](scripts/trace_browser_webrtc_behavior_report.py) `<trace.jsonl>` with `python3 -B`. It reports SDP stages, selected H264 answer profile, peer/ICE/signaling timeline, first inbound packet / decoded / keyframe decoded / presented milestones, and PLI/FIR/NACK deltas. Use `--latest --max-age-seconds <seconds>` after a fresh browser-direct run.
10. For healthy-network `rebuildPeerConnection` regressions, run [`scripts/trace_lifecycle_reconnect_gate.py`](scripts/trace_lifecycle_reconnect_gate.py) `<trace.jsonl>`. It verifies a healthy TWCC/output window, fails if local recovery reasons such as `receiverWaitingKeyframe` or `livenessNoProgressTimeout` are consumed as runtime reconnects after that window, and fails on `rx closed cause=rebuildPeerConnection` after healthy playback. Use `--require-lifecycle-block` for fresh traces that must prove the new lifecycle gate emitted `reconnectBlocked:lifecycleGate:connectedHealthyNoProgress`.
11. For schema v3 traces, read `traceProfile` first (`production` = key/essential evidence under budget, `dev` = detailed diagnostics), then use `dimension` and `importance` counts to decide whether missing evidence is instrumentation loss, profile filtering, or a real absence.
12. Treat `traceBudgetNotice` as evidence that debug/raw rows were dropped under writer queue pressure; key/essential rows are the reliable evidence lane.
13. Read [`references/log-schema.md`](references/log-schema.md) when you need field semantics (including `traceMode`, `traceProfile`, `dimension`, and `importance`).
14. Read [`references/analysis-playbook.md`](references/analysis-playbook.md) when you need the project-specific workflow, output contract, or heuristics.
15. Re-open the raw trace around the key `seq` / `tsMs` window before making conclusions.
16. For recovery regressions, always read these structured events first:
   - `pictureRecoveryTransition`
   - `pictureRecoveryBlockerObserved`
   - `videoIngressTermination`
   - `firstFrameLatencyObserved`
17. Treat the recovery mainline as:
   - `PliRequested -> PliSent -> ResponseObserved/PacketSeen -> Decoded -> CleanAnchorCommitted -> DisplayStable`
18. Read the two recovery gates with fixed semantics:
   - `cleanAnchorCommitted`: media gate，表示 decode 后的恢复锚点已经被下游真正接住
   - `DisplayStable`: display gate，表示显示侧稳定闭环成立
19. Read `stableServingSettled` as the `DisplayStable` close reason / event name.
20. For receive-feedback arbiter traces, read in this order after running step 8:
   - **控制事实（决策权）**：`keyframeRequired` / `responseState` / `receiveDisplayState` / `ledgerGeneration` / `packetRecoveryActionStage`
   - **诊断投影（仅 trace/UI）**：`mediaSupplyPhaseDiagnostic` / `displayedIdrHostHint` / `displayedIdrHostHintDiagnostic`
   - `receiveFeedbackDecision` (`action`, `reason`, `coalescing`, `outcome`, `lastKeyframeSentAgeMs`, `referenceState`, control facts above)
   - `referenceChainStateChanged` (`decoderReferenceSynced`, `bootstrapReady`, `bootstrapRejectReason`, `hasActiveGap`, `nackExhausted`, `submitAgeMs`, control facts; `displayedIdrHostHint` 仅诊断)
   - `receivePictureRecoveryTerminal` (`reason=remote-no-usable-idr` when远端长期无 usable IDR)
   - `keyframeRequestOutcome` (executor outcome: `sent` / `coalesced` / `throttled` / `feedbackUnavailable`)
   - `insertGateDecision` (control facts + `needKeyframeNonIdrFeedViolations` / `insertControlProjectionMismatch` in script output)
   - Script JSON: `rates.*`, `receiveFeedbackGate` (`PASS`/`FAIL`), `displayStableWithoutLedgerClosure`, `terminalRemoteNoUsableIdr`
21. For recovery regressions, always read the script's `recovery_audit.keyframeEffectiveness` and `recovery_audit.nackEffectiveness`, not only aggregate request counters.
22. For recovery quality scoring, also read:
   - `recovery_audit.keyframeEffectiveness.chainBuildSuccessRate`
   - `recovery_audit.nackEffectiveness.effectiveRate`
   - `recovery_audit.repairabilityPersistence`
   - `recovery_audit.recoveryEffectiveness`
23. Read post-decode scheduling with fixed ownership:
   - `pacer*`: decode 后唯一主决策层
   - `renderMailbox*`: render latest-slot 的单槽交接 / overwrite 执行态
   - `hostMailbox*`: host pending/displayed mailbox 与上屏执行态
24. Do not read `renderMailboxStateTransition` or `renderMailboxDecision` as a second value-comparator.
    They report mailbox overwrite / recovery telemetry after `pacer` has already chosen the frame.
25. For browser-direct render pacing / WebGL2 drawing questions, always read these browser-side structured events first:
   - `renderTelemetryObserved`
   - `renderFrameDropped`
   - `renderBackpressureChanged`
   - `renderCauseClassified`
   - `renderPolicyApplied`
26. Read browser-direct render telemetry with fixed semantics:
   - `trackingSource`: `videoFrameCallback` 表示基于 `requestVideoFrameCallback`，`timeupdate` 表示 fallback 粗粒度节拍
   - `callbackCountSinceLastSample` / `frameEventsSinceLastSample`: sample 窗口内浏览器回调次数；两者当前等价，后者是历史兼容字段
   - `callbackGapCountSinceLastSample`: sample 窗口内“回调间隔超过本地阈值”的次数；优先拿它判断 callback 稀疏/晚到
   - `presentedFramesAdvancedSinceLastSample`: sample 窗口内浏览器 `presentedFrames` 总推进量；拿它和 callback count 对比，区分“50Hz 回调 + 60Hz 呈现推进”与“真实显示不足 60Hz”
   - `presentedFramesDelta`: 单次回调跨度内浏览器已呈现帧数变化，`>1` 表示该窗口内有跳帧/批量呈现迹象
   - `presentedFramesJumpCountSinceLastSample`: sample 窗口内 `presentedFramesDelta > 1` 的次数；优先拿它判断多帧推进被浏览器合并到一次 callback
   - `mediaTimeDeltaSec`: 相邻 video-frame callback 之间源 `mediaTime` 推进量；判断源节拍是否真在 30/60fps 附近
   - `expectedDisplayLeadMs`: callback 到达时距离浏览器预计显示时间的提前/滞后量；负值表示回调已晚于预计显示点
   - `sourceFpsEstimate` / `sourceFrameIntervalMs`: 视频源节拍估算，优先用来区分 30fps / 60fps 源与本地绘制问题
   - `droppedFramesSinceLastSample` / `droppedLikeStreak`: 浏览器侧 dropped-like 并集计数与连续性；它同时覆盖 callback gap 和 `presentedFrames` jump，不能单独当成真实掉帧结论
   - `maxCallbackIntervalMsSinceLastSample` / `maxPresentedFramesDeltaSinceLastSample`: 当前 sample 窗口内最差回调间隔与最大跳帧跨度
27. Treat `renderFrameDropped` as browser-side dropped-like evidence, not literal GPU draw failure.
    It means callback cadence or presented-frame progression crossed the local threshold.
    Use `callbackGap` and `presentedFramesJump` to split “callback 稀疏” from “多帧合批推进”.

## Follow This Workflow

1. Confirm the user goal: incident triage, regression comparison, startup failure, streaming fault, or performance review.
2. Establish the time window: first row, last row, duration, active `sessionId`, and major phase boundaries.
3. Separate `state` / `decision` / `snapshot` rows from `log` noise before chasing individual messages.
4. Use the script output to locate phase windows, long gaps, anomaly clusters, and compare deltas before reconstructing the fine-grained timeline.
5. Reconstruct the primary timeline with concrete `seq`, `tsMs`, `domain`, and `event` anchors.
6. Identify the first abnormal signal, not only the final failure symptom.
7. Correlate front-end, Tauri, service, and `xbxengine` observations before inferring causality.
8. Distinguish evidence from hypothesis. State uncertainty explicitly when the trace is incomplete.
9. Recommend the next diagnostic step only after listing the missing evidence.
10. When the question involves keyframe or NACK behavior, separate:
   - attempted vs suppressed/coalesced
   - sent vs response observed vs packet seen vs decoded
   - decoded vs `cleanAnchorCommitted` vs `DisplayStable`
   - recovered vs recovered late vs skipped vs expired
11. When the question involves recovery, read the structured chain in this order:
   - `pictureRecoveryTransition`: recovery phase progression mainline
   - `pictureRecoveryBlockerObserved`: current gate blocker and accumulation
   - `videoIngressTermination`: `RtcVideoFrameSource rx closed` and upstream-cause chain
   - `firstFrameLatencyObserved`: first-frame five-stage latency breakdown
   - `h264InspectionObserved` / `h264InspectionRejected`: packet-level H264 bootstrap verdict
12. When the question involves browser-direct render stability, read the structured chain in this order:
   - `renderTelemetryObserved`: sample 窗口总览
   - `renderFrameDropped`: 单次 dropped-like 证据
   - `renderBackpressureChanged`: 本地 backpressure 起落点
   - `renderCauseClassified`: 本地 render 原因归类
   - `renderPolicyApplied`: 当前 WebGL2 / video / SR 路径与 display degrade 策略结果

## Use The Bundled Resources

- Use [`scripts/summarize_runtime_trace.py`](scripts/summarize_runtime_trace.py) first for row counts, domains, sessions, log levels, schema v3 `traceProfile/dimension/importance` counts, budget notices, and suspicious rows.
- Use [`scripts/trace_midsegment_report.py`](scripts/trace_midsegment_report.py) for mid-session steady-state gates: `statsSnapshot` steady ratio, recovering / `receiverWaitingKeyframe` pulses, `submit_age_ms` / `present_age_ms` P95, and `hostMailboxRetainedDisplayed` + `hasPendingFrame` anomalies. The default window skips startup by anchoring at the first steady snapshot after +79s; use `--start-s` / `--end-s` for fixed-window regression comparisons. Prints `GATE: PASS` or `GATE: FAIL` and exits `0` / `2`.
- Use [`scripts/trace_receive_feedback_report.py`](scripts/trace_receive_feedback_report.py) for receive feedback arbiter acceptance: aggregates `receiveFeedbackDecision`, `keyframeRequestOutcome`, `referenceChainStateChanged`, `receivePictureRecoveryTerminal`, arbiter mismatch, `NeedKeyframe` non-IDR feed violations, projection-layer gates (`displayStableWithoutLedgerClosure`, `insertSurfacePhaseActionStage`, `insertControlProjectionMismatch`), `rates` (response/decoded/clean-anchor/display/usable-IDR/chain-build), and `receiveFeedbackGate` (`PASS`/`FAIL`). Add `--fail-on-gate --require-media-recovered --require-display-stable` when the command itself should fail stale or incomplete traces. `nackEffectiveRate` 在 trace 无统一 NACK 事件时为 `null`，可改用 `summarize_runtime_trace.py` 的 `recovery_audit.nackEffectiveness`.
- Use [`scripts/trace_webrtc_acceptance_gate.py`](scripts/trace_webrtc_acceptance_gate.py) for the final combined gate. It runs the strict receive feedback report and the midsegment report, then prints a compact JSON with `acceptanceGate`, receive failures, keyframe chain, and midsegment gate states.
- Read [`references/log-schema.md`](references/log-schema.md) for the JSONL envelope, row categories, and interpretation rules.
- Read [`references/analysis-playbook.md`](references/analysis-playbook.md) for project-specific analysis steps, common focus areas, and reporting format.
- The script now surfaces structured recovery timeline anchors:
  - `pictureRecoveryTransition`: recovery phase progression samples and latest transitions.
  - `pictureRecoveryBlockerObserved`: blocker gate / blocker kind / severity aggregation and samples.
  - `videoIngressTermination`: ingress termination causal chain samples, including upstream cause linkage.
  - `firstFrameLatencyObserved`: first-frame five-stage latency samples and terminal phase.
- The script now surfaces structured recovery effectiveness:
  - `keyframeEffectiveness`: request suppression, sent/seen/decoded progression, invalid H264 response, decoded-after-success but chain not rebuilt, and effective recovery count.
  - `nackEffectiveness`: `nackSent` / `nackRecovered` / `nackSkipped` / `nackExpired`, plus disposition and unrecoverable-reason breakdown.
  - `keyframeEffectiveness.chainBuildSuccessRate`: 建链成功率聚合统计（`chainRecoveredCount / decodedCount`）。
  - `nackEffectiveness.effectiveRate`: NACK 有效性聚合统计（`effectiveCount / sentCount`）。
  - `repairabilityPersistence`: repairability 评分样本、均值区间、缺口长度、缺失连续段等持久化统计。
  - `recoveryEffectiveness`: 综合恢复有效性评分（由 keyframe、建链、NACK、repairability 持久化加权得到）。
- Browser-direct render telemetry currently relies on raw structured events in the trace:
  - `renderTelemetryObserved`: 浏览器直连绘制 sample 汇总
  - `renderFrameDropped`: 单次 dropped-like 事件
  - `renderBackpressureChanged`: 本地 backpressure 门限跨越
  - `renderCauseClassified`: render starvation / decode backpressure / stable 分类
  - `renderPolicyApplied`: renderer attach / display degrade / shader 路径落地结果

## Reporting Contract

Return results in this shape unless the user asked for a different format:

1. Scope: which trace file(s) and what question you answered.
2. Timeline: the smallest useful sequence of events with `seq` / `tsMs` anchors.
3. Key Signals: failure, timeout, stall, retry, degradation, or recovery evidence.
4. Likely Cause: the highest-confidence explanation and competing hypotheses.
5. Evidence: concrete rows or field changes that support the conclusion.
6. Gaps: what the trace cannot prove yet.
7. Next Actions: code paths, extra logs, or experiments to run next.
8. If keyframe or NACK is involved, explicitly state:
   - whether the request was suppressed/coalesced
   - whether a usable response arrived
   - whether decode succeeded
   - whether `cleanAnchorCommitted` happened
   - whether `DisplayStable` happened
   - whether NACK was effective, late, skipped, or expired
   - chain build success rate 聚合值
   - NACK effective rate 聚合值
   - repairability 持久化统计是否连续
   - recovery effectiveness 综合评分及其主要分项
9. If recovery is involved, list evidence in this order:
   - `pictureRecoveryTransition`
   - `pictureRecoveryBlockerObserved`
   - `videoIngressTermination`
   - `firstFrameLatencyObserved`
   - `h264InspectionObserved` / `h264InspectionRejected` as packet-level supplement
10. If browser-direct render stability is involved, explicitly state:
   - `trackingSource` 是 `videoFrameCallback` 还是 `timeupdate`
   - `sourceFpsEstimate` / `sourceFrameIntervalMs` 是否稳定
   - `callbackIntervalMs` 与 `maxCallbackIntervalMsSinceLastSample` 是否出现长尾
   - `callbackGapCountSinceLastSample` 是否增长，以及它和 `presentedFramesAdvancedSinceLastSample` 是否互相矛盾
   - `presentedFramesDelta` / `maxPresentedFramesDeltaSinceLastSample` 是否持续大于 `1`
   - `presentedFramesJumpCountSinceLastSample` 是否增长
   - `droppedFramesSinceLastSample` 与 `droppedLikeStreak` 是否连续增长
   - `renderBackpressure` 是否仅短时出现，还是长期维持
   - `renderCauseClassified` 与 `renderPolicyApplied` 是否把问题收敛为本地绘制、解码背压或稳定状态

## Guardrails

- Prefer `state`, `decision`, and `snapshot` rows over raw `log` chatter when both exist.
- Do not treat the last visible error as the root cause without checking earlier state drift.
- Do not collapse symptom and cause into one statement.
- Quote only the minimum needed from raw logs; summarize the rest.
- Flag missing instrumentation when the trace cannot prove a causal link.
- Compare multiple traces only after normalizing by phase, `sessionId`, and timestamp window.
- Do not treat `recovery_keyframe_request_count` or raw `nack` counts as outcome metrics; use episode/disposition/effectiveness fields first.
- Do not treat `renderFrameDropped` count alone as proof of GPU draw failure; always correlate it with `trackingSource`, `callbackGap`, `presentedFramesJump`, `sourceFpsEstimate`, `presentedFramesDelta`, and `renderBackpressure`.
