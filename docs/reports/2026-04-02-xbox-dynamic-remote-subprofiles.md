# Xbox Dynamic Remote Subprofiles Stage 1 Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-02-xbox-dynamic-remote-subprofiles.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-02-xbox-dynamic-remote-subprofiles.md)
- 已完成“Xbox 动态子画像”第一阶段：共享合同、runtime 分类 helper、diagnostics/trace/TS DTO 透传链路全部落地。

## Delivered

- 共享层新增动态子画像合同：`Steady`、`CloudStartup`、`CloudHighRtt`、`DecoderConstrained`、`DisplayConstrained`。
- engine 侧新增统一 runtime 分类 helper（仅消费现有信号，不新增底层采集）。
- 外部可直接读取 `baseline_remote_profile`、`dynamic_remote_subprofile`、`effective_remote_profile_label`。

## Changes

- 在 `xbxengine-protocol` 新增 `XbxEngineRemoteSubprofileKindDto` 与 `compose_effective_remote_profile_label`。
- 在 `xbxengine core` 新增 `recovery/remote_profile_runtime.rs`，按 baseline + session_phase + bitrate band + RTT + decoder/renderer/pressure/freshness 进行分类。
- 在 `XbxEngineStatsDto`、trace projection、TS RPC/Runtime 类型中打通新字段透传。

## Validation

- `cargo test -p xbxengine-protocol -- --nocapture`
- `cargo test -p xbxengine diagnostics::stats -- --nocapture`
- `cargo test -p xbxengine remote_profile_runtime -- --nocapture`
- `cargo test -p xbxrc trace_projection -- --nocapture --test-threads=1`
- `pnpm -s exec tsc --noEmit`

## Risks

- 动态子画像阈值为阶段一保守值，后续需结合真实运行 trace 做标定。
- 本阶段未接入动作策略，收益主要在可观测性与后续策略演进准备。

## Follow-up

- 用真实 cloud/home trace 回放评估子画像切换稳定性与误判率。
- 在后续阶段讨论“子画像驱动恢复/BWE/NACK 行为”的最小变更与回归矩阵。
