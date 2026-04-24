# 解码后显示调度与媒体恢复解耦 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: in-progress
- Owner: Codex
- Last Updated: 2026-04-21

## Background

- 近期多份播放期 trace 显示，当前系统会把 `decode -> pacer -> renderer -> host present` 这一段的本地显示供给问题继续翻译成媒体恢复叙事，例如 `displaySupplyCritical`、`displaySupplyDegraded`、`hostPresentStalled` 仍可能推动 `keyframe / reconnect / transportAwaitRecoveryAnchor` 一类动作。
- 这导致两个问题：
  - 低延迟目标不稳定：高刷新 host、present feedback 落后、renderer/presenter 局部抖动，会把 decode 后的本地积压放大成媒体恢复风暴。
  - 职责边界混乱：解码前的媒体有效性问题与解码后的本地显示调度问题混在同一个 recovery 梯子里，难以推导、难以验证，也容易出现脆弱耦合。
- 参考 `moonlight-qt` 当前实现：
  - [`app/streaming/video/ffmpeg-renderers/pacer/pacer.cpp`](/Users/guo.xu/Documents/code/games/moonlight-qt/app/streaming/video/ffmpeg-renderers/pacer/pacer.cpp) 中的 `Pacer` 只负责本地 `release-clock + local drop`，不会把 decode 后队列压力升级为媒体动作。
  - [`app/streaming/video/ffmpeg.cpp`](/Users/guo.xu/Documents/code/games/moonlight-qt/app/streaming/video/ffmpeg.cpp) 仅在真实 decode 连续失败时请求 IDR / 重建 decoder。
  - [`app/streaming/session.cpp`](/Users/guo.xu/Documents/code/games/moonlight-qt/app/streaming/session.cpp) 中 `SDL_RENDER_DEVICE_RESET` 只触发本地 decoder/renderer 重建，不直接映射成网络恢复动作。

## Goal

- 将 `decode success/failure` 之后的链路明确划为本地显示域，不再直接驱动媒体恢复动作。
- 将播放期调度收敛为 `Moonlight` 式 `release-clock + local drop`：视频流节拍决定供给上限，host 刷新率决定最早交付时机，积压时优先本地丢旧帧而不是升级媒体动作。
- 保持真实媒体问题的恢复能力：解码前断链、等待关键帧、bootstrap 无法继续、连续 decode failure 仍可触发 `keyframe / decoder reset / reconnect`。

## Scope

