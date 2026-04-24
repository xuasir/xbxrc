# xbxengine Mainline Pollution Isolation RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 最新多份 runtime trace 都指向同一类问题：网络并非主矛盾，真正卡死点在本地主流程被非关键路径拖慢甚至打断。
- 当前 `xbxengine` runtime 主链同时承担了视频 present、宿主桥接、副作用调用、trace 投影、恢复判定与部分控制面同步动作，缺少线程隔离、通道隔离与优先级隔离。
- 前几轮已止血了部分典型问题：
  - burst rumble backlog 会拖死 tick/present
  - 首帧前 `presentEpoch=0` 被过早判成 `SupplyStarved`
  - 重复 `attach_viewport()` 会重置 native video media epoch
- 但系统性风险仍在：任何新的 host side effect、全量 drain、trace 放大、共享锁滥用，都可能继续污染 present 主链。

## Goal

- 对 `xbxengine` runtime 主流程做一次系统性的“污染隔离”改造。
- 明确并收口视频 present 主链，只保留必须同步的动作。
- 将 haptics、观测投影、非关键宿主副作用、可延期恢复动作从主链剥离或降级为预算内后台任务。
- 为后续新增功能建立明确规则：默认不能污染主流程，除非被证明必须同步。

## Scope

- In scope:
  - `crates/xbxengine/core/src/api/runtime/*`
  - `src-tauri/src/mods/xbxengine/*`
  - `src-tauri/src/mods/native_video/*`
  - `src-tauri/src/mods/runtime_trace/*`
  - 与 runtime 主链直接耦合的 transport/recovery/host bridge 路径
- Out of scope:
  - 大规模改写 transport 协议语义
  - 更换 Tauri/Vue/Rust 既有栈
  - 与本次主流程污染无直接关系的 UI 调整

## Plan

1. 建立主流程污染清单，按“必须同步 / 可延期 / 可异步 / 必须移出主链”分类所有 runtime tick 路径。
2. 实施主链隔离改造：
   - 收紧 runtime tick 主链
   - 抽离 haptics/trace/宿主副作用到独立 worker 或预算队列
   - 缩短共享锁与主线程桥接对主链的影响
   - 按 `moonlight-qt` 的 pacing / render 原则重构 macOS `native_video layer` 链路：主线程只保留 layer 生命周期与布局，同步视频 present 改为独立 worker + vsync 驱动 + 有界队列
3. 补齐组合行为测试与 harness，覆盖 burst input/rumble、trace 压力、重复 attach、恢复链叠加等污染场景。

## Validation

- [x] `cargo test -p xbxengine runtime_tick_ -- --nocapture`
- [x] `cargo test -p xbxengine recovery_integration_home_burst_input_rumble_ -- --nocapture`
- [x] `cargo test -p xbxengine recovery:: -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::stack::transport_session::tests -- --nocapture`
- [x] `cargo test -p xbxrc trace_projection -- --nocapture`
- [x] `cargo test -p xbxrc native_video -- --nocapture`
- [ ] `cargo test -p xbxengine transport::rtc::session::policy:: -- --nocapture`
- [x] `cargo check -p xbxrc`
- [x] `cargo check -p xbxengine`
- [ ] 用最新 runtime trace 回归验证 present 主链不再出现长时间停摆

## Risks

- 主链剥离后如果状态同步边界不清，可能引入新的一致性问题。
- 将副作用移到独立 worker 后，如果缺少最新态合并与 backpressure 策略，可能把问题从“阻塞”变成“积压”。
- 恢复策略与 present 反馈的时序若处理不当，可能放大误诊断或造成恢复动作延迟。
- 本轮明确不考虑向下兼容旧的 `layer` 单槽 + 主线程 present 语义，重构期间测试与诊断口径会一起调整；若仍保留旧统计假设，会导致结论失真。

## Progress

- [x] Step 1: 已确认这是复杂任务，进入 RFC 管理
- [x] Step 2: 已完成第一版污染清单和优先级排序
- [x] Step 3: 已完成 rumble worker 化与第一批 trace gating 隔离改造
- [ ] Step 4: 完成组合测试与 trace 回归

## Execution Notes

- Date: 2026-04-05 | Status: planned
- Update: 建立 RFC，任务正式切换为“主流程污染隔离”专项，不再以单点日志修补为主。
- Decision: 先并行完成污染清单，再按 runtime 主链 / 宿主桥接 / 恢复与观测 三条线分片实施。
- Risk/Blocker: 当前仓库有较多并行修改，实施时必须严格限定写集，避免覆盖无关在制改动。

