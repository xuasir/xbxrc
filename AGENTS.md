# Objective

Continue developing the current desktop application as the canonical codebase, using the established Tauri + Vue 3 + TypeScript + Rust stack, while improving code quality, maintainability, and delivery efficiency.

# Fixed Stack

- Canonical product form is a **desktop application built with Tauri + Vue 3 + TypeScript + Rust**.
- Active codebases are:
  - `src`: Vue 3 + TypeScript frontend
  - `src-tauri`: Tauri Rust application
  - `crates/*`: Rust libraries and shared engine/domain modules
- Rust module organization follows [`dev-docs/rust-mod-organization.md`](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/rust-mod-organization.md); any new additions or refactors under `src-tauri/src/mods/*` must follow that document.

# Allowed Technical Route

- Frontend UI must stay on **Vue 3 + TypeScript** and extend the existing component/composable/state patterns.
- Desktop shell, native capability bridging, lifecycle management, and system integration must stay on **Tauri + Rust**.
- Core domain logic, protocol handling, transport, performance-sensitive paths, and system-facing integrations must continue to live in **Rust**.
- WebRTC / streaming / transport evolution must continue on the current Rust-owned RTC architecture and migration route.
- Gamepad navigation, focus routing, and controller UX must continue to use the geometric pathfinding engine only. Refer to [`dev-docs/gamepad-navigation.md`](dev-docs/gamepad-navigation.md).

# Forbidden Drift

- Do not introduce Electron, React, Next.js, React Native, Flutter, or other alternate client stacks as a parallel implementation path.
- Do not create a second native runtime or duplicate native bridge outside Tauri + Rust.
- Do not re-implement mature Rust-side protocol / transport / system logic in TypeScript just for convenience.
- Do not fork a second transport path, alternate signaling stack, duplicated media pipeline, or ad-hoc controller navigation path without explicit approval.

# Execution Rules

- Prefer readability, maintainability, and clear module boundaries.
- Code should include light comments in Chinese where implementation intent is not obvious.
- New code should follow the current project architecture instead of introducing parallel patterns.
- Any proposal that changes the stack, runtime boundary, transport mainline, or state architecture must be documented first under `docs/` before implementation.

# Basic Workflow

## Standard Task Workflow

1. Clarify the target and impacted modules.
2. Check existing architecture / docs / current implementation.
3. If the task is straightforward, execute directly.
4. If the task is complex, determine the implementation plan first.
5. Implement the change.
6. Run appropriate validation.
7. Update tracking documents.

## Complex Task Workflow

A task should be treated as complex if it involves one or more of the following:

- architecture or module-boundary changes
- protocol / transport / streaming changes
- cross-layer refactors
- multi-step feature delivery
- work that will span multiple commits or sessions
- work that requires explicit milestones for tracking

Default rule: 1 complex task = 1 RFC = 1 Report.

For complex tasks, execution must follow this order:

1. Determine the solution and scope first.
2. Before implementation, create an RFC plan under `docs/rfcs/`.
3. Use that RFC as the execution checklist and progress tracker during implementation.
4. After the implementation is complete, create a summary report under `docs/reports/`.
5. Update `docs/project-task.md` with the completed result and the relevant RFC/report references.

## RFC Requirements

- Complex tasks must create an RFC plan file under `docs/rfcs/` before execution starts.
- Complex tasks should create the RFC from `docs/rfcs/_template.md` by default, and keep it concise unless the task genuinely needs more detail.
- The RFC should describe background, goals, scope, non-goals, impacted modules, implementation steps, validation plan, risks, and progress checkpoints.
- If the solution changes materially during implementation, update the RFC instead of letting execution drift away from the plan.

## Report Requirements

- After a complex task is completed, create a summary report under `docs/reports/`.
- Complex tasks should create the report from `docs/reports/_template.md` by default, and keep it concise unless more detail is necessary for handoff or risk tracking.
- The report should record what was delivered, what changed, validation performed, residual risks, and follow-up items.
- The report should reference the corresponding RFC when applicable.

# Tracking Policy

- `docs/project-task.md` MUST be the single source of truth for active task tracking.
- Every completed task must be recorded in `docs/project-task.md` immediately in the current task list.
- When the current task list exceeds **100 entries**, archive completed historical items into a dated archive file such as `docs/project-task.archived.YYYY-MM-DD.md`.
- After archiving, keep `docs/project-task.md` focused on active and recent work, and preserve the archive reference at the top of the tracker.

# Completion Checklist

Before closing a task, confirm all applicable items below:

- code changes are aligned with the fixed stack and allowed technical route
- validation has been run at the appropriate level
- `docs/rfcs/` has been updated for complex work
- `docs/reports/` has been added for completed complex work
- `docs/project-task.md` has been updated immediately
