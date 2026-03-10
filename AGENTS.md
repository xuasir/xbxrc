# Code Guidelines

1. Emphasize readability and maintainability.
2. Modules should be properly designed.
3. Code should include comments explaining its functionality and implementation details (light comments, use Chinese).

# Development Policy

- `src` (Vue 3/TypeScript Frontend), `src-tauri` (Tauri Rust application), and `crates/*` (Rust libraries) are the active codebases.
- Rust module organization follows [`dev-docs/rust-mod-organization.md`](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/rust-mod-organization.md); any new additions or refactors under `src-tauri/src/mods/*` must adhere to this document.

Stack requirements:

- Tauri
- Rust
- Vue 3
- TypeScript

Primary objective: Continue developing the current Tauri/Vue 3/TypeScript application as the canonical codebase, improving code quality, maintainability, and feature delivery efficiency.

# Planning & Tracking Policy

- `docs/project-task.md` MUST be the single source of truth for active task tracking.

After each completed task, you MUST update the `docs/project-task.md` file immediately.
