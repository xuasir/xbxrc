# runtime-trace-1775101572131 两个问题修复 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 最新 trace `runtime-trace-1775101572131.jsonl` 同时暴露两类问题：
  - cloud 首窗在 `Provisioned` 前后附近过早进入 `failed-terminal`
  - Connected 后仍会进入 `transportAwaitRecoveryAnchor` / `noPendingFrame` 恢复压力窗口
- 这两类问题共享同一条恢复主线，但症状分属不同阶段，不能用单点绕过修复。

## Goal

- 让 cloud 首帧前的 no-progress / reconnect 终态判定更完整，避免在会话刚进入 `Provisioned` 前后被过早锁死。
- 让 Connected 后的恢复链能正确区分“关键帧等待 / 供给恢复”与真正的渲染空转，避免 `noPendingFrame` 被当成独立故障。
- 保持现有 Rust-owned RTC 分层不变，不引入旁路恢复链或临时特判。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/session/policy.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`
  - `crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`
  - 必要时极小范围调整 `src-tauri/src/mods/native_video/mod.rs`
  - 对应单测补充与修正
- Out of scope:
  - 协议栈替换
  - 独立恢复路径
  - 非 RTC 相关渲染体系改造

## Plan

1. 审核 cloud 首窗的 liveness / failed-terminal 判定链，确认哪些状态重置会吞掉真实进展，哪些阈值需要收敛或前移。
2. 审核 Connected 后的 recovery owner / coordinator / native_video 链路，理顺 `transportAwaitRecoveryAnchor`、`noPendingFrame` 和 `stable-serving` 的边界。
3. 实现修复并补齐回归测试，验证两个问题都不会回退。

## Validation

- [x] `cargo test -p xbxengine` 中覆盖 session policy 的 cloud 首窗回归测试
- [x] `cargo test -p xbxengine` 中覆盖 recovery coordinator / video scheduling owner 的 Connected 恢复回归测试
- [x] `cargo check -p xbxengine`
- [x] 用最新 trace 语义复核关键状态是否还能复现过早终态或长时间 `noPendingFrame`

## Risks

- 过度放宽首窗阈值可能掩盖真实的连接卡死。
- 恢复链修复若只看 `Connected` 而忽略 `presentAge` / `noPendingPressure`，可能把真正的 supply stall 误放行。
- 如果只改一侧逻辑，另一侧可能继续制造同样的体感问题，因此需要同步验证两条链。

## Progress

- [x] Step 1: 解析并收敛 cloud 首窗终态链
- [x] Step 2: 收敛 Connected 后恢复链与 noPendingFrame 语义
- [x] Step 3: 补齐测试并验证无回退

## Execution Notes

- Date: 2026-04-02 | Status: completed
- Update: 已完成 cloud 首窗终态收口、Connected 恢复链 staging / hard fallback 收口，并补齐回归测试与 `cargo check`。
- Decision: 保持 Rust-owned RTC 分层不变，采用“首窗终态”和“Connected 后恢复链”双线同步修复，避免单点补丁。
- Risk/Blocker: 当前无未解决阻塞；残余风险仅在真实链路上继续观察是否有新的恢复边界。