- Date: 2026-04-05 | Status: in-progress
- Update: 已完成第一批隔离改造：
  - `runtime_state` 改为锁外采样 `native_video` feedback、锁内短写回
  - `runtime_trace` 从同步 `Mutex + flush-per-line` 改为异步 writer 线程批量写盘
  - `native_video` 对 `frame_submit / frame_slot_take_skipped / prepare_sample_ready / sample_presented` 等高频 hostTiming 改为按窗口采样，并引入 lazy payload，避免主线程/显示链逐帧构造 JSON
- Decision: 第一批先处理公共放大器和最热路径，不在本轮同时重构 recovery 控制面，避免跨层风险失控。
- Risk/Blocker: trace 已异步化，但 `trace_projection` 仍是高频生产者；后续仍需继续做事件分级与降频，否则会把问题从同步阻塞转为异步积压。

- Date: 2026-04-05 | Status: in-progress
- Update: 已完成第二批主链隔离改造：
  - `crates/xbxengine/core/src/api/runtime/*` 把 runtime tick 的 rumble 路径改为“提交意图”而非同步执行，并在 `start/stop` 清理待执行请求，移除 runtime 内部 rumble backlog/drain
  - `src-tauri/src/mods/xbxengine/rumble_worker.rs` 新增独立 rumble worker，host bridge 只做 enqueue；worker 串行执行真实 `gamepad.play/stop`，按 target 合并最新态、限频派发、支持 clear/shutdown
  - `src-tauri/src/mods/xbxengine/trace_projection.rs` 对 `directGamingState / hostPresentState / videoTrackState` 增加语义变化门控与采样桶，避免时戳抖动、计数器自然增长和 track bytes 增长持续放大 trace
  - 现有 harness `recovery_integration_home_burst_input_rumble_submit_gap_and_latest_slot_overwrite_stays_local` 已纳入本轮验证，确认动作期 `burst input/rumble + submitGap + latestSlotOverwrite` 组合仍停留在 local absorb，不误升级恢复
- Decision: rumble 先在宿主侧彻底 worker 化，优先保证视频 present/tick 不再和 haptics 同链执行；trace gating 则继续收口高频状态，避免把同步阻塞转成异步积压。
- Risk/Blocker: 仍缺“修复后真实运行 trace”做最终闭环；如果新 trace 仍显示 display starvation，则下一层要继续评估 `present_frame` 宿主调用是否也需要完全 enqueue 化。

- Date: 2026-04-05 | Status: in-progress
- Update: 基于最新 trace 与 `moonlight-qt` 对照，已确认当前 macOS `native_video layer` 的根问题不是网络或解码，而是本地 present 供给链设计失衡：
  - `display link callback -> take_ready_frame() -> pending_sample -> run_on_main_thread() -> enqueueSampleBuffer()` 把视频提交绑到主线程队列
  - `ScheduledFrameSlot` 的单槽 `latest_frame.take()` 会在高刷 display tick 下天然放大 `noPendingFrame / starved / latestSlotOverwrite`
  - `submitGap + main-thread queue delay + single-slot overwrite` 叠加后，会把 `decode≈60` 压成 `present≈40~50`
- Decision: 本轮不再做旧链路的参数修补，直接按 `moonlight-qt` 设计原则做非兼容重构：
  - 主线程只负责 `AVSampleBufferDisplayLayer` 创建/销毁/布局
  - 视频 present 改成独立 worker 线程直接 `enqueueSampleBuffer`
  - `layer` 路径改用有界队列/两级缓冲，不再使用“取一帧即清空单槽”的模型
  - `starved/noPending` 重新定义为“连续供帧失配”，不再把高刷显示器上的单次空 tick 直接判成饥饿
- Risk/Blocker: 这是显式的非兼容重构，`native_video` 现有单测与 trace 口径需要一起迁移；若 presenter 线程模型与调度模型分批落地，短期内会出现编译/测试中间态，必须以最终集成结果为准。

