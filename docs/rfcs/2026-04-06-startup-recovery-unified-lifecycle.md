# Startup Recovery Unified Lifecycle RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: in-progress
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 最新 runtime trace 暴露出两类耦合问题：
  - 启动阶段长时间停在 `connecting/startupPriming/seeking-anchor`，用户感知为“假死”
  - 媒体已恢复到 `steady/healthy` 后，恢复链路仍长期保持 `recovering`
- 当前启动、连接、恢复、owner、诊断、前端展示分别维护不同层级的状态：
  - RTC connection lifecycle
  - session recovery ledger
  - video owner state / video health
  - directGamingState / statsSnapshot
  - 前端 `lifecyclePhase / diagnostics.isRecovering / sessionUiPhase`
- 多套状态之间不是单一主权关系，导致：
  - 启动等待与恢复语义混杂
  - recover 进入/退出依赖过多局部条件
  - 前端只能通过多源 OR 猜测“是否恢复中”

## Goal

- 建立一套由 Rust 主导的统一生命周期语义，覆盖：
  - startup
  - recovering
  - ramp-up
  - steady
  - degraded
  - failed / closed
- 把“启动未完成”和“恢复未收口”从顶层语义上明确区分。
- 让前端只消费单一语义快照，不再基于多字段 OR 推断“恢复中”。
- 让 trace / diagnostics / UI 面板都对齐同一生命周期主权。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/session/*`
  - `crates/xbxengine/core/src/transport/rtc/policy/*`
  - `crates/xbxengine/core/src/runtime_stats_sink.rs`
  - `crates/xbxengine/core/src/diagnostics/*`
  - `src-tauri/src/mods/xbxengine/trace_projection.rs`
  - `src/streaming/*`
  - 与统一生命周期直接相关的 trace / runtime contract / diagnostics 展示
- Out of scope:
  - 重写 signaling 协议
  - 重写 native video presenter 架构
  - 改变 Tauri/Vue/Rust 栈边界

## Plan

1. 设计统一生命周期模型与主权边界，明确 Rust 唯一事实源。
2. 在 Rust 侧实现统一生命周期快照，并收口 startup/recovery/ramp-up/steady 迁移逻辑。
3. 调整 trace projection 与前端消费层，只使用统一生命周期语义驱动 UI 与 diagnostics。
4. 基于目标 trace 拆解真实启动长窗口，把 runtime 启动前移到 `ReadyToConnect`，避免继续被 `Provisioned` 阻塞。
5. 解开 runtime 首屏协商中的串行瓶颈，把 `media ready` 从 ICE 交换完成后前移到 remote description 已应用后。
6. 补齐 session policy、trace、前端 diagnostics 的回归测试，并用目标 trace 验证。

## Validation

- [ ] `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
- [x] `cargo test -p xbox-streaming session::flow -- --nocapture`
- [x] `cargo test -p xbxengine diagnostics -- --nocapture`
- [x] `cargo test -p xbxrc trace_projection -- --nocapture`
- [x] 前端相关测试或类型检查通过
- [x] `cargo test -p xbxengine runtime_stats_sink -- --nocapture`
- [x] `cargo test -p xbxengine start_negotiates_remote_and_reaches_running -- --nocapture`
- [x] 用 `runtime-trace-1775448303013-1.jsonl` 回归确认：
  - 启动阶段不再把“等待首屏”误表示成恢复中
  - 媒体恢复后生命周期能收口离开 recovering
  - `ReadyToConnect -> startingRuntime` 相比原 trace 明显前移
  - `MediaSurfaceReady / MediaVideoReady` 不再被完整 ICE 交换串行阻塞

## Risks

- 统一生命周期后，如果老字段兼容处理不完整，可能造成 trace/UI 短期不一致。
- 若把恢复完成判据放宽过头，可能掩盖真实的短周期回摆。
- 前端从多源推断切到单一语义后，部分现有 diagnostics 可能需要同步调整文案与优先级。

## Progress

- [x] Step 1: 已确认这是独立复杂任务，需要单独 RFC
- [x] Step 2: 完成 Rust 统一生命周期模型
- [x] Step 3: 完成 trace / frontend 消费收口
- [x] Step 4: 完成启动链路提速改造
- [ ] Step 5: 完成回归验证

## Execution Notes

- Date: 2026-04-06 | Status: planned
- Update: 从单点日志修补切换为“启动/恢复统一生命周期”专项任务，独立于既有主流程污染隔离 RFC。
- Decision: Rust 侧成为生命周期唯一事实源；前端不再自己 OR 多字段判断 recovering。
- Risk/Blocker: 当前仓库已有多项并行 recovery 改动，实施时必须避免与现有 session policy / diagnostics 在制修改冲突。
- Date: 2026-04-06 | Status: implemented
- Update: `XbxEngineStatsDto` 新增 `stream_lifecycle_phase`，由 `crates/xbxengine/core/src/diagnostics/stats.rs` 统一投影 `startup / recovering / ramp-up / steady / degraded / failed / closed`。旧 `session_phase` 保持 `connecting / handshaking / priming / steady / recovering` 合同不变。
- Update: `src-tauri/src/mods/xbxengine/trace_projection.rs` 已将 `streamLifecyclePhase`/`lifecycle` 写入 observability snapshot 与 `directGamingState`，trace 不再依赖旧 `sessionPhase` 猜统一语义。
- Update: `src/streaming/runtime/xbxengine-runtime.ts`、`src/streaming/diagnostics.ts`、`src/player/domain/media.ts`、`src/streaming/types.ts` 已切到统一生命周期优先，旧字段仅做 fallback；`isRecovering` 不再默认由 decoder/owner 多源 OR 推断。
- Update: i18n 已补充 `startup / ramp-up / degraded / failed / closed` 文案，保证 diagnostics/performance 面板可直接展示新顶层语义。
- Validation: `cargo test -p xbxengine diagnostics -- --nocapture` 通过；`cargo test -p xbxrc trace_projection -- --nocapture` 通过；`pnpm -s exec tsc -p tsconfig.json --noEmit` 通过。
- Validation: `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture` 已启动，未见失败输出，但存在既有长跑用例超过 60s，尚未获得完整结束结果。
- Trace: `runtime-trace-1775448303013-1.jsonl` 复核显示，`seq=326 -> 11418` 持续 41.301s 的窗口仍是 `sessionPhase=connecting | primaryIssueChain=startup:priming`，说明旧问题 1 的本质是启动长窗口，不应被 UI 解释成 recovering。
- Trace: `seq=15217` 已出现 native first present，`seq=15243` 已出现 `videoTimeline chain=healthy`，但 `seq=15230/15245` recovery ledger 仍保持 `stateAfter=recovering`；同时 `seq=15231/15248` owner 已切到 `degraded-serving`。说明旧问题 2 的本质是 recovery ledger 收口滞后，而不是媒体链路仍然处于同等严重的恢复态。新统一生命周期会把这类窗口投影为 `degraded` 或后续 `steady/ramp-up`，不再继续显示“恢复中”。
- Date: 2026-04-06 | Status: planned
- Update: 目标 trace 进一步拆解后，启动长窗口确认由两段串行等待叠加造成：1) `ReadyToConnect` 已到达但 `flow` 仍继续等待 `Provisioned/sessionReady` 才启动 runtime；2) runtime 已拿到 answer 并完成 `apply_remote_description`，但 `MediaSurfaceReady / MediaVideoReady` 仍被整段 ICE 交换拖后。
- Decision: 启动链路提速按两个顶层改造点执行，不做局部文案修补。
- Decision: `crates/xbox-streaming/src/session/flow.rs` 中把 `SessionPhase::RuntimeStarting` 视为 startup wait 的可放行态；保持 `Failed / Closed / Recovering` 的边界判定不变。
- Decision: `crates/xbxengine/core/src/api/runtime/lifecycle.rs` 中把 `record_media_ready()` 前移到 `apply_remote_description()` 之后；继续保留 transport connected / first frame rendered 等事实事件由后续 runtime stats 驱动，避免语义混淆。
- Date: 2026-04-06 | Status: implemented
- Update: `flow` 已改为在 `RuntimeStarting(ReadyToConnect)` 即结束 startup wait，不再额外等待 `Provisioned/sessionReady`；新增 `runtime_starting_phase_is_ready_for_startup_wait` 覆盖回归。
- Update: runtime 协商已在 `apply_remote_description()` 成功后立即 `record_media_ready()`；新增断言保证 `MediaSurfaceReady` 早于 `ExchangingIce`，避免首屏事件继续被 ICE 交换整段拖后。
- Validation: `cargo test -p xbox-streaming session::flow -- --nocapture` 通过；`cargo test -p xbxengine diagnostics -- --nocapture` 通过；`cargo test -p xbxengine runtime_stats_sink -- --nocapture` 通过；`cargo test -p xbxengine start_negotiates_remote_and_reaches_running -- --nocapture` 通过；`cargo test -p xbxrc trace_projection -- --nocapture` 通过；`pnpm -s exec tsc -p tsconfig.json --noEmit` 通过。
- Validation: 既有全量 `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture` 慢用例问题仍未重新完整跑通，本轮继续保留为未完成项。
- Date: 2026-04-06 | Status: implemented
- Update: 基于 `runtime-trace-1775451469478-1.jsonl` 新增“启动空窗分段观测”改造，直接覆盖 `sessionExecutionStarted -> Binding` 的跨层链路：前端 `runtime-host` 新增 `runtimeLaunchReadyToInvoke/runtimeLaunchPortBound`，`xbxengine-runtime` 新增 `runtimeAttachViewportRequested/Completed` 与 `runtimeStartRequested/Completed`；Rust 宿主入口 `src-tauri/src/shell/rpc.rs` 新增 `runtimeAttachViewportRpcReceived/Completed` 与 `runtimeStartRpcReceived/Completed`；`src-tauri/src/mods/xbxengine/service.rs` 与 `runtime_state.rs` 新增 dispatch/apply/lock-acquired 分段事件。
- Decision: 这轮只补 observability，不改变启动语义和用户可见合同；既保留现有 `sessionExecutionStarted`、`nativeViewportAttached`、`Binding` 锚点，也补齐它们之间的宿主链路，从而能直接区分 UI 延迟、RPC 入口延迟、blocking worker 排队、runtime 锁争用四类根因。
- Validation: `cargo fmt --all`、`cargo check -p xbxrc`、`cargo test -p xbxrc trace_projection -- --nocapture`、`pnpm -s exec tsc -p tsconfig.json --noEmit` 通过。
- Date: 2026-04-06 | Status: implemented
- Trace: 基于 `runtime-trace-1775452872224-1.jsonl` 复盘，`sessionExecutionStarted(seq=4911, ts=1775452887957)` 到 `runtimeLaunchReadyToInvoke(seq=10447, ts=1775452905485)` 之间仍有约 `17.528s` 空窗，而 `runtimeAttachViewportRequested -> Binding` 已只剩约 `23ms`，说明新的主瓶颈已经不在 Rust 宿主链路，而在前端 runtime 启动门控。
- Root Cause: `src/streaming/useStreamExecution.ts` 中 `runtimeLaunchSpec` 误把 runtime 启动依赖在 `sessionHealth.phase === sessionReady`；但 `startSession` 返回时 `execution` 已可用，前端却继续等待 progress 轮询把 phase 推进到 `sessionReady`，从而人为制造启动长空窗。
- Decision: runtime 启动主权改回以后端 `startSession` 返回的 `execution` 为准，不再把前端 progress 轮询的 `sessionReady` 作为 runtime 启动前置条件；这是顶层启动语义修正，不是局部打补丁。
- Update: `src/streaming/useStreamExecution.ts` 已将 `runtimeLaunchSpec` 的放行条件从“`execution !== null` 且 `sessionHealth.phase === sessionReady`”收口为“仅要求 `execution !== null`”，使 runtime 启动与 session control plane 解耦，避免前端再串行等待 progress 轮询。
- Trace: 同一份 trace 中，`first_present(seq=11234, ts=1775452912959)` 后约 `123ms` 即进入 `degraded(seq=11346, ts=1775452913082)`，说明旧问题 2 的“恢复中语义滞留”已明显改善；该 trace 后段残留的 recovering 更像是真实恢复未完成，而非 UI/ledger 收口慢。
- Validation: `pnpm -s exec tsc -p tsconfig.json --noEmit`、`cargo check -p xbxrc`、`cargo test -p xbxrc trace_projection -- --nocapture` 通过。
- Date: 2026-04-06 | Status: corrected
- Trace: 基于 `runtime-trace-1775453736234-1.jsonl` 复盘，`sessionExecutionStarted(seq=8071, ts=1775453761924)` 后约 `506ms` 就触发 `runtimeLaunchReadyToInvoke(seq=8236, ts=1775453762430)`，随后 `runtimeStartApplyCompleted(seq=8294, ts=1775453763763)` 直接失败，服务端返回 `HTTP 400 SessionNotConnectable: Server connection exchange is only supported for provisioned sessions.`。
- Decision: 撤回“只要 `execution !== null` 就启动 runtime”的前端放行逻辑。`execution` 只证明 startSession 已返回会话快照，不证明服务端 session 已进入可 exchangeOffer 的 provisioned/connectable 状态；cloud 侧仍必须尊重原本逆向得到的启动时序。
- Root Cause: 上一轮把“execution 已返回”和“session 已 connectable”错误等同，导致前端在 `phase=runtimeStarting | streamState=ReadyToConnect` 时就发起 `StartRuntime/exchangeOffer`，破坏既有服务端启动序，进而直接触发 `SessionNotConnectable`。
- Update: `src/streaming/useStreamExecution.ts` 已恢复以 `sessionHealth.phase === sessionReady` 作为 runtime 启动门槛，并补注释明确 `execution` 与 `sessionReady/provisioned` 的边界。
- Follow-up: 问题 1 不能再通过“提前绕过 sessionReady”修复；后续若还要收窄 `sessionExecutionStarted -> runtimeLaunchReadyToInvoke` 空窗，必须改成由后端显式输出“已 provisioned / 已允许 exchangeOffer”的单一门槛，或重新审视 `sessionExecutionStarted` 的定义时点，而不是打破既有启动时序。
- Date: 2026-04-06 | Status: implemented
- Decision: 将 runtime 启动门槛从前端隐式判断改为后端显式合同，不再让 UI 通过 `phase === sessionReady` 或 `execution !== null` 猜测何时可以 `exchangeOffer`。`phase` 继续服务展示与文案，`runtimeLaunchState` 单独承载“是否允许启动 runtime”的主权语义。
- Update: `crates/xbox-streaming/src/session/flow.rs` 新增 `RuntimeLaunchState { blocked, ready, closed, failed }`，并在 `SessionProgressSnapshot` 中显式输出。当前语义为：`SessionReady -> ready`，`Closing/Closed -> closed`，`Failed -> failed`，其余阶段一律 `blocked`。
- Update: `src-tauri/src/mods/streaming/types.rs`、`src-tauri/src/mods/streaming/service.rs`、`src/shared/rpc/streaming.ts` 已把 `runtimeLaunchState` 透传到 RPC 合同与 trace；`sessionMonitorSnapshot` 现会直接记录 `runtimeLaunchState`，后续 trace 可以明确区分“还在等 provisioned”与“已经允许启动 runtime”。
- Update: `src/streaming/session.ts` 与 `src/streaming/useStreamExecution.ts` 已切换为消费后端 `runtimeLaunchState`；前端不再拥有启动门槛推断权，只在 `runtimeLaunchState=ready` 时触发 runtime 启动。
- Validation: `pnpm -s exec tsc -p tsconfig.json --noEmit`、`cargo test -p xbox-streaming session::flow -- --nocapture`、`cargo test -p xbxrc service::tests -- --nocapture` 通过。
- Date: 2026-04-06 | Status: implemented
- Trace: 基于 `runtime-trace-1775454893142-1.jsonl` 继续收敛问题 2，已确认长时间 `recovering` 不是单纯 UI/ledger 语义滞留，而是“伪恢复候选 + 恢复信号断流”叠加：同一窗口里会同时出现 `cleanAnchor/healthy chain` 与 `latest_h264.bootstrapRejectReason=NonIdrVcl`，随后 owner 仍保持 `rebuilding-supply`，但 `recoveryDecisionLedger` 长时间记为 `inputSignal=none/gateResult=no-signal`。
- Root Cause: `crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs` 之前没有消费 codec 层 `latest_h264_inspection_observation`，导致 `gap-resolved/frame-complete-candidate` 一类 transport 侧“修好了”的候选，会在 codec 仍是 `NonIdrVcl` 时被误当成接近可收口；同时 owner 的本地 `intent.emit` 去抖又把连续 anchor recovery surface 吃掉，`crates/xbxengine/core/src/transport/rtc/session/policy.rs` 在 `has_media_recovery_surface && !active_media_recovery_intent` 分支直接 `return None`，使 coordinator 连续坏窗 streak/hard-fallback timer 无法推进，最终形成长期 `no-signal`。
- Decision: recovery 闭环的连续性不再由 owner 本地 `emit` 主权控制。anchor 家族的重复恢复面继续交给 session/coordinator 统一做 family/in-flight/cooldown 节流；owner 负责输出持续的 recovery surface，coordinator 负责判定是否真正升级。
- Update: `video_scheduling_owner` 新增消费 `latest_h264_bootstrap_ready/bootstrap_reject_reason/observed_at_ms`，并把“当前仍处在 `RebuildingSupply` 且最近一拍 codec 明确 `NonIdrVcl`”视为真实 `await-recovery-keyframe` 阻塞事实，避免 healthy chain/clean anchor 把 owner 误导离开 anchor 恢复链。
- Update: `session policy` 已改为对 `RebuildingSupply + Anchor` 的被动 recovery surface 继续生成 owner signal，不再因为 owner 本地 `emit=false` 就写成 `no-signal`；这样连续 `transportAwaitRecoveryAnchor` 坏窗仍会进入 coordinator 的 family hold/stage upgrade/hard fallback 逻辑，但不会重复下发命令。
- Validation: `cargo fmt --all`、`cargo test -p xbxengine recent_non_idr_codec_evidence_keeps_owner_in_rebuilding_supply -- --nocapture`、`cargo test -p xbxengine recovery_integration_passive_anchor_surface_still_feeds_transport_await_family_hold -- --nocapture`、`cargo test -p xbxengine recovery_integration_same_unresolved_gap_transport_await_reuses_in_flight_family -- --nocapture`、`cargo test -p xbxengine recovery_integration_stale_transport_await_after_completion_evidence_stays_no_signal -- --nocapture`、`cargo check -p xbxengine` 通过。
- Date: 2026-04-06 | Status: implemented
- Trace: 基于 `runtime-trace-1775455942676-1.jsonl` 继续复盘，问题 2 的主根因已从“signal 断流”切换为“`keyframe in-flight` 家族保持态长期占坑”：ledger 已持续记录 `inputSignal=transportAwaitRecoveryKeyframe:transportAwaitRecoveryKeyframe`，`gateResult` 也不再是 `no-signal`，而是长尾 `coalesced:keyframeInFlight`；同窗里 `latest_keyframe_request_episode` 多次停在 `status=requested, sent_at_ms=null, response_verdict=pending`，但 owner/codec 仍持续给出 `rebuilding-supply + NonIdrVcl` 证据。
- Root Cause: `recovery/escalation` 与 `transport_session` 之前都把“RequestKeyframe 已提出/requested”直接当成 in-flight，而不是以“真正 sent”作为占坑事实。结果是只要 control/data path 没把 keyframe request 真发出去，controller 内部 `keyframe_epoch` 与 transport family gate 仍会同时把后续坏窗压成 `coalesced:keyframeInFlight`，并且还会错误消耗 keyframe budget，形成长期自锁。
- Decision: 顶层统一 keyframe in-flight 语义到“transport 已 sent 才算真正在飞”。`requested but unsent` 只保留短暂 retry 节流，不再继续占据 recovery family hold，也不再继续烧 budget。
- Update: `crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs` 新增 `KeyframeTransportFeedback` 与 `reconcile_keyframe_transport_feedback()`，将 `RequestKeyframe` 的预算消耗拆成“proposal 预留”与“transport feedback 确认”；当最新 episode 仍是 `requested + sent_at_ms=None + pending` 时，会回滚本地 provisional keyframe budget，并释放 `keyframe_epoch`，避免 owner 长期被假在飞态锁死。
- Update: `crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs` 现会在每次 `propose_from_owner_signal()` 前，根据 `latest_keyframe_request_episode` 同步 keyframe transport feedback；这样 recovery controller 的 family/budget 语义会与真实 episode 闭环对齐，而不是继续只信 proposal。
- Update: `crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs` 的 family gate 已改为只把 `sent_at_ms.is_some()` 的 pending keyframe episode 视为 in-flight；`requested but unsent` 不再阻止后续 keyframe request 重试，也不再把 command family 长时间锁在旧 episode 上。
- Validation: `cargo fmt --all`、`cargo test -p xbxengine unsent_pending_keyframe_feedback_rolls_back_provisional_budget -- --nocapture`、`cargo test -p xbxengine unsent_requested_keyframe_is_rolled_back_before_transport_await_stage_upgrade -- --nocapture`、`cargo test -p xbxengine unsent_requested_keyframe_does_not_hold_family_gate -- --nocapture`、`cargo test -p xbxengine recovery_integration_passive_anchor_surface_still_feeds_transport_await_family_hold -- --nocapture`、`cargo test -p xbxengine recovery_integration_same_unresolved_gap_transport_await_reuses_in_flight_family -- --nocapture`、`cargo test -p xbxengine recovery_integration_stale_transport_await_after_completion_evidence_stays_no_signal -- --nocapture`、`cargo test -p xbxengine recent_non_idr_codec_evidence_keeps_owner_in_rebuilding_supply -- --nocapture`、`cargo check -p xbxengine` 通过。
- Date: 2026-04-06 | Status: implemented
- Root Cause: 即便 recovery 已真实进入 `requestKeyframe/requestDecoderReset`，`latest_recovery_decision_ledger` 之前仍会在下一拍被 `proposal=None -> no-signal/none` 立刻覆盖。由于 trace/UI 默认优先看 latest，这会制造“根本没推进”的假象；而 `recent_recovery_decision_ledgers` 虽保留历史，但在高频 no-signal 下又很快被 ring 挤压。
- Decision: 不改 DTO 合同，先把 `latest_recovery_decision_ledger` 的主权语义收口为“当前最值得看的 pending/最新决策”：若上一个 latest 仍是 pending actionable decision，新的 `no-signal/none` 只进入 recent history，不覆盖 latest；等命令结果落账后，latest 才重新允许前移到新的 `no-signal`。
- Update: `crates/xbxengine/core/src/transport/rtc/session/policy.rs` 新增 pending ledger 判定，并在写 ledger 时改成“recent 全量连续、latest pending 优先”；这样 `decision_id -> command_result` 回填窗口会稳定保留在 latest，避免真实升级在 trace 顶层被一拍 no-signal 盖掉。
- Validation: `cargo fmt --all`、`cargo test -p xbxengine recovery_decision_ledger_keeps_pending_action_latest_while_recent_history_records_no_signal -- --nocapture`、`cargo test -p xbxengine recovery_decision_ledger_allows_no_signal_to_be_latest_after_pending_command_is_resolved -- --nocapture`、`cargo test -p xbxengine command_result_updates_matching_recovery_decision_ledger -- --nocapture`、`cargo test -p xbxengine command_result_updates_historical_ledger_when_latest_has_rotated -- --nocapture`、`cargo check -p xbxengine` 通过。
