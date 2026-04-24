# Home Session Provisioning Recreate Retry Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-03-23-home-session-provisioning-recreate-retry.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-23-home-session-provisioning-recreate-retry.md)
- 已为 home 串流在 `creatingSession` 成功后卡在 `Provisioning` 的启动失败增加单次 session recreate 恢复。

## Delivered

- 在 `crates/xbox-streaming/src/session/flow.rs` 为 home 启动链加入单次 bounded recreate。
- 在首次 home startup attempt 上增加 `Provisioning` 卡住 `10s` 的快速 recreate 补偿，不再必须等满 `startupTimeout`。
- 增加旧 session 的 best-effort cleanup 与重试探针提取。
- 增加 recreate 前的 cleanup 收敛等待，以及同 session id 复用的快速失败保护。
- 补充纯函数回归测试，覆盖 `startupTimeout` 和 `WaitingForServerToRegister` 两类恢复判定。

## Changes

- `start_session_execution_with_observer()` 现在会在 home 启动链的 `waitingSessionReady` 阶段，针对 `Provisioning` 卡死做一次 session recreate。
- `wait_until_session_started_or_failed()` 现在会在首个 home attempt 检测 `Provisioning` 是否长时间无推进，并在 `10s` 时快速失败到 recreate 分支。
- `cleanup_session_for_recreate()` 在 stop 失败时只记 warn，不阻断下一次 create。
- `wait_until_session_cleanup_settled()` 会在 recreate 前短轮询旧 session 是否已 `Closed/Failed/404`，减少马上拿回同一 session 的概率。
- `sessionCreated` / `sessionRecreateCleanup` trace 事件会直接记录 recreate 前后的 session id 与 cleanup 收敛结果。
- `decide_home_session_ready_recreate_retry()` 将重试触发条件收敛为纯函数，便于后续扩展和测试。

## Validation

- `cargo fmt -p xbox-streaming`
- `cargo test -p xbox-streaming home_provisioning_startup_timeout_is_retryable_once -- --nocapture`
- `cargo test -p xbox-streaming home_provisioning_stall_timeout_is_retryable_once -- --nocapture`
- `cargo test -p xbox-streaming fast_home_provisioning_retry_only_applies_to_waiting_provisioning -- --nocapture`
- `cargo test -p xbox-streaming cleanup_terminal_state_only_accepts_closed_or_failed -- --nocapture`
- `cargo test -p xbox-streaming recreate_reused_session_only_fails_when_cleanup_did_not_settle -- --nocapture`
- `cargo test -p xbox-streaming remote_console_ready_requires_registration_signal_after_wake -- --nocapture`
- `cargo test -p xbox-streaming remote_console_ready_accepts_remote_management_signal -- --nocapture`
- `cargo test -p xbox-streaming non_home_provisioning_timeout_is_not_retryable -- --nocapture`
- `cargo test -p xbox-streaming non_provisioning_state_is_not_retryable -- --nocapture`
- `cargo test -p xbox-streaming home_waiting_for_server_registration_is_retryable_in_provisioning -- --nocapture`
- `cargo check -p xbox-streaming`

## Risks

- 如果主机或服务端始终无法完成注册，单次 recreate 仍可能失败。
- 当前前端进度态还看不到 startup recreate 次数，需要查日志确认是否发生过恢复。

## Follow-up

- 继续用新日志验证：首次 wake 后是否会先走 `10s` 快速 recreate，以及 `sessionCreated` 是否真正拿到了新 session id。
- 若仍失败，再转查 ICE/Teredo 补偿、或把 startup recreate 次数透出到前端进度态。
