# Home Ready Gate Remove xHome Source Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-03-23-home-ready-gate-remove-xhome-source.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-23-home-ready-gate-remove-xhome-source.md)
- 已将 home ready 路径中的 xHome 数据源彻底移除，只保留 SmartGlass host snapshot 和 SmartGlass-ready 候选。

## Delivered

- `TauriSessionFlowProvider::get_remote_consoles()` 不再调用 xHome `/v6/servers/home`。
- trace 事件收敛为 `smartglassConsolesSnapshot` 和 `consoleReadySnapshot`。
- 测试已同步改成 SmartGlass-only 语义。

## Changes

- 删除 xHome ready 查询失败降级逻辑。
- 删除 xHome 对照字段与 shared comparison trace。
- `consoleReadySnapshot` 只包含 SmartGlass hosts 和 SmartGlass-ready 候选。

## Validation

- `cargo test -p xbxrc build_console_ready -- --nocapture`
- `cargo test -p xbxrc build_smartglass_ready_candidates_keeps_smartglass_only_host_ready -- --nocapture`
- `cargo check -p xbxrc`

## Risks

- 新日志格式与前几轮不兼容，分析时要按 SmartGlass-only 口径看。
- 后续失败若仍出现，将直接落到 session 生命周期而非 ready 数据源。

## Follow-up

- 采一份新日志，确认 `consoleReadyWaitResult.readySource` 稳定为 `smartglass`。
- 继续处理 `homeRecreateReusedSession` / `Provisioning` sticky session。
