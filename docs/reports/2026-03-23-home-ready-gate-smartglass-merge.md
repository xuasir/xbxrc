# Home Ready Gate SmartGlass Merge Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-03-23-home-ready-gate-smartglass-merge.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-23-home-ready-gate-smartglass-merge.md)
- 已完成 home wake 后 `waitingConsoleReady` 的最小改造：ready gate 不再被 xHome `/v6/servers/home` 单源阻断，而是使用 SmartGlass / xHome 合并后的 ready 候选统一裁决。

## Delivered

- `crates/xbox-streaming/src/session/flow.rs` 的 ready 判定已收敛到显式注册信号。
- `src-tauri/src/mods/streaming/service.rs` 已输出合并后的 `mergedReadyConsoles`，并把 `readySource` 打进 trace。
- 补充了 session core 与 Tauri merge 路径的定向测试。

## Changes

- `RemoteConsoleSnapshot` 新增 `ready_source`，`remote_console_ready_reason()` 移除 `consoleAddrs` ready 分支。
- `TauriSessionFlowProvider::get_remote_consoles()` 现在会合并 xHome consoles 与 SmartGlass hosts；若 xHome 查询失败，降级为空列表并继续用 SmartGlass 观测。
- `consoleReadySourcesSnapshot` 新增 `mergedReadyCount` / `mergedReadyConsoles`，`consoleReadyWaitResult` 新增 `readySource`。

## Validation

- `cargo test -p xbox-streaming remote_console_ready -- --nocapture`
- `cargo test -p xbxrc build_console_ready -- --nocapture`
- `cargo check -p xbox-streaming`
- `cargo check -p xbxrc`

## Risks

- identity 合并仍依赖 `serverId / id / deviceId`，若服务端后续出现新的 identity 形态，还需要继续补对齐。
- 本轮只解决 wake 后 ready gate；如果后续仍失败，问题点将继续落在 session 创建或更后面的链路。

## Follow-up

- 用下一份失败或成功 trace 验证 `consoleReadyWaitResult.readySource` 是否稳定落在 `smartglass` / `merged`。
- 如果仍卡在 `Provisioning`，继续只在会话层排查 `create_session` / server-side session 复用问题。
