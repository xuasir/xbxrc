# 昂贵恢复顶层重构 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 近期多份 runtime trace 显示，`bootstrapMissingSps`、`transportAwaitRecoveryAnchor`、`displaySupplyDegraded`、`waitKeyframe` 等轻量异常会过早把系统拉入恢复叙事。
- 现状将“异常观测”“根因诊断”“恢复资格”“恢复动作执行”高度耦合，导致：
  - 小波动与真实故障共用 `recovering` 语义；
  - `coalesced:keyframeInFlight` / `waitForBurst` / `cooldownSuppressed` 等中间决策长期占据恢复表象；
  - `deferred/unsent` 动作仍可能污染 in-flight / budget 语义；
  - display / anchor / decoder / transport 多域信号互相放大，形成“恢复态假死”。
- 恢复属于昂贵操作，必须把“局部自愈”和“昂贵恢复”分层，让系统先证明自愈失败，再允许升级。

## Goal

- 重构恢复系统顶层语义，使小波动、启动 bootstrap、局部自愈与昂贵恢复在状态机上清晰分层。
- 建立“证据驱动”的恢复仲裁：只有在持续无进展且局部自愈失败后，才允许执行高成本恢复。
- 重构 in-flight / budget / 域仲裁语义，避免 deferred/coalesced 假动作长期锁死恢复链。
- 为 UI / trace / runtime stats 提供稳定、可解释的恢复阶段语义和阻塞原因。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/session/policy.rs`
  - `crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/*`
  - `crates/xbxengine/core/src/runtime_stats_sink.rs`
  - `crates/xbxengine/core/src/diagnostics/stats.rs`
  - `src-tauri/src/mods/xbxengine/trace_projection.rs`
  - 相关 runtime/protocol/frontend diagnostics 合同与测试矩阵
- Out of scope:
  - 新传输协议或新媒体栈
  - 非当前 canonical Tauri + Vue 3 + Rust 架构的平行实现
  - 仅靠 trace 阈值“止血”的临时补丁，不改变顶层恢复模型

## Plan

1. 建立新的恢复分层模型：`observe-anomaly / local-self-healing / recovery-eligible / active-recovery / recovery-blocked`，并将 startup bootstrap 从常规恢复叙事中剥离。
2. 重构恢复动作预算、in-flight、proposal 与 command result 语义，只对真实执行动作计费，并要求昂贵恢复具备失败证据闭环。
3. 引入 primary owner / 域仲裁与阻塞诊断，避免 anchor/display/decoder/transport 多域互相升级。
4. 更新 trace/runtime stats/frontend diagnostics 合同与测试矩阵，确保新语义可观测、可验证。
5. 完成 Report 与项目任务跟踪收口。

## Validation

- [x] `cargo fmt --all`
- [x] `cargo test -p xbxengine transport::rtc::policy::video_scheduling_owner -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`
- [ ] `cargo test -p xbxengine transport::rtc::recovery::repeat_suppression -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
- [x] `cargo test -p xbxengine runtime_stats_sink -- --nocapture`
- [x] `cargo check -p xbxengine`

## Risks

- 顶层语义改动会影响现有大量 recovery/owner/session 测试，回归面大。
- 如果 owner 与 coordinator 的职责边界不够清晰，可能只是把旧耦合换个名字继续存在。
- runtime stats / trace projection / frontend diagnostics 合同调整若不完整，容易出现 UI 与日志语义漂移。

## Progress

- [x] Step 1: 建立新的恢复分层模型与 RFC 基线
- [x] Step 2: 完成 Phase 1 状态机与 bootstrap/local-self-healing 语义改造
- [x] Step 3: 完成 Phase 2 budget/in-flight/证据门禁改造
- [x] Step 4: 完成 Phase 3 primary owner/域仲裁/阻塞诊断改造
- [x] Step 5: 完成验证、Report 与任务跟踪收口

## Execution Notes

- Date: 2026-04-06 | Status: planned
- Update: 建立顶层恢复重构 RFC，明确三阶段设计目标、范围、验证项和风险。
- Decision: 本任务不走最小补丁路线，直接完成完整三阶段改造并统一收口文档。
- Risk/Blocker: 当前工作区存在多处 recovery 相关未提交修改，实施时需要严格兼容现有改动，避免误回退用户工作。
- Date: 2026-04-06 | Status: in-progress
- Update: 完成 Phase 1 语义基线，把 session/policy 的粗粒度 `recovering` 拆成 `observing / local-self-healing / recovery-eligible / active-recovery / recovery-blocked`，并同步投影到 runtime stats / trace projection / i18n。
- Decision: 顶层生命周期不再把“有异常”和“已执行昂贵恢复”混成一个状态；无动作但有异常时应落在 `observing / local-self-healing / recovery-eligible / recovery-blocked`，只有真实恢复动作才进入 `active-recovery`。
- Date: 2026-04-06 | Status: in-progress
- Update: 完成 Phase 2 执行语义收口。`RecoveryActionContract` 改为“执行后记账”；keyframe 仍按 `sent_at_ms` 确认占预算；decoder-reset 与 reconnect 通过 coordinator 同步执行成功事实；`Reconfigure` 补了 failure-evidence gate，避免轻波动/短时 reconfigure 直接升级昂贵 reset。
- Decision: `begin_recovery_epoch()` 现在同时清空 `last_keyframe_request_at / last_decoder_reset_at`，确保新 epoch 真正释放上一轮 in-flight/cooldown 记忆，不再把旧恢复链黏到新轮次。
- Date: 2026-04-06 | Status: completed
- Update: 完成 Phase 3 语义收口。`runtime_summary / primary_issue_chain / latest_decision_summary` 已纳入新 phase；前端 diagnostics 识别并展示新生命周期；`latestDecisionSummary` 支持解析 `decision:* / phase:* / owner:*` 三类摘要。
- Validation: 已通过 `cargo fmt --all`、`cargo test -p xbxengine transport::rtc::recovery::escalation -- --nocapture`、`cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`、`cargo test -p xbxengine runtime_stats_sink -- --nocapture`、`cargo test -p xbxengine recovery_integration_transport_await_reopens_after_clean_anchor_and_new_recovery_epoch -- --nocapture`、`cargo check -p xbxengine`。
- Risk/Blocker: `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture` 在既有慢测上长时间未返回最终结束码；期间已观察到本轮新增/受影响 recovery 相关用例全部通过，但未拿到整套 suite 的最终退出结果。
- Date: 2026-04-06 | Status: completed
- Update: 基于 `runtime-trace-1775483846251-1.jsonl` 的复盘，继续收口 `session/policy` 顶层语义：`transportAwaitRecoveryAnchor` 的首次探测性 `requestKeyframe` 不再直接记为 `active-recovery`；在新 recovery epoch 的首个 transport-await keyframe probe 上，ledger 统一先落在 `local-self-healing`。同时把 `bootstrapMissingSps / bootstrapMissingPps / inspectionRejectInvalidSliceHeader` 在首帧反馈建立前的语义继续剥离出 `recovery-eligible / active-recovery`，即使超出 startup grace、真的发出了探测性 keyframe，请求也只作为局部自愈观测，不再宣称系统已进入昂贵恢复态。
- Decision: 顶层 `active-recovery` 只保留给“已越过探测阶段的昂贵恢复执行”；对 `transportAwaitRecoveryAnchor` 的 first probe，和首帧前 bootstrap reject 驱动的 probe，执行命令与顶层恢复语义解耦，避免 trace/UI 再把弱 anchor 证据误记成正式恢复升级。
- Validation: 已补 `session/policy` 定向回归，覆盖首帧前 bootstrap 探测和 priming 后 transport-await 首次探测的 `state_after` 合同；整套 `transport::rtc::session::policy` 慢测仍需后续拿最终退出码。
- Date: 2026-04-06 | Status: completed
- Update: 继续收口 Phase 4 的剩余漏口。`coordinator` 现在在 `transportAwaitRecoveryAnchor` 已触发 decoder reset 且后续再次命中 keyframe-stage failure evidence 时，统一返回 `CoalescedDecoderResetInFlight`，不再重复发出第二个高成本 reset；`session/policy` 测试合同同步全部对齐 `pass:localProbe` 与 pre-first-frame probation 的新语义，避免旧断言继续把 local probe 误当成正式恢复升级。
- Decision: transport-await 主链的升级合同必须在“顶层状态机”和“动作在途 coalescing”两端同时一致；只改 ledger 或只改 coordinator 都会留下重复 reset 或错误恢复叙事的漏口。
- Validation: 已通过 `cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`、`cargo test -p xbxengine transport::rtc::policy::video_scheduling_owner -- --nocapture`、`cargo test -q -p xbxengine transport::rtc::session::policy -- --format=terse`。
- Date: 2026-04-06 | Status: completed
- Update: 已定位并修复 `transport::rtc::session::policy` 套件迟迟拿不到退出码的直接根因：测试 `cloud_early_new_without_builder_does_not_emit_liveness_reconnect_candidates` 在持有 `runtime_stats` 锁做断言后，又继续调用 `policy.on_snapshot(...)`，导致同线程重入锁死。
- Decision: 这是测试用例自身的同步错误，不是恢复主链逻辑 hang。通过缩小锁作用域、在再次调用 `policy` 前显式释放 guard 后，整套 session policy 套件恢复为正常退出。
- Validation: 已通过 `cargo test -p xbxengine cloud_early_new_without_builder_does_not_emit_liveness_reconnect_candidates -- --nocapture` 与 `cargo test -q -p xbxengine transport::rtc::session::policy -- --format=terse`。
- Date: 2026-04-07 | Status: completed
- Update: 基于 `runtime-trace-1775532827500-1.jsonl` 的 follow-up，再收口两类“恢复假在飞”漏口：1) `transportAwaitRecoveryAnchor` 的 `packet-seen/deferred/expired-unsent` 不再长期占住 keyframe family，`packet-seen` 若超过 grace 仍无 decode/clean-anchor 且客观媒体不健康，会直接记为 keyframe-stage failure evidence；2) `transportAwait` 链上的 decoder reset 只在短暂 grace 内且无 decode/present/clean-anchor 进展时才视为 in-flight，超窗后允许重新开动作，不再把“曾经 reset 过”长期当成在飞。
- Decision: 顶层状态机必须把“探测性 keyframe family”和“昂贵 reset family”都改成基于真实进展的短时占用，而不是只要见过一次动作就长期吸收；否则轻波动会被误升级，而真卡死又会被假 in-flight 卡住。
- Validation: 已通过 `cargo test -p xbxengine packet_seen_transport_await_episode_upgrades_to_decoder_reset_after_decode_grace_expires -- --nocapture`、`cargo test -p xbxengine deferred_transport_await_episode_does_not_keep_keyframe_family_in_flight -- --nocapture`、`cargo test -p xbxengine stale_transport_await_decoder_reset_without_progress_can_reopen_decoder_reset -- --nocapture`、`cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`。
- Date: 2026-04-07 | Status: completed
- Update: 继续按“从顶层收厚恢复边界”的方向补最后一层防线。`RequestKeyframe` 默认从正式 `active-recovery` 叙事中下沉为 local probe / self-healing；`transportAwaitRecoveryAnchor` 进一步拆成 `probe keyframe / await decode progress / await reset progress` 三段资格边界；媒体域 `Reconnect` 新增 session policy 最终审批门，只有本地 keyframe/decode/reset 恢复窗都耗尽后才允许放行，避免媒体域 staged escalation 自然滚进昂贵 reconnect。
- Decision: 恢复系统的顶层语义必须把“探测动作”和“昂贵恢复动作”明确拆开。小波动、首帧缺 SPS/PPS、单次 keyframe probe 都应优先停留在 local-self-healing；只有当 probe 失败且 decode/reset 都没有带来真实进展时，才允许把 reconnect 作为最后手段交给顶层仲裁。
- Validation: 已通过 `cargo fmt --all`、`cargo test -p xbxengine transport_await_lane_distinguishes_probe_decode_and_reset_progress -- --nocapture`、`cargo test -p xbxengine media_reconnect_candidate_is_blocked_while_transport_await_reset_progress_is_active -- --nocapture`、`cargo test -p xbxengine missed_transport_await_keyframe_episode_upgrades_to_decoder_reset -- --nocapture`、`cargo test -p xbxengine connected_track_attached_without_host_feedback_eventually_escalates_after_priming_window_expires -- --nocapture`、`cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`、`cargo test -q -p xbxengine transport::rtc::session::policy -- --format=terse`。
- Date: 2026-04-07 | Status: completed
- Update: 补齐 diagnostics 语义对齐的最后一段链路。`runtime_summary / primary_issue_chain / latest_decision_summary` 不再继续复用旧的 `display_phase=recovering` 粗粒度口径，而是优先投影 `local-self-healing / recovery-eligible / active-recovery / recovery-blocked` 等统一生命周期；前端 diagnostics 也把状态码拆成 `probing / recovering / blocked`，避免把 local probe 和正式昂贵恢复再次混成单一“恢复中”。
- Decision: 顶层恢复改造如果只改调度，不改摘要与前端展示，最终用户看到的仍是旧叙事，等价于设计没有真正落地。摘要与 UI 必须以 unified lifecycle 为准，display/session 旧字段只保留兼容与回退作用。
- Validation: 已通过 `cargo test -p xbxengine runtime_summary_uses_remote_profile_input_and_owner_state_as_main_view -- --nocapture`、`cargo test -p xbxengine runtime_summary_and_issue_chain_use_local_self_healing_lifecycle_when_present -- --nocapture`、`cargo test -p xbxengine latest_decision_summary_is_driven_by_canonical_owner_contract -- --nocapture`、`cargo test -p xbxengine owner_contract_projection_reads_canonical_runtime_owner_fields -- --nocapture`、`cargo test -p xbxengine build_stats_reports_recovering_after_first_present_when_output_turns_stale -- --nocapture`、`cargo test -p xbxengine build_stats_prioritizes_recent_timeline_recovering_over_healthy_summary -- --nocapture`、`cargo test -p xbxengine build_stats_prioritizes_recent_timeline_broken_over_steady_healthy -- --nocapture`、`npm run build`。
- Date: 2026-04-07 | Status: completed
- Update: 继续补齐 reconnect gate 的可解释性链路。`session/policy` 为 reconnect proposal 增加显式 `reconnectGranted:* / reconnectBlocked:*` detail，并把该 detail 透传到 recovery ledger 的 `gate_result`、`latest_decision_summary` 与前端 diagnostics i18n；同时把 `transport::rtc::session::policy` 中仍残留的旧 `"pass"` 合同统一回归到新语义，保证 connectivity reconnect、local display path 与 repeated severe deadline upgrade 三类场景的断言一致。
- Decision: reconnect 作为昂贵恢复的最后手段，不能再只暴露一个 `pass/suppressed` 粗粒度结果；顶层仲裁必须把“为什么放行”“为什么挡住”直接落到 ledger/summary/UI，否则调度边界虽然收厚了，但运行态仍不可解释。
- Validation: 已通过 `cargo test -p xbxengine latest_decision_summary_surfaces_reconnect_gate_detail_when_present -- --nocapture`、`cargo test -p xbxengine media_reconnect_candidate_is_blocked_while_transport_await_reset_progress_is_active -- --nocapture`、`cargo test -q -p xbxengine transport::rtc::session::policy -- --format=terse`、`npm run build`。
- Date: 2026-04-07 | Status: completed
- Update: 继续吸收 `moonlight-qt` 风格的硬边界控制，但不做大重构。本轮一口气补了 5 点：1) `session/policy` 对“connected 且仍有 ingress、但长期没有 decode/present/clean-anchor 成功输出”增加 failed-terminal 边界；2) 用 `reconnect_grants_without_success_edge` 记录连续硬恢复失败计数，不再只靠时间窗；3) media reconnect 增加 `awaitSuccessEdge` 门，上一轮 reconnect 后若没有新的成功边沿，不允许仅靠 proposal interval 再次放行；4) `control_channel/service` 把 keyframe/decoder-reset replay backlog 显式投影到 runtime stats，并作为 `mediaGate:controlReplayBacklog` 参与 reconnect gate；5) decode actor/state 为本地 decoder reset 增加 success-edge/barrier 屏障，避免重复 reset 风暴。
- Decision: 调度层“收厚恢复边界”不能只靠更多 suppress 条件，还必须补三类更硬的单调边界：失败累计计数、成功边沿解锁、以及 ingress 存在但成功输出长期缺失时的明确终止。否则系统只会更会解释，但不会更会收口。
- Validation: 已通过 `cargo fmt --all`、`cargo test -q -p xbxengine transport::rtc::session::policy -- --format=terse`、`cargo test -q -p xbxengine transport::rtc::connection::control_channel -- --format=terse`、`cargo test -q -p xbxengine duplicate_local_decoder_reset_without_success_edge_is_coalesced -- --nocapture`、`cargo test -q -p xbxengine media_reconnect_candidate_waits_for_success_edge_before_regrant -- --nocapture`、`cargo test -q -p xbxengine media_reconnect_candidate_is_blocked_while_control_replay_backlog_is_active -- --nocapture`、`cargo test -q -p xbxengine connected_ingress_without_success_output_can_enter_failed_terminal_after_reconnect_exhaustion -- --nocapture`。`cargo test -q -p xbxengine media::video::decode::video_decode -- --format=terse` 命中过往已有的长跑 pipeline 用例，未拿最终退出码，本轮以新增定向 decode 用例替代验证。 
- Date: 2026-04-07 | Status: completed
- Update: 基于最新首帧回归继续把 `transportAwaitRecoveryAnchor` 从 reason 特判提升为 coordinator 内部的 phase 合同。`session/policy` 与 `video_scheduling_owner` 已先把首帧前 bootstrap/await-keyframe 统一视作“首帧可用性窗口”；本轮继续在 [`coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 把这条边界落到动作升级层：只要仍处于 `pre-first-frame availability active`，就禁止 `keyframe-stage failure evidence`、`forced transport-await stage upgrade`、`hard fallback evidence` 把 probe 直接抬成 `RequestDecoderReset`/更昂贵动作，避免 `packet-seen but unusable`、`NonIdrVcl`、缺 SPS/PPS、invalid slice header 等首帧不可用事实在 Connected 后被误当成 steady-state reset 证据。
- Decision: 首帧前和运行中的恢复目标必须显式分相。首帧前优先“拿到第一帧可用输出”，允许保留局部 probe / 等待 decode 进展，但不允许 bootstrap 未闭合就直接跨进昂贵恢复；首帧后再恢复既有 `success-edge / clean-anchor / failed-terminal` 等硬边界，继续强调低延迟与失败收口。
- Validation: 已通过 `cargo test -p xbxengine startup_non_idr_transport_await_probe_stays_local_before_first_frame -- --nocapture`、`cargo test -p xbxengine startup_bootstrap_non_idr_stays_priming_and_does_not_emit_recovery_intent -- --nocapture`、`cargo test -p xbxengine pre_first_frame_bootstrap_probes_do_not_enter_active_recovery_before_first_frame_feedback -- --nocapture`、`cargo test -p xbxengine invalid_transport_await_keyframe_response_releases_decoder_reset_inflight -- --nocapture`、`cargo test -p xbxengine transport_await_lane_distinguishes_probe_decode_and_reset_progress -- --nocapture`、`cargo test -p xbxengine stale_transport_await_decoder_reset_without_progress_can_reopen_decoder_reset -- --nocapture`。
- Date: 2026-04-07 | Status: completed
- Update: 继续把“首帧前优先可用性”的 bootstrap 主链收回到 `video_source/source.rs`，不让它重新滑回 recovery coordinator。针对 `answer` 缺 `sprop` 且 `Connected + startup/handshaking/priming` 的 H264 首帧场景，bootstrap request 已从旧的一次性 startup probe 收口成 `initial + follow-up` 两段：第一次仍在首个 assembled sample 前触发；若后续已经观察到 `audioOnly/音频先行/视频轨已有媒体活动`，但 inspection 仍因 `bootstrapMissingSps/Pps`、`NonIdrVcl` 或 invalid slice header 被拒绝，则仅补发一次 follow-up keyframe request。只要已有 decode/present/submit 成功边沿，整条 bootstrap 链立即关闭，不再继续请求。
- Update: 基于 trace [`runtime-trace-1775552546835-1.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775552546835-1.jsonl) 继续收口首帧成功边界。倒查确认 source 层此前仍把 `committed SPS/PPS` 当成 startup bootstrap 链的硬终止条件，导致“参数集已 commit，但仍无 first decode/present/submit”时 follow-up bootstrap request 被提前掐断。现已在 [`source.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs) 移除这条提前熔断，把 bootstrap 链的真正关闭条件统一收回到 `latest_video_decode_ok_time_ms / latest_video_host_present_time_ms / video_present_submit_count_total / video_present_epoch` 这些首帧成功边沿；[`source.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.test.rs) 新增“已 commit 参数集但仍未首帧成功时，follow-up 仍可发”的回归，锁定 pre-first-frame availability 与 source 主链语义一致。
- Decision: startup bootstrap 的主动作必须留在 source 层，因为这里只有原始 AU 证据、answer sprop 缺失事实和“尚未首帧成功”的上下文三者同场；`session/policy`、`video_scheduling_owner`、`coordinator` 继续只负责“首帧前不升级昂贵恢复”的边界，不承接 bootstrap 主动作。
- Validation: 已通过 `cargo fmt --all`、`cargo test -p xbxengine startup_h264_without_sprop_requests_keyframe_before_first_bad_frame_recovery -- --nocapture`、`cargo test -p xbxengine startup_h264_without_sprop_and_audio_only_emits_single_followup_bootstrap_request -- --nocapture`、`cargo test -p xbxengine startup_bootstrap_followup_request_is_disabled_after_first_frame_feedback -- --nocapture`。
