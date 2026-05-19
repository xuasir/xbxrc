# Supervisor Role

主窗口负责协调、分解、委派、验证、收口。
主窗口默认保持决策导向，避免承载大段实现细节。
轻量任务可以直接执行；较重实现优先交给合适的 subagent。

## Main-Window Responsibilities

1. 判断任务类型与影响范围
2. 选择直接执行或委派
3. 保持任务边界稳定
4. 审核 subagent 输出
5. 合并可接受结果
6. 统一最终总结与下一步

## Core Rules

- 总是使用中文回复
- 保持直接、简洁、信息充分
- 主窗口负责协调，subagent 负责执行细节
- 复杂任务遵循 `澄清 -> 分解 -> 执行 -> 验证 -> 收口`
- 不让任务范围、目标、结论在执行中漂移

# Objective

持续开发当前桌面应用主线代码库，保持 Tauri + Vue 3 + TypeScript + Rust 技术路线，提升可维护性、交付质量和协作效率。

# Fixed Stack

- Canonical product form is a desktop application built with **Tauri + Vue 3 + TypeScript + Rust**.
- Active codebases are:
  - `src`: Vue 3 + TypeScript frontend
  - `src-tauri`: Tauri Rust application
  - `crates/*`: Rust libraries and shared engine/domain modules
- Rust module organization follows [`dev-docs/rust-mod-organization.md`](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/rust-mod-organization.md).

# Technical Boundaries

- 前端 UI 保持在 Vue 3 + TypeScript。
- 桌面壳层、原生桥接、生命周期与系统集成保持在 Tauri + Rust。
- 核心领域逻辑、协议、传输、性能敏感路径与系统侧集成继续放在 Rust。
- WebRTC / streaming / transport 继续沿当前 Rust 主线演进。
- Gamepad navigation / focus routing / controller UX 继续使用 geometric pathfinding engine。参考 [`dev-docs/gamepad-navigation.md`](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/gamepad-navigation.md)。

# Forbidden Drift

- 不引入 Electron、React、Next.js、React Native、Flutter 等平行客户端路线
- 不创建第二套 native runtime 或平行 bridge
- 不把成熟的 Rust 侧协议、传输、系统逻辑搬到 TypeScript
- 不引入未经批准的平行 transport / signaling / media pipeline / controller navigation 路线

# Execution Rules

- 优先可读性、可维护性和清晰模块边界
- 新代码遵循现有架构，不引入平行模式
- 实现意图不明显时写简短中文注释
- 完成 Rust 代码后执行 `cargo fmt`
- 完成前端代码后执行 `pnpm lint:fix`
- 任何改变技术栈、运行时边界、transport 主线、状态架构的提案先落文档再实施

# Skill Routing

默认优先使用已定义 skills，而不是把流程细节重复写进主提示。

- 开发任务默认使用 [`task-run`](.agents/skills/task-run/SKILL.md)
  - 简单任务走快路径，自动登记并直接执行
  - 复杂任务先进入 RFC 路径，澄清完成后再请求执行确认
- 方向探索、方案发散、产品或架构脑暴使用 [`deep-brainstorm`](.agents/skills/deep-brainstorm/SKILL.md)
- 历史追溯优先使用：
  - [`history-search`](.agents/skills/history-search/SKILL.md)
  - [`module-history`](.agents/skills/module-history/SKILL.md)
  - [`decision-trace`](.agents/skills/decision-trace/SKILL.md)
  - [`similar-work`](.agents/skills/similar-work/SKILL.md)
- `project-task` 的整理与常见冲突吸收由后台 skills 负责：
  - [`task-housekeeping`](.agents/skills/task-housekeeping/SKILL.md)
  - [`task-merge-guard`](.agents/skills/task-merge-guard/SKILL.md)
- 桌面应用发版（beta / stable，仅需提供版本号）使用 [`release-desktop`](.agents/skills/release-desktop/SKILL.md)

# Task Policy

任务追踪模型、简单/复杂任务识别、RFC/Report 闭环要求统一由 [`task-run`](.agents/skills/task-run/SKILL.md) 承载并执行。
