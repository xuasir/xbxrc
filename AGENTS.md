# Code Guidelines

1. Emphasize readability and maintainability.
2. Modules should be properly designed.
3. Code should include comments explaining its functionality and implementation details (light comments, use Chinese).

# Development Policy

- `src/main`、`src/preload`、`src/renderer` 是当前应用的唯一活跃代码库，应直接在这些目录上持续开发。

Stack requirements:

- Electron
- Vue 3
- TypeScript

Primary objective: Continue developing the current Electron/Vue 3/TypeScript application as the canonical codebase, improving code quality, maintainability, and feature delivery efficiency.

# Planning & Tracking Policy

- `docs/project-task.md` MUST be the single source of truth for active task tracking.

After each completed task, you MUST update the `docs/project-task.md` file immediately.
