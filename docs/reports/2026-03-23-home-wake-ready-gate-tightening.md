# Home Wake Ready Gate Tightening Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-03-23-home-wake-ready-gate-tightening.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-23-home-wake-ready-gate-tightening.md)
- 已完成 xHome wake 后 `waitingConsoleReady` gate 收紧，避免仅凭 `powerState=On` 提前创建 sticky `Provisioning` session。

## Delivered

- 移除 wake 后 `power-ready` fallback 放行。
- 新增 `consoleReadyWaitResult` trace，直接标记 `explicitRegistration` / `consoleAddrs` / `timeout`。
- 补最小回归测试，锁定显式注册信号 gate。

## Changes

- [`crates/xbox-streaming/src/session/flow.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/src/session/flow.rs) 的 `wait_until_console_ready()` 现必须看到显式注册信号才返回 ready。
- [`src-tauri/src/mods/streaming/service.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/streaming/service.rs) 新增 `consoleReadyWaitResult` 运行时 trace 事件。
- 定向测试补到 `flow.rs`，覆盖“仅 On 不算 ready / remote management 算 ready / console addrs 算 ready”。

## Validation

- `cargo fmt -p xbox-streaming -p xbxrc`
- `cargo test -p xbox-streaming remote_console_ready -- --nocapture`
- `cargo check -p xbxrc`

## Risks

- 若数据源长期不给注册信号，失败会更早暴露为 `remoteConsoleNotReady`，但不会再伪装成 `Provisioning` 卡死。
- 若确有少量真实成功样本只返回 `powerState=On`，这轮收紧会降低其兼容性。

## Follow-up

- 采下一份失败/成功 trace，重点看 `consoleReadyWaitResult` 是否稳定落在 `timeout` 或显式注册原因。
- 若仍失败，再往下检查 `get_remote_consoles()` 数据源是否本身缺注册字段。
