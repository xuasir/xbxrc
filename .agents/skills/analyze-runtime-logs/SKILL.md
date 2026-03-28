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
3. If you need to isolate one session or one subsystem, add `--session-id` and/or `--domain` before reading more rows.
4. Read [`references/log-schema.md`](references/log-schema.md) when you need field semantics.
5. Read [`references/analysis-playbook.md`](references/analysis-playbook.md) when you need the project-specific workflow, output contract, or heuristics.
6. Re-open the raw trace around the key `seq` / `tsMs` window before making conclusions.

## Follow This Workflow

1. Confirm the user goal: incident triage, regression comparison, startup failure, streaming fault, or performance review.
2. Establish the time window: first row, last row, duration, active `sessionId`, and major phase boundaries.
3. Separate `state` / `decision` / `snapshot` rows from `log` noise before chasing individual messages.
4. Use the script output to locate phase windows, long gaps, and anomaly clusters before reconstructing the fine-grained timeline.
5. Reconstruct the primary timeline with concrete `seq`, `tsMs`, `domain`, and `event` anchors.
6. Identify the first abnormal signal, not only the final failure symptom.
7. Correlate front-end, Tauri, service, and `xbxengine` observations before inferring causality.
8. Distinguish evidence from hypothesis. State uncertainty explicitly when the trace is incomplete.
9. Recommend the next diagnostic step only after listing the missing evidence.

## Use The Bundled Resources

- Use [`scripts/summarize_runtime_trace.py`](scripts/summarize_runtime_trace.py) first for row counts, domains, sessions, log levels, and suspicious rows.
- Read [`references/log-schema.md`](references/log-schema.md) for the JSONL envelope, row categories, and interpretation rules.
- Read [`references/analysis-playbook.md`](references/analysis-playbook.md) for project-specific analysis steps, common focus areas, and reporting format.

## Reporting Contract

Return results in this shape unless the user asked for a different format:

1. Scope: which trace file(s) and what question you answered.
2. Timeline: the smallest useful sequence of events with `seq` / `tsMs` anchors.
3. Key Signals: failure, timeout, stall, retry, degradation, or recovery evidence.
4. Likely Cause: the highest-confidence explanation and competing hypotheses.
5. Evidence: concrete rows or field changes that support the conclusion.
6. Gaps: what the trace cannot prove yet.
7. Next Actions: code paths, extra logs, or experiments to run next.

## Guardrails

- Prefer `state`, `decision`, and `snapshot` rows over raw `log` chatter when both exist.
- Do not treat the last visible error as the root cause without checking earlier state drift.
- Do not collapse symptom and cause into one statement.
- Quote only the minimum needed from raw logs; summarize the rest.
- Flag missing instrumentation when the trace cannot prove a causal link.
- Compare multiple traces only after normalizing by phase, `sessionId`, and timestamp window.
