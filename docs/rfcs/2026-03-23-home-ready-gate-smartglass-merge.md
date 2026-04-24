# Home Ready Gate SmartGlass Merge RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- `runtime-trace-1774268830150.jsonl` 已证明：wake 后 SmartGlass 一直能看到主机从 `ConnectedStandby` 变为 `On`，且 `remoteManagementEnabled=true`、`consoleStreamingEnabled=true`。
- 同一窗口内，xHome `/v6/servers/home` 长时间返回空列表，导致当前 `waitingConsoleReady` 继续使用 xHome 单源 hard gate 时会被卡死。
- `XStreamingDesktop` 首页和启动入口实际依赖 SmartGlass `getConsolesList()`，不会因为 xHome `/v6/servers/home` 暂时缺席而阻断启动。

## Goal

- 将 home wake 后的 `waitingConsoleReady` 从“xHome 单源 gate”调整为“SmartGlass / xHome 双源观测，显式 ready 信号统一裁决”。
- 去掉 `consoleAddrs` 作为 ready hard gate，避免继续引入与 `XStreamingDesktop` 不一致的推断信号。

## Scope

- In scope:
  - `crates/xbox-streaming/src/session/flow.rs`
  - `src-tauri/src/mods/streaming/service.rs`
  - `docs/reports/2026-03-23-home-ready-gate-smartglass-merge.md`
  - `docs/project-task.md`
- Out of scope:
  - RTC / ICE / 视频流后续问题
  - xHome `/v6/servers/home` 业务接口改造
  - UI 层启动流程改动

## Plan

1. 调整 session core ready 判定，只接受显式注册信号。
2. 在 Tauri provider 合并 xHome / SmartGlass 的 ready 候选，并保留双源 trace。
3. 运行定向验证并回写报告与 tracker。

## Validation

- [x] `cargo test -p xbox-streaming remote_console_ready -- --nocapture`
- [x] `cargo test -p xbxrc build_console_ready -- --nocapture`
- [x] `cargo check -p xbox-streaming`
- [x] `cargo check -p xbxrc`

## Risks

- SmartGlass 与 xHome 仍可能出现 identity 不一致；当前合并只按 `serverId/id/deviceId` 做最小匹配。
- 如果 SmartGlass 侧也短暂不可见，启动仍可能 timeout，但这比被 xHome 单源硬卡更接近 `XStreamingDesktop` 行为。

## Progress

- [x] Step 1: `RemoteConsoleSnapshot` 增加 `ready_source`，`remote_console_ready_reason()` 仅接受 `remoteManagementEnabled=true`。
- [x] Step 2: `TauriSessionFlowProvider::get_remote_consoles()` 改为合并 xHome / SmartGlass ready 候选，并在 trace 中记录 `mergedReadyConsoles`。
- [x] Step 3: 定向测试、`cargo check`、Report 与 `docs/project-task.md` 已完成。

## Execution Notes

- Date: 2026-03-23 | Status: completed
- Update: home ready gate 已改为双源观测；xHome 查询失败会降级为 SmartGlass-only 观测，不再直接打断 `waitingConsoleReady`。
- Decision: `consoleAddrs` 保留为诊断字段，不再参与 ready hard gate；最终 gate 统一收敛到 `powerState=On` 且 `remoteManagementEnabled=true`。
- Risk/Blocker: 当前仅解决握手前 ready 判定，不覆盖后续 session / RTC / 视频阶段故障。
