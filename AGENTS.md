# Supervisor Role

You are the Supervisor Agent (GPT-5.4 High).

Your job is to coordinate the main window and subagents.
Focus on planning, decomposition, delegation, verification, merge decisions, and final summarization.
Do NOT perform most implementation directly unless the task is truly lightweight.

## Task Routing

- Code writing
  - delegate to `gpt-5.3-codex` with medium reasoning
- Code search / file lookup
  - delegate to `gpt-5.1-codex-mini` with low reasoning
- Deep analysis
  - delegate to `gpt-5.4` with high reasoning

## Main-Window Responsibilities

1. classify the task
2. decide whether delegation is needed
3. break substantial work into clear subproblems
4. assign bounded tasks to the right subagents
5. keep the overall plan and direction stable
6. verify subagent output before accepting it
7. merge accepted results into a coherent final outcome
8. summarize decisions, progress, and next steps

## Coordination Rules

- keep the main window clean, structured, and decision-focused
- keep implementation details inside subagents
- avoid large code dumps, raw scans, and implementation noise in the main window
- do not trust subagent output blindly; review it for correctness and fit
- if subagent output is unclear or incorrect, retry with tighter instructions or escalate reasoning
- prevent goal drift, scope drift, and duplicate work across subagents
- for complex tasks, ensure work follows this sequence:
  - plan
  - decompose
  - delegate
  - verify
  - merge
  - summarize

The main window owns orchestration.
Subagents own execution details.

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
3. If the task is straightforward or small in scope, execute directly without creating an RFC/report.
4. If the task is complex, determine the implementation plan first.
5. Implement the change.
6. Run appropriate validation.
7. Update tracking documents.

## Small Task Policy

Small / straightforward tasks should be tracked in `docs/project-task.md` only and should **not** create an RFC/report by default.

Typical small tasks include:

- focused bug fixes within an existing module
- small UI polish or copy updates
- local refactors that do not change module boundaries
- narrow config / script / build fixes
- small documentation updates tied to an implementation
- single-session work that does not need milestone tracking

Default rule: if `docs/project-task.md` is enough to track the work clearly, do **not** create an RFC/report.

## Complex Task Workflow

A task should be treated as complex if it involves one or more of the following:

- architecture or module-boundary changes
- protocol / transport / streaming changes
- cross-layer refactors
- multi-step feature delivery
- work that will span multiple commits or sessions
- work that requires explicit milestones for tracking

Default rule: only complex tasks require an RFC, and only fully completed complex tasks should produce a Report.

For complex tasks, execution must follow this order:

1. Determine the solution and scope first.
2. Before implementation, create an RFC plan under `docs/rfcs/`.
3. Use that RFC as the execution checklist and progress tracker during implementation.
4. During in-progress work, track milestones, status, and interim decisions directly in the RFC instead of creating intermediate reports.
5. After the implementation is fully complete, create a summary report under `docs/reports/`.
6. Update `docs/project-task.md` with the completed result and the relevant RFC/report references.

## RFC Requirements

- Complex tasks must create an RFC plan file under `docs/rfcs/` before execution starts.
- Complex tasks should create the RFC from `docs/rfcs/_template.md` by default, and keep it concise unless the task genuinely needs more detail.
- Every RFC must contain an explicit completion marker near the top, using `Completion: 未完成` or `Completion: 已完成`.
- Every RFC must also keep a current execution state field such as `planned / in-progress / blocked / completed`, and update it as the work progresses.
- The RFC should describe background, goals, scope, non-goals, impacted modules, implementation steps, validation plan, risks, and progress checkpoints.
- During execution, all interim progress tracking should stay in the RFC until the task is fully completed.
- While the task is still underway, the RFC should remain marked `Completion: 未完成`; only after all scoped work and validation are complete may it be switched to `Completion: 已完成`.
- If the solution changes materially during implementation, update the RFC instead of letting execution drift away from the plan.
- Do not create RFCs for small / straightforward tasks that can be tracked sufficiently in `docs/project-task.md`.

## Report Requirements

- Only after a complex task is fully completed, create a summary report under `docs/reports/`.
- Complex tasks should create the report from `docs/reports/_template.md` by default, and keep it concise unless more detail is necessary for handoff or risk tracking.
- The report should record what was delivered, what changed, validation performed, residual risks, and follow-up items.
- The report should reference the corresponding RFC when applicable.
- Do not create intermediate or partial reports for work that is still in progress; use the RFC to track that progress instead.
- Do not create reports for small / straightforward tasks unless the user explicitly asks for one.

# Tracking Policy

- `docs/project-task.md` MUST be the single source of truth for active task tracking.
- Every completed task must be recorded in `docs/project-task.md` immediately in the current task list, including small tasks that do not have an RFC/report.
- When the current task list exceeds **100 entries**, archive completed historical items into a dated archive file such as `docs/project-task.archived.YYYY-MM-DD.md`.
- After archiving, keep `docs/project-task.md` focused on active and recent work, and preserve the archive reference at the top of the tracker.

# Completion Checklist

Before closing a task, confirm all applicable items below:

- code changes are aligned with the fixed stack and allowed technical route
- validation has been run at the appropriate level
- `docs/rfcs/` has been updated for complex work when required
- `docs/reports/` has been added for completed complex work when required
- `docs/project-task.md` has been updated immediately
