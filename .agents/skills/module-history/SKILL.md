---
name: module-history
description: Reconstruct the history and current constraints of a code module or path by reading the repository's task-tracking documents. Use when starting work in a module and you need the relevant tasks, RFCs, reports, and still-active constraints before editing code.
---

# Module History

Use this skill before changing an unfamiliar or high-risk module.

## Inputs

Take one or more of:

- module path
- crate name
- frontend directory
- conceptual subsystem name

## Workflow

1. Normalize the target into search terms, path segments, and subsystem names.
2. Search `docs/project-task.md` for direct mentions and nearby wording.
3. Search `docs/rfcs/`, `docs/reports/`, and `docs/isu/` for the same terms and adjacent architecture language.
4. Separate historical decisions from still-active work.
5. Extract the constraints that should influence the next edit.

## Reporting Contract

Return:

1. Module summary
2. Key historical tasks
3. Important RFCs and reports
4. Current active work
5. Constraints to preserve while editing
6. Suggested starting files or docs

## Guardrails

- Prefer module-relevant history over broad project summaries.
- Call out unresolved active work explicitly.
- Treat recent RFCs and reports as higher-confidence than old exploratory notes.
