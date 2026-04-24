---
name: task-housekeeping
description: Maintain the lightweight task-tracking documents for this repository in the background. Use when Codex needs to tidy `docs/project-task.md`, normalize task-line shape, archive stale completed entries, or keep task tracking aligned with actual execution without burdening the developer.
---

# Task Housekeeping

Use this skill as a low-visibility maintenance pass for task tracking.

## Responsibilities

- Keep `docs/project-task.md` compact and readable.
- Preserve the lightweight task-line format for small tasks.
- Keep complex-task references to RFCs and reports coherent.
- Archive stale completed items according to the repository's tracking rules.

## Workflow

1. Scan `docs/project-task.md` for formatting drift, stale completed items, and missing references.
2. Normalize only the minimum needed to restore consistency.
3. Keep active work visible.
4. Move or archive old completed work only in deliberate maintenance passes.
5. Avoid broad cosmetic rewrites.

## Guardrails

- Prefer local, low-churn edits.
- Avoid reordering active tasks unless the tracking model explicitly requires it.
- Do not change task meaning while cleaning formatting.
- Treat this as maintenance, not as planning or implementation.
