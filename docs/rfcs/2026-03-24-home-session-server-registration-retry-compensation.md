# Home Session Server Registration Retry Compensation RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: in-progress
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 最新两份 home 日志已经证明：待机唤醒后的第一次 session 可能会在 `Provisioning` 挂到最终 `Failed`，错误明确为 `ProvisioningTimedOut / ServerNeverRegistered / WaitingForServerToRegister`。
- 同一台主机在稍后再次发起时可以成功从 `Provisioning` 推进到 `Provisioned`，说明这是可恢复的瞬态注册失败，而不是硬失败。
- 当前本地已移除基于 `Provisioning` 卡点的激进 recreate，但留下一个判定缺口：显式注册失败发生在最终 `Failed` 终态时，没有触发那次 bounded recreate。

## Goal

- 在 home 会话层补一条受控的 `ServerNeverRegistered / WaitingForServerToRegister` 自动补偿。
- 只在服务端明确暴露“尚未完成注册”的错误时触发一次 bounded recreate，不重新引入基于本地超时推断的激进重试。

## Scope

- In scope:
  - `crates/xbox-streaming/src/session/flow.rs`
  - home `waitingSessionReady` 阶段的 recreate 判定
  - 对应纯函数测试与任务文档
- Out of scope:
  - SmartGlass ready gate
  - RTC / ICE / 视频链路
  - 非 home 场景的启动补偿

## Plan

1. 收紧并修正显式注册失败判定。
2. 补充 `Failed` 终态的 recreate 回归测试。
3. 运行定向验证并更新文档。

## Validation

- [ ] `cargo fmt -p xbox-streaming`
- [ ] `cargo test -p xbox-streaming failed_server_registration_error_is_retryable_once -- --nocapture`
- [ ] `cargo test -p xbox-streaming waiting_session_ready_server_registration_error_stays_retryable -- --nocapture`
- [ ] `cargo test -p xbox-streaming home_provisioning_startup_timeout_no_longer_triggers_recreate -- --nocapture`
- [ ] `cargo check -p xbox-streaming`

## Risks

- 如果错误消息匹配范围过宽，可能把本不该重试的 `Failed` 终态误判为注册延迟。
- 如果服务端在 recreate 后仍未完成注册，这条补偿也只能恢复一次，仍需用户日志继续验证。

## Progress

- [x] Step 1: 已从“首次失败、二次成功”的日志确认补偿应绑定显式注册失败，而不是 `Provisioning` 超时本身。
- [ ] Step 2: 待补 `Failed` 终态回归测试并完成实现。
- [ ] Step 3: 待运行验证并回写结果。

## Execution Notes

- Date: 2026-03-24 | Status: in-progress
- Update: 计划将 home session recreate 的触发条件从“`stream_state=Provisioning` + 显式注册错误”收窄为“显式注册错误优先，允许落在 `Failed` 终态”，以覆盖 wake 后注册延迟的真实失败形态。
- Decision: 不恢复 `Provisioning` 10 秒快失败；只把“服务端明确说还在等待注册”的失败纳入 bounded recreate。
- Risk/Blocker: 需要避免把普通 `Failed` 终态误纳入同一补偿分支。
