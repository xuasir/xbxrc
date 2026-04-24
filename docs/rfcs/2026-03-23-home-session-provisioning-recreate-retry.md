# Home Session Provisioning Recreate Retry RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 最新 home 串流失败日志显示：`wakingConsole`、`waitingConsoleReady`、`creatingSession` 都已成功，但 session 长时间停留在 `Provisioning`。
- 本地最终以 `streamingStartTimeout` 超时，随后远端 `/state` 返回 `ProvisioningTimedOut / ServerNeverRegistered / WaitingForServerToRegister`。

## Goal

- 为 home 串流启动阶段补一个有边界的 session recreate 恢复分支。
- 仅处理 `creatingSession` 之后卡在 `Provisioning` 的启动失败，不扩散到 ICE/runtime 主链。

## Scope

- In scope:
  - `crates/xbox-streaming/src/session/flow.rs`
  - home 启动链 `create_session_with_observer -> wait_until_session_started_or_failed`
  - best-effort 旧 session 清理与单次 recreate
- Out of scope:
  - ICE candidate/Teredo 处理
  - xbxengine runtime / WebRTC 媒体链
  - 非 home 场景的启动重试策略

## Plan

1. 提炼可恢复的 `Provisioning` 卡死判定。
2. 在 session flow 增加单次 bounded recreate。
3. 补纯函数测试并跑定向验证。

## Validation

- [x] `cargo fmt -p xbox-streaming`
- [x] `cargo test -p xbox-streaming home_provisioning_startup_timeout_is_retryable_once -- --nocapture`
- [x] `cargo test -p xbox-streaming home_provisioning_stall_timeout_is_retryable_once -- --nocapture`
- [x] `cargo test -p xbox-streaming fast_home_provisioning_retry_only_applies_to_waiting_provisioning -- --nocapture`
- [x] `cargo test -p xbox-streaming cleanup_terminal_state_only_accepts_closed_or_failed -- --nocapture`
- [x] `cargo test -p xbox-streaming recreate_reused_session_only_fails_when_cleanup_did_not_settle -- --nocapture`
- [x] `cargo test -p xbox-streaming non_home_provisioning_timeout_is_not_retryable -- --nocapture`
- [x] `cargo test -p xbox-streaming non_provisioning_state_is_not_retryable -- --nocapture`
- [x] `cargo test -p xbox-streaming home_waiting_for_server_registration_is_retryable_in_provisioning -- --nocapture`
- [x] `cargo test -p xbox-streaming remote_console_ready_requires_registration_signal_after_wake -- --nocapture`
- [x] `cargo test -p xbox-streaming remote_console_ready_accepts_remote_management_signal -- --nocapture`
- [x] `cargo check -p xbox-streaming`

## Risks

- 远端如果持续无法注册，单次 recreate 只能改善部分瞬态失败，不能消除主机或服务端根因。
- `SessionProgressSnapshot.retry_count` 仍未对外透传本次 recreate 次数，诊断信息暂时只在日志里可见。

## Progress

- [x] Step 1: 已确认当前失败符合 `Provisioning` 卡住 + `ServerNeverRegistered` 模式。
- [x] Step 2: 已在 home 启动链加入单次 bounded recreate。
- [x] Step 3: 已补纯函数测试并完成定向验证。
- [x] Step 4: 已补首次 `Provisioning` 长时间无进展时的快速 recreate 补偿，避免必须等满 `startupTimeout`。
- [x] Step 5: 已在 recreate 前等待旧 session cleanup 收敛，并补 trace 钩子确认新旧 session id。

## Execution Notes

- Date: 2026-03-23 | Status: completed
- Update: `start_session_execution_with_observer()` 已改为围绕 `create_session_with_observer -> wait_until_session_started_or_failed` 做单次 bounded recreate；重试前 best-effort `close_session + clear_session`，并再次执行 `prepare_remote_console()`。
- Update: 在首次 home startup attempt 上，如果 session 已进入 `waitingSessionReady` 且 `stream_state=Provisioning` 持续 `10s` 仍无进展，会提前抛出 `homeProvisioningStallTimeout` 进入 recreate，而不再被动等满 `startupTimeout`。
- Update: recreate 前新增旧 session cleanup 收敛等待；若 cleanup 未收敛且新 create 仍拿回相同 session id，则直接按 `homeRecreateReusedSession` 失败，避免继续在同一条卡死 session 上空转。
- Update: Tauri trace 新增 `sessionCreated` 与 `sessionRecreateCleanup` 事件，用于确认 recreate 是否真的轮换 session。
- Decision: 仅当 `home + waitingSessionReady/Failed + stream_state=Provisioning`，且命中 `streamingStartTimeout`、`homeProvisioningStallTimeout` 或 `WaitingForServerToRegister / ServerNeverRegistered` 时才触发重试。
- Risk/Blocker: 当前未改 ICE/runtime；若后续日志仍在 recreate 后失败，再转向 ICE/Teredo 或会话进度可视化。
