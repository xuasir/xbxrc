# Home Session Provisioning Wait Alignment Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-03-23-home-session-provisioning-wait-alignment.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-23-home-session-provisioning-wait-alignment.md)
- 已将 home 会话层的 `Provisioning` 补偿逻辑对齐到 `XStreamingDesktop`：不再因本地超时推断而主动 recreate。

## Delivered

- 删除 `waitingSessionReady` 阶段基于 `Provisioning` 卡点的 10 秒快速 recreate 触发。
- 删除基于 `streamingStartTimeout` / `homeProvisioningStallTimeout` 的 home recreate 判定。
- 保留仅针对 `WaitingForServerToRegister / ServerNeverRegistered` 显式注册错误的 bounded recreate。
- 更新相关纯函数测试与任务跟踪。

## Changes

- `crates/xbox-streaming/src/session/flow.rs` 的 `wait_until_session_started_or_failed()` 不再维护 `Provisioning` 卡点计时器。
- `crates/xbox-streaming/src/session/flow.rs` 的 `decide_home_session_ready_recreate_retry()` 现在只接受显式注册类错误，不再把 `Provisioning` 超时视为重建信号。
- 启动日志文案从“after provisioning stall” 调整为更准确的“after session-ready failure”。

## Validation

- `cargo fmt -p xbox-streaming`
- `cargo test -p xbox-streaming home_provisioning_startup_timeout_no_longer_triggers_recreate -- --nocapture`
- `cargo test -p xbox-streaming home_provisioning_stall_timeout_no_longer_triggers_recreate -- --nocapture`
- `cargo test -p xbox-streaming retry_requires_explicit_registration_error_message -- --nocapture`
- `cargo test -p xbox-streaming home_waiting_for_server_registration_is_retryable_in_provisioning -- --nocapture`
- `cargo check -p xbox-streaming`

## Risks

- 如果远端在 `Provisioning` 真的永久卡死，当前行为会等到统一 `startupTimeout` 或显式失败，而不再 10 秒内自发重建。
- 仍需真实 home 握手日志确认本次调整确实避免了本地补偿打断。

## Follow-up

- 让用户基于当前代码再跑一轮 home 握手，确认 session 是否能继续从 `Provisioning` 推进到 `ReadyToConnect`。
- 若仍停在 `Provisioning`，下一轮只分析服务端 session 推进证据，不再回头调整 ready gate 或 RTC。
