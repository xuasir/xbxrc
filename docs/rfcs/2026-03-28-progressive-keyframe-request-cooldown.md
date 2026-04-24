# 渐进式 PLI / Keyframe Request 冷却 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: planned
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 当前 `runtime-trace-1774677241747.jsonl` 已经能证明，开始游玩后的长空窗主要来自上游恢复/准入策略，而不是 `native_video` present 层慢。
- 现有 `video_source` 在 Cloud 高 RTT 下会把一部分恢复包判成 `cloudHighRttLowValueAdmission`，对应 trace 里可见大量 `skippedLowValue` / `abandonedLate`，最终表现为长时间 `noPendingFrame`。
- 代码里已经存在 `RequestKeyframe` 的主链路，但目前还缺少一个“从轻到重”的渐进升级策略，以及明确的冷却期，避免短时间内反复给服务施压。

## Goal

- 把 PLI / keyframe request 设计成**渐进式恢复升级**，而不是一次性硬切换。
- 在短空窗内优先依赖现有 NACK / gap repair；若连续出现空窗或低价值恢复被大量放弃，再逐级升级到 PLI / keyframe request。
- 为 PLI / keyframe request 增加**冷却窗口**，避免在高 RTT / 抖动场景下短时间重复请求，造成服务端编码压力和控制面噪声。
- 保持现有 Rust-owned RTC 主线不变，不新增平行恢复路径。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/*`
  - `crates/xbxengine/core/src/transport/rtc/recovery/*`
  - `crates/xbxengine/core/src/session/*`
  - `crates/xbxengine/core/src/transport/rtc/connection/*`
  - `src-tauri/src/mods/xbxengine/trace_projection.rs`
  - 相关 runtime trace 与观测字段
- Out of scope:
  - `native_video` 渲染层策略重写
  - 新增第二条媒体 transport 主线
  - decoder / renderer 的大幅重构
  - 与本问题无关的 session / signaling 协议重写

## Plan

1. 梳理现有 keyframe request 出口、恢复升级入口和冷却点，明确谁是策略 owner。
2. 设计渐进升级条件：短空窗先 repair，连续低价值恢复或长空窗再升级到 PLI / keyframe request。
3. 定义冷却策略：同一类 request 在冷却期内不重复发，避免对服务端造成重复压力。
4. 补齐观测与验证：让 trace 能直接看到 request 触发原因、升级层级、冷却抑制与最后一次请求时间。

## Validation

- [ ] 回归 `video_source` 的恢复与 admission 相关单测
- [ ] 回归 `recovery` / `session` 里 keyframe request 相关单测
- [ ] 回归 `trace_projection` 对 keyframe request / cooldown 观测的投影
- [ ] 用新的 runtime trace 验证长空窗下不会出现 keyframe request 风暴

## Risks

- keyframe request 过于积极，可能把本来还能自愈的链路打得更抖。
- 冷却窗口过长，会让空窗继续拖大，用户体感仍然差。
- 多个 owner 同时触发 request，可能造成重复请求与语义漂移。

## Progress

- [ ] Step 1: 明确现有 RequestKeyframe / PLI 出口与策略 owner
- [ ] Step 2: 定义渐进升级阈值与冷却规则
- [ ] Step 3: 补齐观测、测试与 trace 验证

## Execution Notes

- Date: 2026-03-28 | Status: planned
- Update: 新增 RFC，准备把现有 `RequestKeyframe` 主链做成渐进式升级，并引入冷却期，避免高 RTT / 长空窗场景下重复施压。
- Decision: 不新增并行恢复路径；PLI / keyframe request 复用现有 RTC 控制面，作为恢复阶梯的一部分。
- Risk/Blocker: 冷却窗口和升级阈值需要结合现有 trace 再做定点收敛，不能只凭经验拍数值。
