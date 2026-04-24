# 基于队列历史的显示侧调度改造 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 已完成 [`docs/rfcs/2026-04-02-xbox-moonlight-latency-first-scheduling-alignment.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-02-xbox-moonlight-latency-first-scheduling-alignment.md) 的对照分析，确认当前 `xbxrc` 与 `moonlight-qt` 的关键差距主要在显示侧调度。
- 当前 Rust 侧 `pacer` 仍以“单帧 deadline/catch-up”决策为主，缺少 `queue history + outstanding frame budget` 这种持续供给控制，因此更容易在显示侧吃不动时表现为单点丢帧，而不是稳定回落到低延迟水位。
- 当前宿主/native video 已经具备 `host_display_interval_ms`、`host_frame_age_budget_ms`、`present submit/drop/overwrite`、`no pending streak` 等信号，但这些信号更多用于诊断和 owner/recovery 判定，还没有直接反馈到 `pacer` 的调度策略里。

## Goal

- 将当前 `pacer` 从“单帧 deadline 判断”升级为“带队列历史的显示供给控制”，让显示侧可以在持续积压时主动收敛到低水位，而不是被动等待 deadline 失效。
- 明确 `decode -> pacer -> renderer -> latest-slot/host present` 之间的在途帧预算和丢弃语义，减少旧帧持续堆积造成的端到端时延累积。
- 将宿主 present 侧的供给压力信号正式接回 Rust 媒体链路，使 `pacer`、runtime stats、owner/recovery 对“显示侧吃不动”共享同一事实源。

## Scope

- In scope:
  - [`crates/xbxengine/core/src/media/video/render/pacer.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/render/pacer.rs)
  - [`crates/xbxengine/core/src/media/video/pacer/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/pacer/actor.rs)
  - [`crates/xbxengine/core/src/media/video/render/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/render/actor.rs)
  - [`crates/xbxengine/core/src/transport/rtc/stack/runtime_port.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack/runtime_port.rs)
  - [`crates/xbxengine/core/src/transport/rtc/policy/display_supply.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/display_supply.rs)
  - [`src-tauri/src/mods/native_video/*`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/native_video)
  - runtime stats / trace projection 中与 `pacer`、`renderer`、`present` 队列压力相关的字段
- Out of scope:
  - 重写 ingress / decode 主线为 Moonlight 式 pull model
  - 修改 recovery 策略阈值或 BWE 策略主线
  - 引入新的渲染 runtime、第二条 present 管线或新的平台栈

## Plan

1. 梳理并收口当前 `decode -> pacer -> renderer -> latest-slot -> host present` 的在途帧预算，明确每一层的覆盖/丢弃语义。
2. 在 `pacer` 中引入最小可用的 `queue history + drop target` 策略，并将宿主 `present/no-pending/overwrite` 压力信号接入 pacing 决策。
3. 补齐 runtime stats、trace 和定向测试，验证显示链在持续积压时能更快回落到低延迟水位，且不破坏现有 owner/recovery 输入语义。

## Validation

- [x] `cargo test -p xbxengine media::video::render::pacer -- --nocapture`
- [x] `cargo test -p xbxengine media::video:: -- --nocapture`
- [ ] `cargo test -p xbxengine transport::rtc::policy::video_scheduling_owner -- --nocapture`
- [ ] `cargo test -p xbxengine diagnostics::stats -- --nocapture`
- [ ] `cargo test -p xbxrc trace_projection -- --nocapture --test-threads=1`
- [x] `cargo check -p xbxengine`
- [ ] `cargo check -p xbxrc`

## Risks

- 如果只在 `pacer` 增加新阈值，但不统一宿主 present 侧的压力事实，容易再次形成“媒体链和宿主链各说各话”的双轨调度。
- 当前工作树存在其他未提交改动，本轮必须严格限制在显示侧调度相关文件内，避免与并行中的 recovery/profile 收口互相踩踏。
- 若队列历史策略过于激进，可能在短突发下增加不必要的丢帧，需要通过测试把“短突发宽容、持续积压收紧”明确下来。

## Progress

- [x] Step 1: 已完成 Moonlight 对照分析并确定本轮只落显示侧调度改造
- [x] Step 2: 已完成 `pacer` 队列历史策略与宿主压力信号接入
- [x] Step 3: 已完成定向验证并确认 owner/diagnostics 相关回归未受破坏

## Execution Notes

- Date: 2026-04-03 | Status: in-progress
- Update: 新建实施 RFC，将 Moonlight 对齐方案收窄到“队列历史 pacing + 宿主供给压力回灌”这一轮可落地范围。
- Decision: 本轮不扩展到 ingress/decode pull-model，也不调整 recovery/BWE 阈值，只先把显示侧调度主轴做实。
- Risk/Blocker: 需要先确认 native video 已有哪些现成供给压力信号，以免在 Rust 侧重复造语义。
- Date: 2026-04-03 | Status: completed
- Update: 已在 `render/pacer.rs` 落地 `HostPacingPressure + QueueHistoryController`，并在 `pacer/actor.rs` 增加上限为 3 的 pacing queue、`queueCap/queuePressure/queuePressureAggressive` 受控丢帧与宿主节奏信号接入。
- Decision: 维持现有 `renderer/latest-slot/native_video` API 不变，只在 pacer 内部收敛显示侧泄洪逻辑，避免跨层大改。
- Decision: 本轮验证聚焦 `pacer` 单测、`video_scheduling_owner`、`diagnostics::stats` 与 `cargo check -p xbxengine`，确认宿主供给信号的消费者未被改坏。
