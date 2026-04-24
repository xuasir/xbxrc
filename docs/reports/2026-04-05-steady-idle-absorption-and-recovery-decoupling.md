# Steady Idle Absorption And Recovery Decoupling Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-05-steady-idle-absorption-and-recovery-decoupling.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-05-steady-idle-absorption-and-recovery-decoupling.md)
- 已完成 steady 阶段短时无包空窗的 source 吸收与 policy 解耦，避免 healthy 链路在开始游玩时被 `adapterIdleTimeout -> MediaStalled -> reconnect` 直接打进恢复风暴。

## Delivered

- 在 [`source.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs) 为实时 idle timeout 增加 steady/render-aware 吸收条件，只有 transport 已连接、当前 recovery epoch 存在 clean anchor、decoder/renderer 未 stalled 且 present/decode 仍在 slack window 内时才吸收短空窗。
- 在 [`policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 增加两层 `adapterIdleTimeout` 保护：实时 diagnosis 吸收与 render slack 抑制，避免 source 单点判定直接落入旧的恢复升级链。
- 补齐 source / policy 两侧正反回归测试，覆盖“steady 短空窗被吸收”和“真实持续无包 stall 仍进入恢复”的行为边界。

## Changes

- source 层新增 `idle_timeout_render_slack_window_ms`、`should_absorb_idle_timeout_for_steady_gap` 与 `RtcVideoFrameSource::should_absorb_idle_timeout`，并在 `recv_frame_inner` 中把 idle timeout 触发改为“先判断，再按 steady/render 证据吸收”。
- policy 层新增 `should_absorb_render_aware_realtime_adapter_idle_timeout`、`should_suppress_adapter_idle_timeout_with_render_slack` 与 `adapter_idle_render_slack_window_ms`，把实时 `adapterIdleTimeout` 与兜底 diagnosis 分层处理。
- source / policy 两侧 slack window 都统一为 `idle_timeout_ms * 1.5`，并夹在 `220ms..450ms` 之间，减少实现口径漂移。

## Validation

- `cargo test -p xbxengine steady_idle_timeout_is_absorbed_when_render_output_is_still_fresh -- --nocapture`
- `cargo test -p xbxengine no_render_slack_or_no_fresh_output_still_emits_idle_timeout_observation -- --nocapture`
- `cargo test -p xbxengine active_adapter_idle_timeout_is_suppressed_when_render_output_is_still_fresh -- --nocapture`
- `cargo test -p xbxengine active_adapter_idle_timeout_still_reaches_recovery_path -- --nocapture`
- `cargo test -p xbxengine transport::rtc::stream::video_source::source -- --nocapture`
- `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
- `cargo check -p xbxengine`

## Risks

- render-aware 吸收依赖 `latest_video_host_present_time_ms` / `latest_video_decode_ok_time_ms` 与 clean anchor 事实持续正确推进，如果 runtime stats 时序漂移，仍可能出现边界误判。
- 这次修复只处理了 steady 短空窗与恢复升级耦合，未改动更大范围的 recovery budget / escalation controller，因此极端网络抖动下的其他恢复放大路径仍需继续观察。

## Follow-up

- 用下一份真实 runtime trace 验证“健康链路开始游玩”场景下，尾段不再因短时无包空窗直接打入 reconnect。
- 继续关注 [`video_source/sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/sink.rs) repair / RTX 识别与投递路径是否还会制造额外的空窗放大。
