# Post-Anchor Transport Gap Supply Promotion RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: blocked
- Owner: Codex Supervisor
- Last Updated: 2026-04-10

## Background

- 新运行时日志显示：`clean anchor` 建立并完成首帧呈现后，transport gap 很快再次出现。
- 当前 `RtcVideoFrameSource` 的 transport NACK 价值判定直接继承 `last_submitted_frame_value`，在首帧后常落到普通 `delta/disposable`。
- `disposable` transport gap 的 retry budget 为 0，导致 `nackExpired retryBudgetExhausted` 在首轮 poll 就出现，恢复链很快失去持续供帧。

## Goal

- 让 `clean anchor` 后短窗内的关键 transport gap 不再一律按低价值 `disposable` 处理。
- 在不改变整体恢复架构的前提下，为 post-anchor 的持续供帧建立最小可恢复能力。

## Scope

- In scope:
  - `video_source/nack.rs` 中 transport gap 价值判定
  - `media/video/ingress/budget.rs` 既有 `Supply/Disposable` 预算语义复用
  - 相关 Rust 单测
- Out of scope:
  - 全局重写 timeline / owner / recovery coordinator
  - 调整所有 low-value gap 的通用 retry budget
  - 前端文案或 trace schema 变更

## Plan

1. 为 transport gap 增加“recent clean-anchor + recent present/decode”短窗识别。
2. 在窄场景下将 transport gap 的 `FrameValue` 从 plain predicted 提升为 `refresh_boost` predicted，使其进入 `Supply` 语义。
3. 补充单测，覆盖提升触发与不触发场景，并跑定向测试验收。

## Validation

- [x] 新增/更新单测覆盖 post-anchor transport gap 提升行为
- [x] `cargo fmt --all`
- [ ] 运行定向 `cargo test -p xbxengine transport::rtc::stream::video_source::nack`

## Risks

- 提升窗口过宽会误伤真正低价值 delta gap，增加无意义 NACK。
- 与现有 soft reentry / wait-keyframe 逻辑叠加时，可能让坏窗内恢复更激进。

## Progress

- [x] Step 1: 完成根因收敛，确认误判主链是 transport gap 价值继承过低
- [x] Step 2: 实现 clean-anchor 后 transport gap 价值提升
- [ ] Step 3: 跑定向测试并复核风险

## Execution Notes

- Date: 2026-04-10 | Status: in-progress
- Update: 新建 RFC，范围限定为 post-anchor transport gap 的窄范围价值提升，不调整全局 low-value retry 规则。
- Decision: 优先复用现有 `refresh_boost => Supply` 语义，避免新增并行预算路径。
- Risk/Blocker: 需要严格限制触发条件，防止在普通 steady delta 丢包场景下过度放宽。

- Date: 2026-04-10 | Status: blocked
- Update: 已在 `video_source/nack.rs` 实现 recent clean-anchor + recent present/decode 的 transport gap 价值提升，并补充 3 条单测覆盖“触发 / 不触发 / wait-keyframe 不提升”。
- Decision: 不改全局 `Disposable => retry_budget=0` 规则，只在 cloud + current recovery epoch clean-anchor + 320ms 短窗内把 plain predicted 提升为 `refresh_boost` predicted，复用现有 `Supply` 预算。
- Risk/Blocker: `cargo test -p xbxengine transport::rtc::stream::video_source::nack -- --nocapture` 因本机缺少 `pkg-config` 与 `cmake` 失败，当前无法完成目标测试验收。
