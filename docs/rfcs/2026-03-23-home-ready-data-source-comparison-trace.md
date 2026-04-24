# Home Ready Data Source Comparison Trace RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 目前 xHome `waitingConsoleReady` 的裁决主要依赖 xHome `/v6/servers/home` 返回的 capability / 地址字段。
- 对比 `XStreamingDesktop` 后已确认：它的首页主机列表实际来自 SmartGlass `getConsolesList()`，并不是同一条数据源。
- 在继续调整 ready gate 前，需要先把 SmartGlass hosts 与 xHome `/v6/servers/home` 在同一时刻并排打到 trace，确认两边观测差异。

## Goal

- 在不改变当前启动裁决逻辑的前提下，为 `waitingConsoleReady` 期间增加双数据源并排对照 trace。
- 让下一份 trace 可以直接回答：同一时刻 SmartGlass 和 xHome 各自看到了什么。

## Scope

- In scope:
  - `src-tauri/src/mods/streaming/service.rs`
  - `waitingConsoleReady` 期间的双数据源对照 snapshot
  - 定向纯函数测试
- Out of scope:
  - ready gate 行为调整
  - session recreate / RTC / 视频链路
  - SmartGlass 或 xHome 数据源本身的实现修改

## Plan

1. 在 Tauri streaming adapter 里并排采样 SmartGlass hosts 与 xHome consoles。
2. 记录新的 comparison snapshot，并保留现有 `remoteConsolesSnapshot`。
3. 补纯函数测试并完成验证。

## Validation

- [x] `cargo fmt -p xbxrc`
- [x] `cargo test -p xbxrc build_console_ready_sources_snapshot_includes_both_sources -- --nocapture`
- [x] `cargo check -p xbxrc`

## Risks

- 并排采样会额外引入 SmartGlass 查询开销，需要做轻量缓存节流，避免把 ready wait 节奏拖慢。
- 若 SmartGlass 本身偶发超时，这轮 trace 仍可能看到一侧为空，但这至少能区分“源为空”和“字段为空”。

## Progress

- [x] Step 1: 已确认 `XStreamingDesktop` 首页主机列表实际来自 SmartGlass，而不是 xHome `/v6/servers/home`。
- [x] Step 2: 已增加 `consoleReadySourcesSnapshot`，并在 adapter 层并排采样两路数据源。
- [x] Step 3: 已补纯函数测试并完成验证。

## Execution Notes

- Date: 2026-03-23 | Status: done
- Update: 本轮只加 observability，不改当前 `waitingConsoleReady` 的行为判定。
- Decision: 对照采样落在 `TauriSessionFlowProvider::get_remote_consoles()`，避免改动 `xbox-streaming` core trait。
- Update: `consoleReadySourcesSnapshot` 现在会并排记录 xHome consoles、SmartGlass hosts、sharedIds 和 sharedComparisons；SmartGlass 侧增加 2 秒轻缓存，避免轮询期重复打满。
- Validation: `cargo fmt -p xbxrc`、`cargo test -p xbxrc build_console_ready_sources_snapshot_includes_both_sources -- --nocapture`、`cargo check -p xbxrc` 已通过。
- Risk/Blocker: 若 comparison snapshot 证明 SmartGlass 稳定早于 xHome 返回注册信号，再决定是否进入下一轮 gate 策略调整。
