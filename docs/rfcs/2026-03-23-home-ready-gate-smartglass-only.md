# Home Ready Gate SmartGlass Only RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 最新日志 `runtime-trace-1774270778944.jsonl` 已证明：`waitingConsoleReady` 现在可以成功，但成功信号来自 `merged` 视图。
- 用户明确要求将 SmartGlass 作为唯一 ready 来源，session 必须在 remote 真正 ready 后再启动。
- xHome `/v6/servers/home` 继续保留价值，但应仅用于对比和诊断，不应进入启动 gate。

## Goal

- 将 home `waitingConsoleReady` 的观测面彻底收敛为 SmartGlass-only。
- 保留 xHome 与 SmartGlass 的并排 trace，便于继续排查 sticky `Provisioning` session。

## Scope

- In scope:
  - `src-tauri/src/mods/streaming/service.rs`
  - `docs/reports/2026-03-23-home-ready-gate-smartglass-only.md`
  - `docs/project-task.md`
- Out of scope:
  - session recreate / sticky `Provisioning` 的后续修复
  - RTC / ICE / 视频链路

## Plan

1. 将 `get_remote_consoles()` 改为只返回 SmartGlass-ready 候选。
2. 调整 trace 字段命名，明确区分 SmartGlass-ready 与 xHome 对照。
3. 跑定向测试并更新文档。

## Validation

- [x] `cargo test -p xbxrc build_console_ready -- --nocapture`
- [x] `cargo check -p xbxrc`

## Risks

- 如果 SmartGlass host 列表短暂不可见，ready gate 仍会 timeout。
- 当前只解决“谁负责放行”，不解决后续 session stuck `Provisioning`。

## Progress

- [x] Step 1: `TauriSessionFlowProvider::get_remote_consoles()` 已改为仅返回 SmartGlass-ready 候选。
- [x] Step 2: `consoleReadySourcesSnapshot` 已改成输出 `smartglassReadyCount` / `smartglassReadyConsoles`。
- [x] Step 3: 定向测试、`cargo check`、文档与 tracker 已完成。

## Execution Notes

- Date: 2026-03-23 | Status: completed
- Update: ready gate 从 `merged` 进一步收敛到 SmartGlass-only；xHome 现在只保留在对照 snapshot 中。
- Decision: session 启动真相只认 SmartGlass，xHome 仅保留为诊断视图。
- Risk/Blocker: 当前日志已显示真正剩余问题在 sticky `Provisioning` session，而不是 ready gate。
