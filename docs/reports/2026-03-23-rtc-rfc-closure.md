# RTC transport RFC closure Report

## Summary

- Related RFC: [`docs/rfcs/rtc.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/rtc.md)
- 本次 RTC transport 目录职责收口与主线拆分改造已完成，RFC 正式转入维护态

## Delivered

- 完成 `facts / projection / policy / executor / connection / stream / session / pipeline / stack` 的主线职责收口
- 完成 `recovery` 与 `bwe` 的主要 legacy 决策拆分，并补齐关键回归测试
- 将 `docs/rfcs/rtc.md` 更新为完成态，不再把继续细拆作为目标

## Changes

- `pipeline` 已收回会话壳定位，`stack` 已收窄为 orchestrator/facade 主线
- `recovery` 已拆出 `runtime_state / nack_outcome / hard_stall / decoder_backend_failure / repeat_suppression`
- `bwe` 已拆出 `coupling / twcc_rules / hybrid_rules`，主文件回到共享上下文组装与模式分发角色

## Validation

- 阶段内已持续执行 `cargo fmt -p xbxengine`
- 阶段内已持续执行 `cargo test -p xbxengine --lib coordinator::tests`
- 阶段内已持续执行 `cargo test -p xbxengine --lib bwe::policy::tests`
- 阶段内已执行 `cargo check -p xbxengine`

## Risks

- `stack.rs`、`recovery/coordinator.rs`、`bwe/policy.rs` 仍保留一定体量，但当前厚度处于可接受维护范围
- 后续若新增较大功能，仍需警惕边界回流到 `stack`、`coordinator` 或 `bwe/policy` 主文件

## Follow-up

- 后续以行为回归、增量需求和真实维护压力驱动的局部调整为主
- 不再以继续拆分文件或压行数为目标
