---
name: task-merge-guard
description: Resolve common concurrent-edit and merge-conflict cases around the repository's lightweight task-tracking documents. Use when `docs/project-task.md` has overlapping edits from multiple local branches and Codex should preserve task integrity, merge non-overlapping changes, and escalate only true semantic conflicts.
---

# Task Merge Guard

Use this skill to absorb routine `project-task` conflicts without turning tracking into a burden.

## Primary Targets

- `docs/project-task.md`
- related lightweight task-tracking edits that accompany RFC and report references

## Merge Policy

Apply these rules in order:

1. Preserve distinct newly added task lines from both sides.
2. Merge independent updates to different task lines.
3. Prefer the more complete version when one side adds RFC or report references.
4. Prefer the newer state update when the change is clearly monotonic and timestamped.
5. Escalate when both sides change the same task semantics in incompatible ways.

## Escalate These Cases

- competing owners for the same task
- incompatible scope or status meaning
- conflicting RFC linkage
- conflicting interpretations of whether the task is done

## Guardrails

- Resolve the smallest conflict surface possible.
- Keep the lightweight line format intact.
- Avoid opportunistic cleanup during conflict resolution.
