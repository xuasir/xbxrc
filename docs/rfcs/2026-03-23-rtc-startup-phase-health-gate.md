# RTC 启动 phase / health gate 改造 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 当前 runtime 侧把 `transport connected` 过早映射为可用/健康，导致 trace 与面板会在首帧真正可见前就显示 `healthy`。
- 本次日志分析确认 `HandshakeAck` 与 control ready 都是启动关键门槛，而首帧 present 才是用户真正感知到“ready”的时刻。

## Goal

- 将外显 `session_phase` 改为更贴近用户可见状态的阶段。
- 将 `video_health` 的 `healthy` 判定收紧到首帧 present 之后。

## Scope

- In scope:
  - `crates/xbxengine/core/src/api/backend.rs`
  - `crates/xbxengine/core/src/transport/rtc/connection/data_channel.rs`
  - `crates/xbxengine/core/src/transport/rtc/connection/negotiation.rs`
  - `crates/xbxengine/core/src/transport/rtc/connection/lifecycle.rs`
  - `crates/xbxengine/core/src/diagnostics/stats.rs`
- Out of scope:
  - recovery / BWE 内部 `SessionPhase` 策略枚举扩容
  - 继续深挖 `HandshakeAck` 延迟根因

## Plan

1. 为握手与 control ready 增加显式 runtime 标记。
2. 在 diagnostics 中派生 display session phase。
3. 用 display phase 重算 health / issue chain / summary，并补回归测试。

## Validation

- [x] `cargo test -p xbxengine --lib diagnostics::stats::tests -- --nocapture`
- [x] `cargo test -p xbxengine --lib service_records_handshake_and_control_ready_timestamps -- --nocapture`
- [x] `cargo check -p xbxengine`

## Risks

- 旧逻辑若仍直接消费内部 `session_phase=startup/recovering`，需要确保只改外显 DTO，不影响内部 policy。
- 部分 trace/面板联动可能依赖既有 `video_health` 文案，需要确认新增 phase/health 不会破坏解析。

## Progress

- [x] Step 1: 完成方案收敛，确定只改外显 gate，不扩内部 phase 枚举。
- [x] Step 2: 已补 `HandshakeAck` / `control ready` 时间戳，并在 diagnostics 派生 display phase。
- [x] Step 3: 已补回归测试、跑通格式化与 check，准备归档到任务跟踪。

## Execution Notes

- Date: 2026-03-23 | Status: in-progress
- Update: 确认采用“内部 recovery phase 保持不动，外显 display phase 独立派生”的低风险方案。
- Decision: 新增 `message_handshake_acked_at_ms` 与 `control_ready_at_ms`，并以首帧 present 作为 `healthy` gate。
- Risk/Blocker: 若部分外部消费方硬编码旧 phase 值，需在验证阶段重点关注。
- Date: 2026-03-23 | Status: completed
- Update: 已落地 display phase=`connecting/handshaking/priming/steady/recovering`，并将 `healthy` gate 收紧到首帧 present 后。
- Decision: `primary_issue_chain` 在 display phase=`recovering` 时优先保留 recovery 诊断，避免被 stall 分类掩盖。
- Risk/Blocker: 当前 `cargo check -p xbxengine` 仅存在仓库内既有 dead_code warnings，本次改造未新增新的阻断项。
