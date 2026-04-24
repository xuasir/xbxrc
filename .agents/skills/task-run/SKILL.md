---
name: task-run
description: Handle a development request in this repository with a low-friction workflow. Use when Codex should take one concrete task request and automatically choose the fast simple-task path or the RFC-first complex-task path, while keeping `docs/project-task.md`, `docs/rfcs/`, and `docs/reports/` aligned in the background.
---

# Task Run

Use this skill as the main task entrypoint for day-to-day development work in this repository.

## Core Contract

- Treat the caller as task-oriented, not document-oriented.
- Hide task registration, claiming, and routine tracking updates behind the workflow.
- Preserve fast response time for simple tasks.
- Push complex tasks into RFC refinement first.
- Ask for explicit execution confirmation after the RFC is clear enough to implement.

## Tracking Model

- Treat `docs/project-task.md` as the lightweight current task ledger.
- Keep small tasks inline in `docs/project-task.md`.
- Keep complex tasks summarized in `docs/project-task.md` with RFC and report links.
- Treat `docs/isu/` as exploration output owned by `deep-brainstorm`.
- Treat `docs/rfcs/` as the execution document for complex tasks.
- Treat `docs/reports/` as the final closure document for completed complex tasks.

## Classify Fast

Choose the `simple` path when most of these are true:

- The goal is concrete and local.
- The likely change stays within one module or one narrow bugfix.
- The task can usually finish in one execution window.
- No architecture or cross-layer contract decision is required.

Choose the `complex` path when any of these are true:

- The task changes workflow, architecture, ownership boundaries, or cross-layer contracts.
- The task spans multiple modules, languages, or phases.
- The task needs tradeoff analysis, staged rollout, or milestone tracking.
- The user is clearly asking for design, workflow, mechanism, or long-form planning.

## Run The Simple Path

For `simple` tasks:

1. Summarize the target in one sentence and start implementation immediately.
2. Update `docs/project-task.md` in the background with a compact line entry.
3. Do not create an RFC.
4. Perform the implementation, validation, and final task-line update.
5. Keep the user-facing response focused on the work, not on documentation mechanics.

## Run The Complex Path

For `complex` tasks:

1. Register the task in `docs/project-task.md`.
2. Draft or update the matching RFC under `docs/rfcs/`.
3. Use the RFC to clarify background, goals, scope, risks, impacted modules, and validation.
4. Iterate with the user until the RFC is implementation-ready.
5. Ask for explicit confirmation before switching from RFC refinement to execution.
6. After confirmation, execute against the RFC and keep progress aligned there.
7. When fully complete, write the matching report under `docs/reports/` and update `docs/project-task.md`.

Use the bundled templates:

- [`references/rfc-template.md`](references/rfc-template.md)
- [`references/report-template.md`](references/report-template.md)

## Completion Rules

Before closing any task, confirm:

- the implementation still fits the fixed stack and repository boundaries
- the task has appropriate validation evidence
- simple tasks are reflected in `docs/project-task.md`
- complex tasks have an updated RFC during execution
- completed complex tasks also have a report
- `docs/project-task.md` reflects the final visible state

## Keep Tracking Low-Friction

- Treat `docs/project-task.md` as the light task ledger.
- Keep small tasks inline there.
- Keep complex tasks summarized there with RFC and report links.
- Prefer append-or-local-line updates over broad reordering.
- Let housekeeping and merge-guard style workflows absorb routine maintenance.

## Classification Contract

The purpose of classification is to keep simple work fast and complex work reviewable.

Typical `simple` tasks:

- local bugfixes
- small UI, copy, or config fixes
- narrow refactors that do not change module boundaries
- single-session scoped work

Typical `complex` tasks:

- architecture or module-boundary changes
- protocol, transport, or streaming changes
- cross-layer refactors
- multi-phase delivery
- work that needs milestones, risk tracking, or explicit validation planning

## Use Existing History Before Acting

Before executing or drafting an RFC, quickly inspect relevant history when it can change the solution:

- Search `docs/project-task.md` for related task lines.
- Search `docs/rfcs/`, `docs/reports/`, and `docs/isu/` for prior work with `rg -n`.
- Reuse existing terminology, constraints, and validated directions when they remain current.

## Response Shape

When the task is simple:

- state the target
- state the first implementation step
- execute

When the task is complex:

- state that the task is entering the RFC path
- present the current RFC draft or the missing clarifications
- keep implementation paused until the user confirms execution

## Guardrails

- Do not push simple tasks through a heavy planning loop.
- Do not start implementing a complex task before the RFC is sufficiently clear and the user has confirmed execution.
- Do not expose internal tracking overhead unless it materially affects the user decision.
- Do not let task tracking drift away from the actual implementation state.