- Date: 2026-04-06 | Status: in-progress
- Update: 已把 recovery 统一仲裁改造成“保留中心层，但按 action family / in-flight / upgrade 进行调度”：
  - [`crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs) 新增 `coalesced:keyframeInFlight` 与 `coalesced:decoderResetInFlight`，把同族命令合并从泛化 `cooldownSuppressed` 中拆出
  - [`crates/xbxengine/core/src/transport/rtc/recovery/repeat_suppression.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/repeat_suppression.rs) 改为按已执行动作 family 回放合并，不再把 `requestKeyframe` / `requestDecoderReset` 混成一个 suppression 结果
  - [`crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 允许 `transportAwaitRecoveryAnchor` 在同族 coalesce 态下继续触发 stage upgrade / decoder reset，不再被“伪 cooldown”锁死
  - [`crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs) 将命令落地调整为 family 语义：支持 `sameFamilyCoalesced:*`、`familyInFlight:*`、`familyUpgrade:keyframeInFlight->decoderReset`，并把语义写回 ledger/detail
  - [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 的 recovery ledger 现在区分 `pass` 与 `coalesced:*`，不再把所有非执行动作都记成 `suppressed:cooldownSuppressed`
- Decision: 继续保留统一仲裁原则，但中心层只负责分层去重、升级与预算，不再用单一 cooldown 覆盖不同恢复家族；`requestKeyframe` 不再压住 `decoderReset`，`decoderReset` 可显式升级吞并 keyframe-family 恢复链。
- Risk/Blocker: `transport::rtc::session::policy` 全量长跑测试仍在执行，暂未拿到完整结束信号；若其中存在只接受旧 `cooldownSuppressed` 文案的断言，还需要继续迁移到 family hold 语义。

- Date: 2026-04-06 | Status: in-progress
- Update: 基于最新 trace `runtime-trace-1775441228316-1.jsonl` 的代码对照，已确认当前主矛盾从“恢复策略是否触发”转为“恢复账本与执行回执是否能稳定闭环”：
  - [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 新增 `recent_recovery_decision_ledgers` ring buffer 写入，避免 `latest_recovery_decision_ledger` 被下一拍覆盖后丢失历史决策
  - [`crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs) 将 command result 回写改为按 `decision_id / observation_id` 反向命中历史 ledger，不再只更新当前 latest 单槽，并补入“latest 已轮转时历史 ledger 仍能回写”的定点测试
  - [`crates/xbxengine/core/src/runtime_stats_sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/runtime_stats_sink.rs) 为 `keyframeRequestEpisode` 增加 recent episode 轨迹，并把 `requested` 记录时机后移到 family gate 之后，避免 `coalesced / familyInFlight` 请求也创建假 episode，减少 `requested + sentAtMs=null` 的失真
- Decision: 不再继续调 recovery 参数阈值，先统一恢复执行事实源；trace 的 `ledger / episode / runtimeSummary(obs:*)` 允许继续并存，但事实归属必须先由 id 关联闭环保证。
- Risk/Blocker: 目前 `decoder reset` 仍主要通过 transport command semantic + escalation summary 观测，没有像 keyframe 一样独立 episode；若后续 trace 仍存在“已发送但不可归属”的盲区，需要继续把 decoder reset 也收敛成完整 episode。

- Date: 2026-04-06 | Status: in-progress
- Update: 针对同一份 trace 中“连接成功但首帧尚未出现时误触发 `transportAwaitRecoveryAnchor` 恢复升级”的问题，已在 [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 增加 startup pre-first-frame 门控：当 `Connected + priming/startup/handshaking + remoteTrackAttached + 尚无 decode/present 首帧反馈 + pipeline 未 stall` 时，`transportAwaitRecoveryKeyframe / ingressWaitKeyframe` 不再进入恢复升级；仅在该窗口过期后才重新允许升级。并补入 [`crates/xbxengine/core/src/transport/rtc/session/policy.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.test.rs) 两条定点测试，分别覆盖“窗口内不升级”和“窗口过期后恢复升级”。
- Decision: 将“远端固定慢首帧”与“本地恢复失败”等价视为错误建模；startup pre-first-frame 一律优先按远端启动特征处理，除非窗口过期或本地解码/渲染明确 stall。
- Risk/Blocker: 当前门控主要覆盖 `transportAwaitRecoveryKeyframe / ingressWaitKeyframe`；如果后续 trace 还显示其他 startup 诊断标签在首帧前误升级，需要继续把这类标签并入同一 pre-first-frame 启动门控集合。

- Date: 2026-04-06 | Status: in-progress
- Update: 继续对照 [`runtime-trace-1775441228316-1.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775441228316-1.jsonl) 与 [`crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs) 后，已确认“恢复动作已成功但从升级恢复到恢复正常耗时过长”的直接根因是 `SupplyStarved` 缺少弱退出路径：状态机此前只允许 `Ready -> StableServing`，导致 `clean anchor + healthy chain + live decode/present 已恢复` 但累计 supply/drop 指标仍偏坏的场景长期停在 `recovering/starved`。本轮已将 `ServingReady` 从 `RebuildingSupply` 扩展到 `SupplyStarved`，允许 owner 先回到 `DegradedServing`，再由 steady 分支继续吸收 supply 回稳；同时在 [`crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.test.rs) 补入 `clean_anchor_recovery_can_exit_supply_starved_to_degraded_serving_before_full_supply_reset` 回归，锁住“命令成功后不再长期挂死在恢复态”的出口。
- Decision: 统一仲裁继续保留，但 owner 的完成判定必须拆成“恢复主链已恢复”和“供给完全回稳”两个层级；前者负责尽快退出 `recovering/starved`，后者才负责回到 `stable-serving`。
- Risk/Blocker: 这次只打通了 owner 的弱退出路径，还没有把同类场景完整上提到 `session policy` 长序列 harness；如果下一份 trace 仍显示长时间 `degraded-serving` 卡住，需要继续收紧 display supply 的累计 drop 语义与 recovery owner 的供给回稳判定。

- Date: 2026-04-06 | Status: in-progress
- Update: 基于后续 trace 暴露出的两类新回归，已在 [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 补齐 session 侧收口：
  - `resolve_recovery_state()` 已从旧的 `recovering -> recovered` 二元结案改成正式三段式 `recovering -> ramp-up -> stable`：`VideoSchedulingOwnerState::StableServing` 在当前 recovery episode 尚未收口时先映射到 `RampUp`，只有 episode 真正完成结案后才映射到 `Stable`
  - `VideoSchedulingOwnerState::DegradedServing` 不再被 session 直接记作恢复完成，而是继续保留在当前 recovery episode 内，避免 owner 的弱退出被误判为“已稳定”
  - pre-first-frame 门控已扩展到 `displaySupplyDegraded`，且只在 `startup/handshaking/priming + remoteTrackAttached + 尚无 decode/present 首帧反馈 + pipeline 未 stall` 下生效，避免首帧前 display pressure 误升级恢复
  - `adapterIdleTimeout` 的首帧门控也前移到了 `active_media_recovery_intent` 分支，避免 signal 走 intent 路径时绕过原有 startup hold
  - 已新增 ramp-up 吸收窗口：在当前 recovery epoch 的 clean anchor 刚建立、pipeline 仍健康且 episode 尚未收口时，`displaySupplyDegraded`、`adapterIdleTimeout`、短窗 `transportAwaitRecoveryAnchor` 等轻信号会被并入同一 recovery episode，不再重新立案；只有重度 `transportAwaitRecoveryAnchor` 仍允许重新升级
  - [`crates/xbxengine/core/src/transport/rtc/session/policy.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.test.rs) 已新增 `connected_track_attached_without_first_frame_feedback_does_not_escalate_display_supply_degraded_during_priming_window`、`degraded_serving_does_not_mark_session_recovered_before_stable_serving`、`recovery_integration_ramp_up_absorbs_display_idle_and_short_transport_await_before_stable`、`recovery_integration_ramp_up_still_reescalates_on_severe_transport_await`，并复跑 `connected_track_attached_without_host_feedback*` / `connected_track_attached_without_first_frame_feedback*` 定向测试
- Decision: 统一仲裁原则继续保留，但 session 的恢复生命周期不再是简单 cooldown 止血，而是显式拆成 `Recovering / RampUp / Stable`。`RampUp` 只负责吸收恢复后短暂回摆，不会吞掉真正的重故障升级。
- Risk/Blocker: 当前这轮已锁住首帧前误升级与恢复后短暂回摆重燃两类主回归；如果新 trace 仍出现长时间卡在 `RampUp` 或重故障被误吸收，需要继续检查 clean-anchor 归属与 severe transport await 的升级阈值是否还存在口径漂移。

- Date: 2026-04-06 | Status: in-progress
- Update: 针对 [`runtime-trace-1775446995939-1.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775446995939-1.jsonl) 中“同一 unresolved gap/reference-chain debt 长窗内反复重发恢复动作”的问题，已在 [`crates/xbxengine/core/src/transport/rtc/recovery/repeat_suppression.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/repeat_suppression.rs) 将 `transportAwaitRecoveryAnchor` 的 repeat suppression 从“依赖 episode 标志短窗判断”改成“依赖新鲜 unresolved transport-await 证据 + 尚无客观恢复成功”的 family-level coalescing：只要 timeline / anchor candidate 仍持续声明 `awaitingRecoveryKeyframe / referenceChainUnrecoverable` 一类未收口债务，就复用当前 in-flight keyframe/decoder-reset family，不再每隔数百毫秒重新立案；一旦出现 clean anchor + healthy chain + 新鲜 decode/present 输出，旧 debt 立即失效，后续新 epoch 仍可重开恢复。
- Decision: 不再通过加大 cooldown 止血，而是把“同一 unresolved transport-await debt”定义成仲裁层的一等事实；episode 标志只作为附属状态，不能决定是否重立案。
- Validation: `cargo test -p xbxengine recovery_integration_same_unresolved_gap_transport_await_reuses_in_flight_family -- --nocapture`、`cargo test -p xbxengine recovery_integration_transport_await_reopens_after_clean_anchor_and_new_recovery_epoch -- --nocapture`、`cargo test -p xbxengine recovery_integration_stale_transport_await_after_completion_evidence_stays_no_signal -- --nocapture`、`cargo test -p xbxengine cooldown_suppressed_cannot_linger_when_connected_track_attached_but_no_present_decode_progress -- --nocapture`、`cargo test -p xbxengine recovery_integration_ramp_up_absorbs_display_idle_and_short_transport_await_before_stable -- --nocapture`、`cargo check -p xbxengine`
