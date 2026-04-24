---
name: similar-work
description: Find similar historical tasks, RFCs, reports, and validation patterns in this repository. Use when preparing a new implementation and you want to reuse prior solutions, avoid repeating mistakes, or borrow naming, rollout, and validation structure from earlier work.
---

# Similar Work

Use this skill to mine prior work patterns before starting a new task.

## Workflow

1. Extract the new task's key signals: module, type of change, symptoms, architecture terms, and validation needs.
2. Search `docs/project-task.md` for close task-line matches.
3. Search `docs/rfcs/` and `docs/reports/` for matching change patterns.
4. Rank results by structural similarity, not only keyword overlap.
5. Summarize the most reusable precedent.

## Reporting Contract

Return:

1. Top similar tasks
2. Reusable approaches
3. Relevant RFCs and reports
4. Historical pitfalls
5. Validation patterns worth reusing

## Guardrails

- Prefer structurally similar work over merely sharing one term.
- Surface the limits of reuse when the new task changes scope or constraints.
- Keep the result actionable.
