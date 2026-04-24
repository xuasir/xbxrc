# Home Ready Gate Remove xHome Source RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 用户明确要求：home ready 判定不再保留 xHome 数据源，客户端画像按 `www.xbox.com` 对齐，其他 ready 来源都不成立。
- 上一轮已经把 ready gate 收敛为 SmartGlass-only，但 trace 中仍保留 xHome 查询与双源命名，造成误导。

## Goal

- 将 `waitingConsoleReady` 路径中的 xHome ready 查询与对照 trace 完全移除。
- 仅保留 SmartGlass host snapshot 与 SmartGlass-ready 候选。

## Scope

- In scope:
  - `src-tauri/src/mods/streaming/service.rs`
  - `docs/reports/2026-03-23-home-ready-gate-remove-xhome-source.md`
  - `docs/project-task.md`
- Out of scope:
  - home session API 本身
  - session stuck `Provisioning` 的后续修复

## Plan

1. 移除 `get_remote_consoles()` 中的 xHome 查询。
2. 将 ready trace 收敛为 SmartGlass-only snapshot。
3. 更新测试、文档并验证。

## Validation

- [x] `cargo test -p xbxrc build_console_ready -- --nocapture`
- [x] `cargo test -p xbxrc build_smartglass_ready_candidates_keeps_smartglass_only_host_ready -- --nocapture`
- [x] `cargo check -p xbxrc`

## Risks

- 后续如果需要和历史日志对比，需要注意事件名已从多源语义收敛到 SmartGlass-only。
- 当前仍未处理 sticky `Provisioning` session。

## Progress

- [x] Step 1: xHome ready 查询已从 `TauriSessionFlowProvider::get_remote_consoles()` 移除。
- [x] Step 2: trace 已改为 `smartglassConsolesSnapshot` 与 `consoleReadySnapshot`。
- [x] Step 3: 测试、文档、tracker 已完成。

## Execution Notes

- Date: 2026-03-23 | Status: completed
- Update: ready 路径已经只剩 SmartGlass；xHome 不再参与查询、比较或快照输出。
- Decision: home session API 继续保留，但它只负责 session create/poll/delete，不再承担 ready 判定。
- Risk/Blocker: 下一阶段问题点仍在 sticky `Provisioning` session 复用。
