# Supervisor Role

Your job is to coordinate the main window and subagents.
Focus on planning, decomposition, delegation, verification, merge decisions, and final summarization.
Do NOT perform most implementation directly unless the task is truly lightweight.

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


<claude-mem-context>
# Memory Context

# [xbxrc] recent context, 2026-04-19 1:23am GMT+8

Legend: 🎯session 🔴bugfix 🟣feature 🔄refactor ✅change 🔵discovery ⚖️decision
Format: ID TIME TYPE TITLE
Fetch details: get_observations([IDs]) | Search: mem-search skill

Stats: 50 obs (26,099t read) | 2,675,376t work | 99% savings

### Apr 18, 2026
1450 11:25p 🔵 Video chain recovered once then stalled indefinitely waiting for keyframe
1451 11:42p ⚖️ Prioritize diagnostic implementation over remote IDR frame handling
1452 " 🔵 Brainstorming skill workflow requires design approval before implementation
1453 11:43p 🔵 Video recovery diagnostic terms extensively used across codebase
1454 11:46p 🔵 Recovery system links keyframe episodes with H.264 inspection observations
1455 " 🔵 XbxEngineMediaRuntimeStats tracks video anchor clean state and keyframe episodes
1456 11:47p 🔵 Runtime stats tracks decode pipeline candidate decisions and bootstrap gate rejections
1457 " 🔵 Trace projection records keyframe episode state changes and lifecycle events
1458 " 🔵 Protocol DTO exposes decode pipeline observations for external diagnostics
1459 11:48p 🔵 Protocol DTOs provide complete diagnostic observation structures for frontend consumption
1460 " 🟣 Added comprehensive diagnostic payload test for keyframe episode recovery tracking
1461 " 🔵 Project uses xbxrc as main Tauri application package name
1462 11:49p 🔴 Test failure reveals missing diagnostic payload enrichment implementation
1463 " 🟣 Implemented diagnostic payload enrichment for keyframe episode trace events
1464 11:50p 🟣 Diagnostic payload enrichment test passes successfully
1465 " 🟣 All 43 trace projection tests pass including new diagnostic enrichment test
1466 11:51p 🟣 Completed diagnostic payload enrichment implementation for keyframe episode recovery tracking
### Apr 19, 2026
1467 12:00a 🔵 Runtime trace analysis script executed on session logs
1468 " 🔵 Video streaming recovery failures traced to non-IDR frame rejections
1469 12:01a 🔵 Video timeline chain exhausted retry budget and entered broken state
1470 " 🔵 Custom analyze-runtime-logs skill provides structured recovery effectiveness metrics
1471 " 🔵 Host presentation remained stuck in priming phase with critical pressure for 4+ seconds
1472 12:02a 🔵 Comprehensive recovery audit reveals 0% keyframe and NACK effectiveness with 1568 expired retries
1473 " 🔵 Keyframe episode analysis confirms 1421 transport-suppressed requests with 1000 NonIdrVcl anchor rejections
1474 " 🔵 Presentation layer experienced maximum frame age of 22.7 seconds with 84 frames exceeding 1 second latency
1475 12:09a 🔵 Complete video streaming recovery failure chain traced to encoder IDR frame delivery failure
1476 " ⚖️ Recovery state machine enhancement approach for NonIdrVcl escalation handling
1477 " 🔵 Code path analysis reveals recovery state machine structure for NonIdrVcl handling
1478 12:10a 🔵 Keyframe episode lifecycle and family coalescing gate mechanisms identified
1479 " 🔵 Transport session bridge family coalescing mechanism and suppression flow identified
1480 " 🔵 Session policy layer transport await handling and NonIdrVcl absorption logic identified
1481 12:38a 🔵 User continued session in Chinese
1482 " 🔵 Cargo test execution blocked on file locks
1483 " 🔵 Multiple parallel cargo tests blocked by file lock contention
1484 " 🔵 All four video recovery tests passed successfully
1485 12:39a 🔵 Final test record_runtime_trace_observations_projects_keyframe_episode_recovery_diagnostics passed
1486 " 🟣 Implemented transport await non-IDR grace window tracking system
1487 12:44a 🔵 Runtime trace analysis reveals transport await recovery anchor behavior with output queue overflow
1488 " 🔵 Runtime trace summary reveals severe recovery ineffectiveness with 0% keyframe and NACK success rates
1489 12:45a 🔵 Grace window diagnostic fields not present in production runtime trace
1490 12:55a ⚖️ RFC Design Approach - Simplicity Requirement
1491 " 🔵 RFC Template Structure Located
1492 12:56a 🔵 WebRTC Recovery System Architecture Analysis
1493 " ✅ RFC Planning Progress Updated
1494 " 🔵 Project Task Tracker and RFC History Examined
1495 12:57a 🟣 Phase-Aware Recovery and Dynamic Repair Policy RFC Created
1496 " 🟣 Phase-Aware Recovery RFC Completed and Registered
1497 1:21a ⚖️ 日志补充与解码/宿主/渲染问题解决计划
1498 1:22a 🔵 Brainstorming技能工作流程探索
1499 " 🔵 视频渲染Pacer和Host调度机制代码结构

Access 2675k tokens of past work via get_observations([IDs]) or mem-search skill.
</claude-mem-context>