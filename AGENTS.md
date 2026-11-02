# Supervisor Role

主窗口负责澄清、分解、执行、验证、收口；当前仓库按用户要求使用单窗口，保持协调模式。
主窗口默认保持决策导向，避免承载大段实现细节。
任务按影响范围分阶段直接执行，保持清晰边界与验证证据。

## Main-Window Responsibilities

1. 判断任务类型与影响范围
2. 划分直接执行的步骤与依赖
3. 保持任务边界稳定
4. 审核代码与验证输出
5. 合并可接受结果
6. 统一最终总结与下一步

## Core Rules

- 总是使用中文回复
- 保持直接、简洁、信息充分
- 当前任务由主窗口直接执行；遵循用户的单窗口要求
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
- `main` 负责 Browser Gamepad API、浏览器 WebRTC 与 WebView 音视频的稳定交付；公共认证、Xbox API、会话/信令、配置、更新、日志及系统集成保持 Rust。
- `rust` 负责 Rust WebRTC/媒体及 Rust Gamepad WIP；原生源码、依赖、SDK与装配在该开发线演进。两条线复用公共协议与领域合同。
- `release/test` 从 main 发行 Browser beta，正式 `vX.Y.Z` tag 来源 main；`release/native-wip` 从 rust 发行独立 Native WIP。分支、包身份与更新源依照[发行隔离 RFC](docs/rfcs/2026-11-02-main-rust-release-isolation.md)核验。
- 应用及 FFmpeg 的 macOS 最低要求保持 26.0；实机启动、串流与长测由用户手动验收。
- Gamepad navigation / focus routing / controller UX 继续使用 geometric pathfinding engine。参考 [`dev-docs/gamepad-navigation.md`](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/gamepad-navigation.md)。
- main 直接装配 Browser GamepadBackend；rust 的 Native WIP 提供 Browser/Rust 两个输入后端与独立媒体开关。公共层决定 UI/串流归属，几何导航与输入路由共用；输入后端与媒体运行时分别装配。

# Streaming / WebRTC Behavior Standard

- 网络层优先贴近标准 WebRTC：TWCC、NACK、PLI/FIR、REMB/GCC、ICE/TURN 与反馈目标绑定遵循可互操作、可诊断、可回归的主线。
- 恢复层以低延迟优先为目标：receive 侧事实权威、ReferenceChain、RecoveryLedger、cleanAnchorCommitted、DisplayStable 形成闭环，恢复判断必须能从 trace 中复盘。
- 显示与上屏调度以远程游戏体验为目标：decode 后允许 latest-only、local drop、host refresh 快速释放，优先压低 glass-to-glass 与 submit-to-present 长尾。
- 与标准 WebRTC 差异较大的策略必须写清目标、边界、风险与验证方式，使用 runtime trace gate、midsegment report、receive feedback report 证明它让延迟、恢复、显示健康度变好。
- 分析网络、恢复、显示、上屏问题时，结论必须同时覆盖网络健康、恢复链健康、media supply、steady supply、submit/present 长尾与帧供给 delta。

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
