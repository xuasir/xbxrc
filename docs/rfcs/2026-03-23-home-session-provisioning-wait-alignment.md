# Home Session Provisioning Wait Alignment RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 最新两份 home 失败日志显示：`waitingConsoleReady` 已由 SmartGlass-only 显式注册成功放行，`creatingSession` 也能成功返回 `sessionId`。
- 失败点继续收敛在 `waitingSessionReady`：session 长时间停在 `Provisioning`，没有进入 `ReadyToConnect`，本地也从未发送 connect token。
- 与 `参比实现` 对照后可确认：对照实现面对 `Provisioning` 只持续轮询，不会因为本地超时推断而主动 recreate。
- 现有本地 `Provisioning` 超时重建补偿已经被新日志证伪：即使 recreate 拿到新的 `sessionId`，仍会继续卡在 `Provisioning`。

## Goal

- 让 home 会话层在 `Provisioning` 阶段对齐 `参比实现` 的保守等待策略。
- 收紧本地补偿逻辑，避免因过早 recreate 打断服务端原本还会推进的 session。

## Scope

- In scope:
  - `crates/xbox-streaming/src/session/flow.rs`
  - home 启动阶段 `waitingSessionReady` 的 recreate 判定
  - 相关纯函数测试与任务文档
- Out of scope:
  - SmartGlass ready gate
  - connect token / ICE / RTC / 视频流
  - 非 home 场景的 session 启动策略

## Plan

1. 移除 `Provisioning` 快速重建触发。
2. 仅保留显式注册类错误的 recreate。
3. 补测试并完成定向验证。

## Validation

- [x] `cargo fmt -p xbox-streaming`
- [x] `cargo test -p xbox-streaming home_provisioning_startup_timeout_no_longer_triggers_recreate -- --nocapture`
- [x] `cargo test -p xbox-streaming home_provisioning_stall_timeout_no_longer_triggers_recreate -- --nocapture`
- [x] `cargo test -p xbox-streaming retry_requires_explicit_registration_error_message -- --nocapture`
- [x] `cargo test -p xbox-streaming home_waiting_for_server_registration_is_retryable_in_provisioning -- --nocapture`
- [x] `cargo check -p xbox-streaming`

## Risks

- 如果远端 `Provisioning` 真会永久卡住，本次改动会把失败显式推迟到更晚的统一 timeout，而不是 10 秒内快速重建。
- 当前结论建立在本地日志与 `参比实现` 行为对照上，仍需用户再跑一轮真实 home 握手验证。

## Progress

- [x] Step 1: 已从最新两份失败日志确认 `Provisioning` recreate 不是有效恢复路径。
- [x] Step 2: 已将 recreate 条件收紧为显式注册类错误，不再接受 `Provisioning` 超时推断。
- [x] Step 3: 已完成定向测试与 `cargo check` 验证。

## Execution Notes

- Date: 2026-03-23 | Status: completed
- Update: 已在 `wait_until_session_started_or_failed()` 移除首个 home attempt 的 `Provisioning` 10 秒快速失败逻辑，不再生成 `homeProvisioningStallTimeout`。
- Update: 已在 `decide_home_session_ready_recreate_retry()` 中删除 `streamingStartTimeout` / `homeProvisioningStallTimeout` 对应的 recreate 分支，仅保留 `WaitingForServerToRegister / ServerNeverRegistered` 这类显式注册错误。
- Decision: `Provisioning` 本身不再作为本地主动 recreate 的证据；本地对齐 `参比实现`，在该状态只继续轮询等待服务端推进。
- Risk/Blocker: 仍需结合下一份真实 home 失败/成功日志，确认本地不再因为补偿逻辑过早打断 session。
