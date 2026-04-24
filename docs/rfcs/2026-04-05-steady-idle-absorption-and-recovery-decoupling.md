# Steady Idle Absorption And Recovery Decoupling RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 新 trace `runtime-trace-1775319678083.jsonl` 显示，steady/healthy 尾段在开始游玩后会先被 `RtcVideoFrameSource` 的实时 `StreamIdleTimeout` 判定击中，再立刻升级成 `MediaStalled -> reconnect`。
- 当前 source idle 判定只看“距上次视频包到达多久没新包”，默认阈值仅 `150ms`，没有结合 render 侧仍有余粮、current clean anchor 或 steady owner 状态。
- 同时 policy / recovery 入口仍会让新的 `adapterIdleTimeout` 落入旧的 `transportAwaitRecoveryAnchor` 升级窗，放大成 reconnect。

## Goal

- 让 steady 阶段的短时包流空窗被更温和地吸收，不再在 render 端仍有余粮时直接触发 `adapterIdleTimeout`。
- 在 policy 入口增加 render-aware 兜底，避免 source 侧偶发 idle 直接把链路推进到恢复主链。
- 保持真实的持续无包 stall 仍然可以及时触发恢复。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs`
  - `crates/xbxengine/core/src/transport/rtc/session/policy.rs`
  - 必要的单测与定向验证
  - RFC / Report / `docs/project-task.md` 跟踪
- Out of scope:
  - UI 展示层改动
  - 大范围重写 recovery budget / escalation controller
  - 与本问题无直接关联的 RTX / sink 路径调整

## Plan

1. 在 source 层为 `StreamIdleTimeout` 增加 steady-aware 吸收条件，结合 render 新鲜度与 clean anchor 证据，避免短空窗直接发 idle hint。
2. 在 policy 层为实时 `adapterIdleTimeout` 增加 render-aware 兜底门控，防止 source 单点判定直接进入恢复主链。
3. 补齐 source / policy 两侧回归测试，覆盖“steady 短空窗被吸收”和“真实无包 stall 仍可恢复”。

## Validation

- [x] `cargo test -p xbxengine steady_idle_timeout_is_absorbed_when_render_output_is_still_fresh -- --nocapture`
- [x] `cargo test -p xbxengine no_render_slack_or_no_fresh_output_still_emits_idle_timeout_observation -- --nocapture`
- [x] `cargo test -p xbxengine active_adapter_idle_timeout_is_suppressed_when_render_output_is_still_fresh -- --nocapture`
- [x] `cargo test -p xbxengine active_adapter_idle_timeout_still_reaches_recovery_path -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::stream::video_source::source -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
- [x] `cargo check -p xbxengine`

## Risks

- source 层吸收条件过宽会延迟真实 stall 的恢复。
- source 与 policy 两层同时吸收，如果口径不一致，可能引入新的时序漂移。

## Progress

- [x] Step 1: 已完成根因倒查，确认第一触发来自 source 实时 idle timeout，而非 stale diagnosis fallback。
- [x] Step 2: 实现 source idle 吸收与 policy 兜底门控。
- [x] Step 3: 完成验证并回填文档。

## Execution Notes

- Date: 2026-04-05 | Status: completed
- Update: 新建 RFC，明确这次问题主线是 `video_source/source.rs` 的实时 idle 判定过于激进，以及它与 policy/recovery 升级窗耦合过深。
- Decision: 双层一起改，source 先减噪，policy 再兜底，避免只改单层留下放大路径。
- Validation: 已完成 source/policy 两侧定向回归与 `cargo check -p xbxengine`，确认 steady 短空窗在 render 仍有余粮时被吸收，真实持续 stall 仍保留恢复通路。
- Residual Risk: 双层 render slack 门控仍依赖 runtime stats 时序一致性，后续需要继续用真实 trace 观察是否还存在极端边界下的时序漂移。
