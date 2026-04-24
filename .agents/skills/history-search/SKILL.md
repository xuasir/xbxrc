---
name: history-search
description: Search the repository's task-tracking documents for topic history. Use when onboarding a topic, checking whether similar work already exists, reviewing recent progress, or reconstructing the task, RFC, report, and ISU trail behind a keyword, feature, or issue.
---

# History Search

Use this skill to turn the task-tracking corpus into working context.

## Search Scope

Search these sources first:

- `docs/project-task.md`
- `docs/rfcs/`
- `docs/reports/`
- `docs/isu/`

Prefer `rg -n` and `rg --files` over manual scrolling.

## Search Workflow

1. Extract the main topic, aliases, module names, and domain terms.
2. Search `docs/project-task.md` for task-line hits.
3. Search RFCs, reports, and ISU notes for richer context.
4. Group results into active work, completed work, and exploratory work.
5. Return the smallest useful reading path.

## Reporting Contract

Return results in this shape:

1. Conclusion
2. Relevant tasks
3. Relevant RFCs and reports
4. Timeline
5. Current active constraints
6. Recommended next read or next action

## Guardrails

- Prefer repository documents over memory when concrete history matters.
- Distinguish active work from superseded work.
- Quote minimally and summarize the rest.