- In scope:
  - [`crates/xbxengine/core/src/media/video/pacer/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/pacer/actor.rs)
  - [`crates/xbxengine/core/src/media/video/render/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/render/actor.rs)
  - [`crates/xbxengine/core/src/media/video/render/renderer.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/render/renderer.rs)
  - [`crates/xbxengine/core/src/api/runtime/sync.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/runtime/sync.rs)
  - [`crates/xbxengine/core/src/transport/rtc/pipeline/session.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/pipeline/session.rs)
  - [`crates/xbxengine/core/src/transport/rtc/stack/runtime_port.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack/runtime_port.rs)
  - [`crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs)
  - [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs)
  - [`crates/xbxengine/core/src/transport/rtc/recovery/*`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery)
  - [`src-tauri/src/mods/native_video/*`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/native_video)
  - 相关 trace / stats / policy tests
- Out of scope:
  - 解码前 ingress、NACK admission、码流 budget 主线重写
  - 引入第二套 renderer / presenter 栈
  - 大规模更换 native_video 与 Tauri 交互模型

## Plan

1. 收紧恢复边界：将 decode 后显示域信号从媒体恢复入口中剥离，只保留本地显示诊断与本地 reset 动作。
2. 将 pacer 收敛为 `release-clock + local drop`：视频帧率只提供供给上限，host 刷新率只作为 release clock，持续积压时只做本地丢旧帧/替换/追帧。
3. 将 `decode -> pacer -> renderer latest-slot -> runtime pull -> host scheduler -> present` 收敛成 `decode -> pacer -> host scheduler -> present`，退出 renderer latest-slot 的独立暂存层。
4. 为 host scheduler / presenter 失败建立本地 reset 闭环，并补跨模块测试与 trace，验证显示域异常不会再误触发媒体动作。

## Validation

- [ ] `cargo test -p xbxengine media::video::pacer -- --nocapture`
- [ ] `cargo test -p xbxengine transport::rtc::policy::video_scheduling_owner -- --nocapture`
- [ ] `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
- [ ] `cargo test -p xbxrc mods::native_video -- --nocapture`
- [ ] 用新 runtime trace 验证 decode 后显示异常不再升级为媒体恢复动作

## Risks

- 如果边界切得过猛，可能会误伤那些“看起来像显示问题、实则 decode 前无新鲜媒体供给”的场景。
- 如果只改 pacer 不同步改 owner/session/recovery，系统仍会通过旧事实映射把显示异常重新翻译回媒体恢复动作。
- 如果本地 renderer reset 与 presenter 生命周期没有闭环，可能会出现“媒体动作退场了，但显示链也无法自愈”的空窗。

## Progress

- [x] Step 1: 已确认当前问题本质是职责边界混乱，而非单点阈值不对。
- [x] Step 2: 完成 RFC 设计并收紧模块职责边界。
- [ ] Step 3: 实现单层 host 调度链路，renderer 已退化为 shadow + staging，owner/session/recovery/startup/runtime lifecycle 已切入 host-first `renderer_stalled` 判据，剩余状态投影与 trace 语义仍需继续收口。
- [ ] Step 4: 完成验证与新 trace 复核。

## Execution Notes

- Date: 2026-04-18 | Status: in-progress
- Update: 基于 `moonlight-qt` 对照分析，明确采用“三条硬规则 + Moonlight 式 release-clock + local drop” 作为目标设计。
- Decision: `decode success/failure` 之后的所有问题都先视为本地显示域问题；只有真实媒体事实才允许进入 `keyframe / decoder reset / reconnect`。
- Decision: host 刷新率只作为 release clock，视频流帧率只作为供给上限；高刷新 host 不再被允许主导媒体恢复判定。
- Risk/Blocker: 需要系统性清理 `owner/session/recovery` 中已有的显示信号到媒体动作的映射，不能只修单个模块。
- Date: 2026-04-21 | Status: in-progress
- Update: 当前播放期链路已确认是 `decode -> pacer -> renderer latest-slot -> runtime pull -> host scheduler -> present`；其中 `renderer latest-slot` 只承担中间暂存，不承担 display tick、pending queue、stale drop、displayed retain。
- Decision: 本轮收敛成 `decode -> pacer -> host scheduler -> present`，host scheduler 保留为唯一宿主调度层，renderer latest-slot 退出独立队列语义。
- Decision: host 继续承担 `pending queue / displayed retain / stale drop / cadence telemetry / frame submit diagnostics`；pacer 只负责 release cadence 与本地 drop 候选。
- Date: 2026-04-21 | Status: in-progress
- Update: 接口层已切成 `drain_pending_render_frames -> host present`，runtime 每个 tick 批量把 staging queue 交给 host scheduler；`latestSlotOverwrite` 只保留为 render 影子诊断。
- Decision: 恢复推断链只认 host/pacer 的真实 drop、host cadence stall、renderer stalled；`latest_render_candidate_decision.detail=latestSlotOverwrite` 保持 trace 可见，不再作为 owner/recovery 的输入事实。
- Date: 2026-04-21 | Status: in-progress
- Update: `renderer` 已退出消费语义；`take_latest_frame/acknowledge_latest_frame` 已移除，`latest_frame` 保留为影子态，`pending_frames` 成为唯一 host handoff staging queue。
- Decision: `latestSlotOverwrite` 改为绑定未 drain 的 staging backlog；只要 host 已 drain backlog，后续新帧会记录 `latestSlotRecovered`，不再把 shadow latest-slot 本身当成覆盖证据。
- Date: 2026-04-21 | Status: in-progress
- Update: `display_supply`、runtime local presenter reset、presentation milestone 已切到 host telemetry 优先；`renderer_stalled` 仅在 host no-pending/age 已经成立时升级为 hard signal。
- Decision: `displaySupplyCritical/Degraded` 的本地 reset 直接跟随 owner reason；`MediaReady` 认 host present/no-pending，不再被 `renderer_stalled` 影子态单独挡住。
- Date: 2026-04-21 | Status: in-progress
- Update: owner/session 之外，`recovery/runtime_state`、`remote_profile_runtime`、`recovery_ramp_guard` 也已统一改用 host-first shadow stall 判据；fresh host present + 正常 no-pending 压力不再把会话重新贴回 `rebuilding-supply` / `displayConstrained`。
- Decision: `video_renderer_stalled` 继续保留 trace 与本地调度诊断价值；恢复域只在 host present 不新鲜或 no-pending 压力过热时把它升级成坏链信号。
- Date: 2026-04-21 | Status: in-progress
- Update: `startup::resolve_session_phase` 与 runtime recovery loop 已同步切到 shared host-first stall 判据；shadow renderer stall 不再单独把 phase 打回 `Recovering`，也不再阻断“显式 decoder stall 优先打 keyframe”的旁路。
- Decision: `session::facts` 继续只搬运原始 telemetry，不额外内嵌策略语义；host-first 解释统一留在 owner/recovery/runtime policy 层。
- Date: 2026-04-21 | Status: in-progress
- Update: `diagnostics/stats`、protocol DTO、前端 runtime snapshot 已补出“双轨字段”：raw `video_renderer_stalled` 保留 shadow 事实，`video_renderer_stall_blocks_presentation` 明确表达 host-first 解释后的真实 presentation block；`stallKind` / `presentationHealth` 已不再把 fresh host present 下的 shadow stall 误投成坏链。
- Decision: 观测面继续遵循“原始事实不丢、结论字段单独解释”的规则；前端诊断消费优先认 `videoHealth / chainHealth / presentationHealth / stallKind`，raw stall 字段只用于细查本地影子积压。
