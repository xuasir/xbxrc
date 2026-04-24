# Steady Recovery Stale Diagnosis Expiry RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 最新 runtime trace 尾段显示链路已经恢复到 steady，`primaryVideo`、`frame_submit`、`sample_presented` 与 packet counter 持续推进。
- 但 `RecoveryProjection.latest_diagnosis_label` 会把旧的 `adapterIdleTimeout` 长时间挂住，`RtcSessionPolicy::build_recovery_proposal()` 在没有新的 `recovery_intent` 时持续 fallback 复用旧 diagnosis，触发恢复风暴并把视频链路打死。

## Goal

- 让 steady 恢复后的短时抖动被温和吸收，不再因为陈旧 `adapterIdleTimeout` 被 policy 重放。
- 保持真实无进展 idle stall 的恢复链路仍然可触发，不降低有效恢复能力。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/session/policy.rs`
  - 必要的定向单测与回归验证
  - RFC 进度与 `docs/project-task.md` 跟踪
- Out of scope:
  - `directGamingState` / UI 展示层补丁
  - 大范围重写 `RecoveryProjection` 账本模型
  - 与本次问题无直接关联的 repair/RTX 路径改造

## Plan

1. 在 policy fallback 分支引入 stale diagnosis 门控，只对缺少当前恢复意图的旧 diagnosis 做失效判断。
2. 复用运行态已有的“fresh media output / steady serving”证据，抑制 steady 后陈旧 `adapterIdleTimeout` 的重复 proposal。
3. 补齐回归测试，验证“steady 恢复后不再风暴”与“真实 idle stall 不被误伤”两个方向。

## Validation

- [x] `cargo test -p xbxengine stale_adapter_idle_timeout_does_not_replay_during_steady_progress -- --nocapture`
- [x] `cargo test -p xbxengine active_adapter_idle_timeout_still_reaches_recovery_path -- --nocapture`
- [x] `cargo check -p xbxengine`

## Risks

- 门控条件过宽会吞掉真实 stall，导致恢复动作变迟钝。
- 过度依赖 runtime stats 的 fresh output 证据，可能让 policy 与 snapshot 的时序边界更敏感。

## Progress

- [x] Step 1: 已完成根因定位，确认问题在 policy fallback 重放陈旧 diagnosis。
- [x] Step 2: 已在 `session/policy.rs` 的 fallback 分支增加 `adapterIdleTimeout` 抑制门控，并补齐正反两侧回归测试。
- [x] Step 3: 已完成定向验证、RFC 回填与任务追踪收口。

## Execution Notes

- Date: 2026-04-04 | Status: in-progress
- Update: 新建 RFC，明确最小修复面收敛在 `session/policy.rs` 的 fallback diagnosis 门控，而不是展示层降级。
- Decision: 优先做 policy 输入层修复，避免继续让 coordinator 收到无意义的 `adapterIdleTimeout` proposal。
- Risk/Blocker: 当前工作区较脏，目标文件已有多轮相关改造，实施前需要严格基于现状增量修改并补充分层验证。
- Date: 2026-04-04 | Status: completed
- Update: `RtcSessionPolicy` 现在会在检测到 fresh media output 或 current clean anchor 且解码/渲染未 stalled 时，直接抑制 fallback 的 `adapterIdleTimeout`，不再把已恢复链路重新推回 recovery coordinator。
- Decision: 门控复用 `runtime_state` 已承认的恢复证据，只拦截 `adapterIdleTimeout` 的 fallback 重放，不改 `RecoveryProjection` 账本模型。
- Risk/Blocker: 仍需继续观察真实 trace 中是否存在“无 clean anchor 但有短时 present/decode 前进”的边角窗口，不过当前门控已覆盖这次复现的主路径。
