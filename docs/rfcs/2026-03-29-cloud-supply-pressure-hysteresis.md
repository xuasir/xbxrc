# Cloud 供给压力 Hysteresis RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 最新 Cloud trace 说明，`cloudHighRttLowValueAdmission` 已不再是最主要的链路破坏源，但在高 RTT + 网络波动常态下，`displaySupplyStarved` / `noPendingFrame` / `presentAgeMs` 持续抬高仍会把播放面拖进卡顿。
- 现有 owner 层会把 `SchedulingDemandSignal` 汇总成 `DisplaySupplyState`，但恢复完成条件仍然偏“单点就绪”，对持续供给压力缺少 hysteresis。
- 这会让系统在“偶发波动已恢复”与“供给真的稳定”之间切换过快，尤其在 Cloud 场景里容易让用户感受到短暂恢复后再次卡顿。

## Goal

- 在不新增平行恢复路径的前提下，让 `StableServing` 的恢复门更稳健，避免把短暂好转误判为稳定。
- 让 owner 层同时考虑 `DisplaySupplyState`、`presentAgeMs`、`decodeAgeMs` 与 `noPendingPressureLevel/noPendingStreak`，把“可播放”与“已稳定”拆开。
- 保持网络波动常态下的容错：短抖动可以继续走现有 recovery 主链，不把链路过早打回 `SupplyStarved`。
- 维持现有 Rust-owned RTC 主线和观测字段语义，不新增第二条恢复分支。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`
  - `crates/xbxengine/core/src/transport/rtc/session/policy.rs`
  - 相关单测与 trace 观察断言
  - `docs/project-task.md`
- Out of scope:
  - `native_video` 渲染层重写
  - `recovery/coordinator.rs` 的恢复动作主链改写
  - 新增第二条媒体 transport / signaling 路径
  - 继续扩大 `cloudHighRttLowValueAdmission` 为 chain break 的语义

## Plan

1. 先把 timeline 的低价值 admission 继续保持“软处理”，确认它不会再把孤立 Cloud 波动升级成 chain break。
2. 在 `VideoSchedulingOwner` 中加入 supply pressure hysteresis：恢复到 `StableServing` 前，要求供给不仅“健康”，还要“present/decode 双 fresh 且压力已降下来”。
3. 让 owner 能消费当前 epoch 的 `SubmittedCleanAnchor` anchor candidate，作为 clean anchor fact 的 fallback，并对 `gap-reorder-pending` 这类 clean anchor 后的短暂 reentry 放宽一次。
4. 补充/调整单测，覆盖高压力但短暂 freshness、clean anchor 后的 gap reentry、以及真正稳定恢复三类场景，再回放最新 Cloud trace。

## Validation

- [x] 回归 `transport::rtc::policy::video_scheduling_owner`
- [x] 回归 `transport::rtc::session::policy`
- [x] 回放最新 Cloud trace，确认 `stable-serving` 窗口更稳、`displaySupplyStarved` 不再频繁打回恢复

## Risks

- clean anchor candidate fallback 如果过于宽松，可能把一次短暂的 reordering 误判成已经稳定。
- hysteresis 过弱则无法抑制 Cloud 场景下的短促振荡。
- owner 语义如果和 `DisplaySupplyState` 叠得过深，未来维护会变难。

## Progress

- [x] Step 1: 固化恢复完成条件与供给压力边界
- [x] Step 2: 实现 owner 级 hysteresis
- [x] Step 3: 补测试并回放 trace

## Execution Notes

- Date: 2026-03-29 | Status: done
- Update: 结合最新 Cloud trace 继续收口，将改造重心收敛到 owner 的 clean-anchor hysteresis：`VideoSchedulingOwner` 现在可以把当前 epoch 的 `SubmittedCleanAnchor` anchor candidate 作为 clean-anchor fact 的 fallback，并对 `gap-reorder-pending` 这类 clean anchor 后的短暂 reentry 放宽一次；`session/policy.rs` 已把 candidate ledger 传入 owner 输入，避免 explicit clean-anchor 事实被清掉后仍无法回稳。
- Decision: `DisplaySupplyState` 仍作为基础分类，`StableServing` 保持 present/decode fresh 与压力降温门槛；但在 clean anchor 后的短暂 gap reentry 上，owner 允许一次 hysteresis，避免单个新 gap 把刚恢复起来的链再次打回恢复态。
- Risk/Blocker: 这个窗口只应覆盖 clean anchor 后的短暂 reentry，后续 trace 仍需验证没有把真实的 reference/keyframe 断裂放软过头。
