# Home Play-State Startup Alignment Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-03-27-home-play-state-startup-alignment.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-27-home-play-state-startup-alignment.md)
- 已完成 home 启动链路语义切换：明确以 `play -> state` 作为主链，不再暴露启动前独立 wake / console-ready 阶段。

## Delivered

- 收敛 `xbox-streaming` 启动与错误语义到 `play -> state` 主链。
- 同步 `src-tauri` 启动 phase / error kind 映射，移除过时枚举分支。
- 同步 shared/frontend 类型与状态文案，移除过时 key。

## Changes

- `crates/xbox-streaming/src/session/flow.rs`
  - `prepare_remote_console` 收敛为空钩子，不再启动前执行独立 wake/ready 流程。
  - 移除 `SessionFlowStartupPhase` 的 `WakingConsole/WaitingConsoleReady` 语义。
  - 移除 `SessionFlowStartupErrorKind` 的 `Wake/ConsoleReady` 语义，并将 `remoteConsoleNotReady` 归并为 `HostRemotePlayUnavailable`。
- `src-tauri/src/mods/streaming/types.rs` / `src-tauri/src/mods/streaming/service.rs`
  - 同步删除 `WakingConsole/WaitingConsoleReady` 与 `Wake/ConsoleReady` 映射分支。
  - 更新 fallback 分类、message key、retryable 判定，保持与新语义一致。
- `src/shared/rpc/streaming.ts` / `src/streaming/session.ts`
  - 同步收敛 startup phase/error 联合类型与状态 key 映射。
- `src/i18n/locales/zh.json` / `src/i18n/locales/en.json`
  - 移除已废弃的 `wakingConsole`、`waitingConsoleReady`、`wakeFailed`、`consoleReadyFailed` 文案键。

## Validation

- `cargo test -p xbox-streaming`（93 passed）
- `cargo check -p xbxrc`（通过）

## Risks

- 仍有配置项描述包含“wake host”文案（功能配置层），不影响启动主链语义，但可能引导误解。
- `flow.rs` 中与远程主机可用性探测相关的工具函数仍保留（用于错误总结/兼容分支），后续可继续减负。

## Follow-up

- 统一配置页描述与产品文案，避免继续暗示“启动前显式 wake 阶段”。
- 继续将 runtime trace 中的 startup phase 事件口径与 `play -> state` 主链做一轮可视化校验。
