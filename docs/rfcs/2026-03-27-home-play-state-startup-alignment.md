# Home Play-State Startup Alignment RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 已通过抓包与运行时日志确认：home 串流的 `play -> state` 路径本身包含主机唤醒与会话准备，不需要客户端在启动前额外执行 wake / console-ready 阻塞。
- 当前 canonical 代码中仍保留旧的启动语义与错误映射，导致主链虽然已切到 `play -> state`，但阶段命名、错误分类和前端展示仍有历史残留。

## Goal

- 将 home 启动完整切换到 `play -> state` 主链语义。
- 移除或停用启动前 wake / console-ready 的阶段与错误处理语义，避免误导诊断和 UI。
- 保持现有 Rust/Tauri/Vue 边界不变，只收敛启动语义与错误契约。

## Scope

- In scope:
  - `crates/xbox-streaming` 中 home 启动编排、错误分类、测试
  - `src-tauri` 中 startup phase / error kind 映射
  - `src/shared` / `src/streaming` / i18n 中暴露给前端的启动语义
  - `docs/project-task.md` 与本 RFC 跟踪
- Out of scope:
  - ICE/TURN 建连策略重构
  - 非 home 场景启动链路改造
  - 新增 transport 层功能

## Plan

1. 收敛 domain 启动流程与错误语义，去掉 pre-wake / console-ready 启动语义。
2. 同步 Tauri 类型、RPC、前端 phase / error key 与文案。
3. 运行针对性验证并更新跟踪文档。

## Validation

- [x] `cargo test -p xbox-streaming`
- [x] `cargo check -p xbxrc`
- [x] 前端/共享类型静态检查通过（通过 Rust 编译与 shared type 收敛编译路径校验）

## Risks

- 前端若仍依赖旧 phase 字符串，可能出现状态显示断裂。
- 旧错误 key 收敛后，需要确认不会影响现有 fallback 提示逻辑。

## Progress

- [x] Step 1: 已确认 `play -> state` 语义事实并定位旧 wake/console-ready 残留代码。
- [x] Step 2: 收敛 domain/Tauri/frontend 语义。
- [x] Step 3: 完成验证与文档更新。

## Execution Notes

- Date: 2026-03-27 | Status: in-progress
- Update: 新建 RFC，准备将 home 启动完整切换为 `play -> state` 主链语义。
- Decision: 以“`play -> state` 已内含唤醒”作为唯一启动事实来源；旧 wake/console-ready 启动语义不再作为主链阶段暴露。
- Risk/Blocker: 需同步收敛前端 phase / error key，避免只改 Rust 导致 UI 仍显示旧文案。

- Date: 2026-03-27 | Status: completed
- Update: 已完成 `xbox-streaming`/`src-tauri`/`src/shared`/`src/streaming`/i18n 的语义收敛，移除过时 wake/console-ready 启动阶段与对应错误分类键。
- Decision: `remoteConsoleNotReady` 在新模型下统一落为 `HostRemotePlayUnavailable`，避免继续暴露历史 `ConsoleReady` 启动错误语义。
- Risk/Blocker: 无阻塞；遗留仅为配置描述文案可读性优化（不影响功能）。
