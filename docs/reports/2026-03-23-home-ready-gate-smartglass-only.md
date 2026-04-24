# Home Ready Gate SmartGlass Only Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-03-23-home-ready-gate-smartglass-only.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-23-home-ready-gate-smartglass-only.md)
- 已将 home `waitingConsoleReady` 的启动 gate 收敛为 SmartGlass-only；xHome `/v6/servers/home` 只保留在 trace 中。

## Delivered

- `get_remote_consoles()` 现在只向 session core 返回 SmartGlass-ready 候选。
- `consoleReadySourcesSnapshot` 继续保留 xHome / SmartGlass 对照，但 ready 候选字段已改为 `smartglassReadyConsoles`。
- 更新了相应测试，覆盖 SmartGlass-only host 场景。

## Changes

- 删除了 `merged` ready 候选作为启动 gate 的角色。
- `readySource` 对于当前 gate 路径稳定为 `smartglass`。
- trace 字段由 `mergedReadyCount` / `mergedReadyConsoles` 收敛为 `smartglassReadyCount` / `smartglassReadyConsoles`。

## Validation

- `cargo test -p xbxrc build_console_ready -- --nocapture`
- `cargo check -p xbxrc`

## Risks

- SmartGlass 作为唯一 ready 来源后，host 列表质量将直接决定是否放行。
- sticky `Provisioning` session 仍是后续需要处理的主问题。

## Follow-up

- 用后续日志确认 `consoleReadyWaitResult.readySource` 只会落成 `smartglass`。
- 继续只在 session 层排查 `homeRecreateReusedSession` / `Provisioning` 复用问题。
