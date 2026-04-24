# Home Wake Ready Gate Tightening RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 最新 xHome 失败 trace 显示：即使完成了 client image 对齐，wake 后仍会在主机未返回显式注册信号时提前进入 `creatingSession`。
- 当前 `waitingConsoleReady` 存在 `power-ready` fallback，只要主机持续处于 `On` 一段时间，即使 `remoteManagementEnabled` / `consoleStreamingEnabled` 仍缺失且 `consoleAddrsCount=0`，也会放行。

## Goal

- 收紧 wake 后的 home ready 判定，避免在主机尚未完成 remote-play 注册时提前创建 session。
- 补最小 trace，让后续日志能直接区分 `waitingConsoleReady` 的成功原因。

## Scope

- In scope:
  - `crates/xbox-streaming/src/session/flow.rs`
  - `src-tauri/src/mods/streaming/service.rs`
  - 定向单测与启动 trace 细化
- Out of scope:
  - RTC / ICE / 视频链路
  - session recreate 策略重写
  - 非 wake 场景的 broader ready policy 调整

## Plan

1. 去掉 wake 后 `power-ready` fallback 放行。
2. 为 `waitingConsoleReady` 成功补充显式原因 trace。
3. 补测试并完成最小验证。

## Validation

- [x] `cargo fmt -p xbox-streaming -p xbxrc`
- [x] `cargo test -p xbox-streaming remote_console_ready -- --nocapture`
- [x] `cargo check -p xbxrc`

## Risks

- 若主机长期不返回注册信号，用户会更早看到 `remoteConsoleNotReady`，但这比创建 sticky `Provisioning` session 更可诊断。
- 若成功样本中存在仅依赖 `power-ready` 而无显式注册信号的真实设备行为，这次收紧可能降低部分边缘场景成功率。

## Progress

- [x] Step 1: 已确认当前主因是 wake 后 ready gate 过早放行，而不是 xHome client image。
- [x] Step 2: 已收紧 wake 后 ready gate，并补 `consoleReadyWaitResult` trace。
- [x] Step 3: 已补测试并完成验证。

## Execution Notes

- Date: 2026-03-23 | Status: done
- Update: 本轮只调整 home wake 后 `waitingConsoleReady` 的放行条件和 observability，不改 recreate / RTC 主链。
- Decision: 让 wake 后必须看到显式注册信号才进入 `creatingSession`，并新增独立 `consoleReadyWaitResult` 事件承载成功/失败原因。
- Update: `wait_until_console_ready()` 已移除 `power-ready` fallback；现在只接受 `remote_management_enabled=true` 或 `console_addrs_count>0`。
- Validation: `cargo fmt -p xbox-streaming -p xbxrc`、`cargo test -p xbox-streaming remote_console_ready -- --nocapture`、`cargo check -p xbxrc` 已通过。
- Risk/Blocker: 若收紧后仍存在卡住，需要继续比较“第二次进入成功”的 capability snapshot 对照样本。
