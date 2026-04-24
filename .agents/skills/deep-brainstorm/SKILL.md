---
name: deep-brainstorm
description: Explore an ambiguous product, workflow, architecture, or feature direction deeply before implementation. Use when the user wants structured brainstorming, idea shaping, option comparison, or an ISU-quality writeup that can later turn into a task or RFC.
---

# Deep Brainstorm

Use this skill when the goal is to think clearly before deciding to build.

## Core Contract

- Stay in exploration mode.
- Produce a high-value thinking artifact with low ceremony.
- Capture durable conclusions in `docs/isu/` when the discussion should remain traceable.
- Hand off to `task-run` only after the direction is concrete enough to become work.

## Produce This Shape

Return the brainstorming result in this order:

1. Problem framing
2. Current constraints
3. Options
4. Recommended direction
5. Open questions
6. Candidate follow-on tasks

When the result should be captured as an ISU note, start from:

- [`references/isu-template.md`](references/isu-template.md)

## Depth Standard

Push beyond surface brainstorming:

- identify the real tension, not only the feature idea
- separate short-term fixes from durable structure
- expose tradeoffs that would change implementation shape
- connect the idea to existing repository patterns and constraints

## Use Project History

Before locking a recommendation:

- search `docs/project-task.md` for related work
- search `docs/rfcs/`, `docs/reports/`, and `docs/isu/` with `rg -n`
- note existing decisions that still constrain the design

## Escalate To Execution Carefully

If the output becomes specific enough to execute:

- propose one or more follow-on tasks
- mark whether each one looks `simple` or `complex`
- let `task-run` own the transition into tracked execution

## Guardrails

- Do not slide into implementation unless the user changes intent.
- Do not create RFCs for raw idea exploration.
- Do not flatten tradeoffs into a vague compromise when a clear recommendation exists.
