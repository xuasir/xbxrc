# RTC 启动 phase / health gate 改造 Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-03-23-rtc-startup-phase-health-gate.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-23-rtc-startup-phase-health-gate.md)
- 已完成 RTC 启动阶段的外显 phase / health gate 改造，避免连接建立后过早显示 `healthy`。

## Delivered

- 为 runtime stats 增加 `message_handshake_acked_at_ms` 与 `control_ready_at_ms`。
- 将外显 `session_phase` 改为基于握手、control ready、首帧 present 与恢复诊断的 display phase。
- 将 `video_health` 的 `healthy` 判定收紧到首帧 present 之后，并补充回归测试。

## Changes

- `HandshakeAck` 首次到达时记录时间戳，control 真正 ready 后记录 `control_ready_at_ms`，rebuild/stop 时清空。
- `diagnostics/stats.rs` 新增 display phase：`connecting`、`handshaking`、`priming`、`steady`、`recovering`。
- `runtime_summary`、`primary_issue_chain`、`session_phase` DTO 与 `video_health` 全部改为消费 display phase，而非直接复用内部 phase。

## Validation

- `cargo fmt -p xbxengine`
- `cargo test -p xbxengine --lib diagnostics::stats::tests -- --nocapture`
- `cargo test -p xbxengine --lib service_records_handshake_and_control_ready_timestamps -- --nocapture`
- `cargo check -p xbxengine`

## Risks

- 若前端或离线分析脚本硬编码旧的 `session_phase` 值，需要同步适配新的 `handshaking` / `priming`。
- 当前 `cargo check -p xbxengine` 仍有仓库内既有 dead_code warnings，但无新的编译阻断。

## Follow-up

- 若后续要继续细化启动可视状态，可考虑再补 `first_decode` / `first_render_submit` 级别的 trace，但不建议回流到内部 recovery phase 枚举。
- 如果面板需要更细的恢复解释，可继续在 `primary_issue_chain` 上区分 `recovering` 与 `stall` 的展示优先级。
