---
name: analyze-runtime-logs
description: Analyze project-specific runtime trace logs written as JSONL. Use when Codex needs to inspect `runtime-logs/runtime-trace-*.jsonl` for incident triage, regression analysis, startup or streaming failures, cross-layer timeline reconstruction, or performance and stall evaluation in this Tauri + Vue 3 + TypeScript + Rust codebase.
---

# Analyze Runtime Logs

Use this skill for `runtime-logs/runtime-trace-*.jsonl` analysis in this repository.

## Quick Start

这些 `scripts/` 和 `references/` 路径都相对于当前 `SKILL.md` 所在目录。

1. Identify the target trace file or log directory.
2. Run [`scripts/summarize_runtime_trace.py`](scripts/summarize_runtime_trace.py) `<trace.jsonl>` to get phase anchors, long gaps, and anomaly windows first.
3. If you need to isolate one session, subsystem, stage window, or metric, add `--session-id`, `--domain`, `--time-window`, `--phase`, and/or `--metric`.
4. To drop noisy rows before summarizing, add `--exclude-categories log` or `--categories state,decision,snapshot,event`.
5. To drill down around a row anchor, use `--anchor-seq <n>` with optional `--context-before` / `--context-after` (prints JSONL lines to stdout).
6. Use `--compare <other-trace.jsonl>` when you need a before/after regression check for the same phase window.
7. Read [`references/log-schema.md`](references/log-schema.md) when you need field semantics (including `traceMode` on schema v2+).
8. Read [`references/analysis-playbook.md`](references/analysis-playbook.md) when you need the project-specific workflow, output contract, or heuristics.
9. Re-open the raw trace around the key `seq` / `tsMs` window before making conclusions.
10. For recovery regressions, always read these structured events first:
   - `pictureRecoveryTransition`
   - `pictureRecoveryBlockerObserved`
   - `videoIngressTermination`
   - `firstFrameLatencyObserved`
11. Treat the recovery mainline as:
   - `PliRequested -> PliSent -> ResponseObserved/PacketSeen -> Decoded -> CleanAnchorCommitted -> DisplayStable`
12. Read the two recovery gates with fixed semantics:
   - `cleanAnchorCommitted`: media gate，表示 decode 后的恢复锚点已经被下游真正接住
   - `DisplayStable`: display gate，表示显示侧稳定闭环成立
13. Read `stableServingSettled` as the `DisplayStable` close reason / event name.
14. For recovery regressions, always read the script's `recovery_audit.keyframeEffectiveness` and `recovery_audit.nackEffectiveness`, not only aggregate request counters.
15. For recovery quality scoring, also read:
   - `recovery_audit.keyframeEffectiveness.chainBuildSuccessRate`
   - `recovery_audit.nackEffectiveness.effectiveRate`
   - `recovery_audit.repairabilityPersistence`
   - `recovery_audit.recoveryEffectiveness`

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

## Use The Bundled Resources

- Use [`scripts/summarize_runtime_trace.py`](scripts/summarize_runtime_trace.py) first for row counts, domains, sessions, log levels, and suspicious rows.
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

## Guardrails

- Prefer `state`, `decision`, and `snapshot` rows over raw `log` chatter when both exist.
- Do not treat the last visible error as the root cause without checking earlier state drift.
- Do not collapse symptom and cause into one statement.
- Quote only the minimum needed from raw logs; summarize the rest.
- Flag missing instrumentation when the trace cannot prove a causal link.
- Compare multiple traces only after normalizing by phase, `sessionId`, and timestamp window.
- Do not treat `recovery_keyframe_request_count` or raw `nack` counts as outcome metrics; use episode/disposition/effectiveness fields first.
