# Code Guidelines

1. Emphasize readability and maintainability.
2. Modules should be properly designed.
3. Code should include comments explaining its functionality and implementation details (light comments, use Chinese).
4. **Gamepad Navigation**: Handled exclusively via the geometric pathfinding engine. Refer to [`dev-docs/gamepad-navigation.md`](dev-docs/gamepad-navigation.md) for architecture and usage rules.

# Development Policy

- `src` (Vue 3/TypeScript Frontend), `src-tauri` (Tauri Rust application), and `crates/*` (Rust libraries) are the active codebases.
- Rust module organization follows [`dev-docs/rust-mod-organization.md`](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/rust-mod-organization.md); any new additions or refactors under `src-tauri/src/mods/*` must adhere to this document.
- `crates/xbxengine/core` 的开发、重构与架构判断必须遵循 [`dev-docs/webrtc-streaming-layer-model.md`](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/webrtc-streaming-layer-model.md)。涉及网络接入、组帧与准入、解码、渲染呈现、观测汇总、实时调度策略的改动时，必须先按该文档判断层归属、输入输出 contract、交互模型与 Moonlight 对照口径，禁止偏离当前 `webrtc-rs` 主线去生搬硬套 Moonlight 的 RTSP/RTP/FEC 结构。

Stack requirements:

- Tauri
- Rust
- Vue 3
- TypeScript

Primary objective: Continue developing the current Tauri/Vue 3/TypeScript application as the canonical codebase, improving code quality, maintainability, and feature delivery efficiency.

# Planning & Tracking Policy

- `docs/project-task.md` MUST be the single source of truth for active task tracking.

After each completed task, you MUST update the `docs/project-task.md` file immediately.
