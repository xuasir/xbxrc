# Steady Jitter Gentle Absorption RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 最新 trace `runtime-trace-1775310674617.jsonl` 暴露出 home steady 后游玩阶段的短时抖动会快速退化为 `transportAwaitRecoveryKeyframe -> adapterIdleTimeout -> MediaStalled -> reconnect`。
- 当前视频调度按 `source admission -> owner state -> recovery/session escalation` 三层工作，但稳态阶段对短时抖动的吸收仍偏保守，缺少“轻度退化但仍属于 steady”的缓冲层。

## Goal

- 让 steady 后的短时视频抖动优先在当前会话内被温和吸收，而不是快速升级为 decoder reset / reconnect。
- 保持 startup 与真正坏链路场景下的严格恢复策略不变，避免把脏参考链持续喂入解码器。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`
  - `crates/xbxengine/core/src/transport/rtc/session/policy.rs`
  - 相关 profile / diagnostics / tests
- Out of scope:
  - Tauri/native video presenter 架构改造
  - 新增并行 transport 路径或新恢复后端
  - 与本次 steady 抖动吸收无关的 UI/交互改动

## Plan

1. 为 steady 场景定义温和吸收策略，补齐 owner/source/recovery 的职责边界。
2. 在 owner 层引入稳态缓冲状态与 hysteresis，让短时 `noPending` / freshness 抖动不立即跌入强恢复。
3. 调整 source admission 与 recovery escalation，让已有 clean anchor 的 steady 抖动优先走轻恢复。
4. 补齐针对 steady 抖动的回归测试与 trace 定点验证。

## Validation

- [x] `cargo test -p xbxengine transport::rtc::policy::video_scheduling_owner -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::stream::video_source::source -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::stream::video_source::sink -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
- [x] `cargo test -p xbxengine diagnostics::stats -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::policy::scheduling -- --nocapture`
- [x] `cargo check -p xbxengine`

## Risks

- 稳态缓冲放宽过头会掩盖真实坏链路，延迟必要的 reconnect。
- steady 场景对 clean anchor 的 hysteresis 若定义不当，可能继续承接脏 delta/reference。
- owner / recovery 状态语义变更会影响 diagnostics，需要同步保持 trace 可解释性。

## Progress

- [x] Step 1: 明确 steady 抖动吸收策略与模块边界
- [x] Step 2: 完成 owner 层缓冲状态改造
- [x] Step 3: 完成 source / recovery / session 升级斜坡改造
- [x] Step 4: 完成测试与 RFC 回填

## Execution Notes

- Date: 2026-04-04 | Status: planned
- Update: 基于 `runtime-trace-1775310674617.jsonl` 启动 steady 后抖动温和吸收改造，计划在 owner/source/recovery 三层补齐缓冲与 hysteresis。
- Decision: 不引入新 transport 路径；沿现有三层调度收敛 steady 抖动吸收。
- Risk/Blocker: 需要在不放宽 startup 严格性的前提下，仅对 steady 场景增加容忍窗口。
- Date: 2026-04-04 | Status: implemented
- Update: owner 层补齐 `DegradedServing` 稳态缓冲态，并在 session/scheduling/stats 中把该状态作为 steady 主路径对待，避免轻度 `noPendingFrame` / freshness 抖动继续触发强恢复或 failed-terminal 清理延迟。
- Update: recovery coordinator 恢复 clean-anchor 的短窗跨 episode 滞回（上一 recovery epoch + 1.5s 窗口），让 `transportAwaitRecoveryAnchor` 在刚完成 clean anchor 后优先走 soft reentry，而不是立刻重新升级。
- Update: `video_source/sink.rs` 将 repair 路径拆成“RTX 解包 / primary 直通 / unsupported repair 丢弃”三类处理，并为“repair 路径上实际承载 primary payload”与“缺失 apt 但主 PT 唯一”的场景增加保守降级，避免恢复流轻微异常时被整体静默丢弃。
- Validation: 已完成 owner/source/sink/coordinator/session/scheduling/diagnostics 定向测试与 `cargo check -p xbxengine`，当前实现编译通过且相关回归通过。
