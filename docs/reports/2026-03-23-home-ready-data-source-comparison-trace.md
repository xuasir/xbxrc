# Home Ready Data Source Comparison Trace Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-03-23-home-ready-data-source-comparison-trace.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-23-home-ready-data-source-comparison-trace.md)
- 已完成 `waitingConsoleReady` 期间的 SmartGlass / xHome 双数据源并排 trace，为下一份日志提供直接对照面。

## Delivered

- 在 streaming adapter 里并排采样 SmartGlass hosts 与 xHome `/v6/servers/home`。
- 新增 `consoleReadySourcesSnapshot`，对比两侧同一 identity 的关键信号。
- 补纯函数测试，锁定 comparison snapshot 结构。

## Changes

- [`src-tauri/src/mods/streaming/service.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/streaming/service.rs) 在 `get_remote_consoles()` 中增加 SmartGlass hosts 采样，并保留现有 xHome `remoteConsolesSnapshot`。
- 同文件新增 `consoleReadySourcesSnapshot` 构造 helper，输出 `sharedIds / xhomeOnlyIds / smartglassOnlyIds / sharedComparisons`。
- SmartGlass 侧增加 2 秒轻缓存，避免 `waitingConsoleReady` 轮询期每次都发起完整 hosts 查询。

## Validation

- `cargo fmt -p xbxrc`
- `cargo test -p xbxrc build_console_ready_sources_snapshot_includes_both_sources -- --nocapture`
- `cargo check -p xbxrc`

## Risks

- 这轮只增加对照 trace，不改变 ready gate，所以不会直接提升成功率。
- 若 SmartGlass 查询本身超时，comparison snapshot 仍可能看到一侧为空，但这已经足够区分“源为空”和“字段为空”。

## Follow-up

- 采下一份失败或成功 trace，优先查看 `consoleReadySourcesSnapshot` 和 `consoleReadyWaitResult` 的组合。
- 若 SmartGlass 明显早于 xHome 提供稳定 ready 信号，再决定是否把 ready gate 改成双源裁决。
