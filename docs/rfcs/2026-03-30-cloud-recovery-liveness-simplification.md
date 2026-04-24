# Cloud Recovery Liveness Simplification RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: in-progress
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- Cloud 场景近期多份 runtime trace 反复出现同类冻结：恢复链路能触发一次 reconnect，但重连后长时间停在 `Connecting + Priming/SeekingAnchor`，且后续无新的恢复动作，最终画面卡死。
- 代表性日志：
  - [`runtime-trace-1774854083591.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1774854083591.jsonl)
  - [`runtime-trace-1774857742839.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1774857742839.jsonl)
  - [`runtime-trace-1774850865543.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1774850865543.jsonl)
- 当前策略问题不是单点 bug，而是结构性问题：局部抑制策略（去抖、cooldown、预算）叠加后缺少“全局活性保证”。

## Goal

- 建立一套“活性优先、结构最简”的恢复策略基线，确保 cloud 抖动网络下不再进入长期无动作静默区。
- 明确单一恢复状态机、单一升级链、单一预算语义与可审计决策账本。
- 将后续实现从“补丁驱动”切换到“契约驱动 + 验收矩阵驱动”。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/session/policy.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/*`
  - `crates/xbxengine/core/src/transport/rtc/policy/*`
  - `crates/xbxengine/core/src/transport/rtc/projection/*`
  - `src-tauri/src/mods/xbxengine/trace_projection.rs`（决策账本投影）
  - `src/streaming/runtime/xbxengine-runtime.ts`（前端观测映射）
- Out of scope:
  - ICE/STUN/TURN 协议栈替换
  - 编解码器能力优化
  - Native video 渲染路径重写

## Plan

1. 定义顶层恢复契约（不改代码）
2. 按契约重构调度链路（状态机/升级链/预算/观测）
3. 基于日志回放与网络矩阵做验收，收敛并发布

## Validation

- [ ] 在 cloud 回放场景中，不再出现“`transportState=Connecting` 长时间停滞且 `videoEscalation=0`”静默窗口
- [ ] 恢复链路满足“无进展上界”约束：超过阈值必有升级动作或终止判决
- [ ] 决策账本可解释每次 `suppressed` / `deferred` / `failed` 的原因与预算变化

## Risks

- 活性优先策略若阈值过激，可能引入重连风暴与稳定性回退。
- 统一状态机需要跨模块收口，短期会增加重构复杂度与回归压力。

## Strategy Contract

### 1) 单一状态机

- 统一恢复状态：`Detecting -> Recovering -> Reconnecting -> Recovered | FailedTerminal`
- 禁止各层自定义平行“半状态”作为最终裁决（如仅凭 owner 层 `priming/seeking-anchor` 阻断动作）。

### 2) 单一活性不变式（Liveness Invariant）

- 若进入 `Recovering/Reconnecting` 后连续 `T_no_progress` 未观察到恢复进展（例如 transport 回到 `Connected` 且视频链回稳），必须执行以下其一：
  - 升级到更强恢复动作；
  - 进入 `FailedTerminal` 并上报明确原因。
- 禁止无限期 `CooldownSuppressed` 或“无命令返回”。

### 3) 单一升级链

- 固定升级序：`RequestKeyframe -> RequestDecoderReset -> RequestReconnectCandidate -> FailedTerminal`
- 每级有硬超时与最大重试次数，不允许回退到更弱动作。

### 4) 单一预算语义

- 预算仅用于限制“同级重复次数”，不能阻断升级链前进。
- reconnect 预算按 recovery epoch 管理，但必须与“已执行结果”一致：
  - `Succeeded` 扣减执行预算；
  - `Deferred` 不计失败，不消耗失败预算；
  - `Failed` 进入下一层升级或终止判决。

### 5) 决策账本（Decision Ledger）

- 每个调度 tick 都输出结构化记录：
  - `decision_id`
  - `state_before/state_after`
  - `input_signals`
  - `gate_results`（通过/抑制及原因）
  - `action_selected`
  - `budget_before/after`
  - `command_result`（succeeded/deferred/failed）
- 目标：任何一次冻结都能在账本中回答“为何此刻无动作”。

## Acceptance Matrix

- 网络维度：
  - RTT: `40/120/220/350ms`
  - 抖动: `low/high`
  - 丢包: `0/2/5/10%`
  - 间歇断流: `none/intermittent`
- 会话维度：`startup/steady/recovering/reconnecting`
- 信号维度：`waitKeyframe/transportAwaitRecoveryKeyframe/expiredDeadline/idleTimeout/thinStream/lifecycleRecovering`
- 执行结果维度：`succeeded/deferred/failed`
- 核心断言：
  - 无动作静默窗口上界
  - 单次重连失败后的再次升级可达性
  - epoch 与预算一致性
  - recover 成功后稳定收口时间上界

## Progress

- [x] Step 1: 顶层契约与验收矩阵定义完成
- [ ] Step 2: 代码按契约重构（状态机/升级链/预算/观测）
  - [x] Phase 1: 决策账本 + 静默窗口上界首轮落地
  - [x] Phase 2(Part A): 关闭 rust-owned runtime 的 legacy 并行恢复链，恢复动作 owner 统一回到 transport session policy 主链
  - [x] Phase 2(Part B): owner/coordinator/planner 状态标签与终止态语义完全收口（`FailedTerminal` + 连续无进展重连尝试上限）
- [ ] Step 3: 网络矩阵回放验收与参数收敛

## Execution Notes

- Date: 2026-03-30 | Status: in-progress
- Update: 基于最新 cloud 冻结日志完成顶层策略收口，明确当前问题是“活性缺失”而非单点门槛 bug。
- Decision: 暂停补丁式修复，后续改造以本 RFC 契约为唯一实现依据。
- Risk/Blocker: 当前仍缺统一回放驱动用于批量验证 `T_no_progress` 上界和升级可达性。
- Date: 2026-03-30 | Status: in-progress
- Update: Phase 1 已落地 recovery decision ledger（含 budget before/after 与 command result 回填）并打通 trace 投影；同时把 rust-owned runtime 的 `drive_runtime_recovery_action` 降为兼容路径，避免与 transport session policy 并行发恢复动作。
- Validation: `cargo test -p xbxengine transport::rtc::session::policy::tests:: -- --nocapture`、`cargo test -p xbxengine transport::rtc::stack::transport_session::tests:: -- --nocapture`、`cargo test -p xbxengine api::runtime::tests::runtime_ -- --nocapture`、`cargo test -p xbxrc trace_projection -- --nocapture --test-threads=1`。
- Date: 2026-03-30 | Status: in-progress
- Update: 收口 `Recovering/Reconnecting` 的终止态语义：当无进展条件下 lifecycle reconnect 连续尝试达到上限，状态机进入 `FailedTerminal`，并在 decision ledger 中输出终止原因（`livenessReconnectAttemptLimitExceeded`），后续 tick 不再继续发恢复命令，直到观察到明确进展再解锁。
- Validation: `cargo test -p xbxengine transport::rtc::session::policy::tests::lifecycle_reconnect_attempt_limit_enters_failed_terminal -- --nocapture`、`cargo test -p xbxengine transport::rtc::session::policy::tests:: -- --nocapture`。
- Date: 2026-03-30 | Status: in-progress
- Update: 补齐 Phase 3 验收矩阵第一批自动化用例：1) `FailedTerminal` 在观察到成功进展后可解锁并重新进入恢复主链；2) `Connecting/Recovering` 两类恢复面都受统一 no-progress 上界约束，超时可达 reconnect。
- Validation: `cargo test -p xbxengine transport::rtc::session::policy::tests::failed_terminal_clears_after_successful_progress_and_rearms_reconnect -- --nocapture`、`cargo test -p xbxengine transport::rtc::session::policy::tests::no_progress_upper_bound_applies_to_connecting_and_recovering_surfaces -- --nocapture`、`cargo test -p xbxengine transport::rtc::session::policy::tests:: -- --nocapture`。
- Date: 2026-03-30 | Status: in-progress
- Update: Phase 3 增加日志自动验收脚本能力（`summarize_runtime_trace.py`）：新增 recovery 专项检查（`Connecting` 窗口 ledger 静默上界、`FailedTerminal` 原因统计、终止后解锁证据）并支持 `--json` 机器可读输出，便于后续回放矩阵自动判定。
- Validation: 对最新批次日志 `runtime-trace-1774862086350~6389.jsonl` 逐个回放，脚本可稳定输出 JSON；结果显示该批日志均未包含 `recoveryDecisionLedger/transportObservation` 关键事件，当前无法在日志层证明调度链是否进入 `FailedTerminal` 或发生静默超界，属于观测证据缺失而非“已通过”。
- Date: 2026-03-30 | Status: in-progress
- Update: 针对 `runtime-trace-1774871210764.jsonl` 的“画面完全不出”回归，确认主因是 liveness no-progress 计时被“恢复命令成功（reconnect consumed）”错误重置：会话长期 `Connecting + noPendingFrame`、`inboundPrimaryBytesTotal=0`，但 no-progress 预算未收敛到终止态。已在 session policy 将 no-progress 进展判据从 `successful_action_count` 改为 `media.frame_count`（仅真实媒体前进才重置），避免命令成功掩盖“无帧”卡死面。
- Validation: `cargo test -p xbxengine transport::rtc::session::policy::tests::command_success_without_frames_does_not_reset_liveness_budget -- --nocapture`、`cargo test -p xbxengine transport::rtc::session::policy::tests:: -- --nocapture`。
- Date: 2026-03-30 | Status: in-progress
- Update: 针对 `runtime-trace-1774872358804.jsonl` 的“cloud 新建连不上”回归，确认启动期出现反复 ICE 重启/交换超时循环（`exchange loop finished` 多轮、`local_ice_gathering_complete=false`、`remote_summary=0` 反复），且 `recovery_reconnect_count` 持续上升。根因是首帧前阶段把 no-progress 判据收紧为纯 `frame_count` 后，容易在 transport 已有进展但尚未出首帧时过早触发重连。已新增“首帧前 transport 进展放宽窗口”：当 `frame_count==0` 且连接已有 RTT/路径/数据通道进展时，liveness 重连上界由 4s 放宽到 15s，避免把建连过程打断。
- Validation: `cargo test -p xbxengine transport::rtc::session::policy::tests::pre_first_frame_transport_progress_uses_relaxed_liveness_timeout -- --nocapture`、`cargo test -p xbxengine transport::rtc::session::policy::tests:: -- --nocapture`。
- Date: 2026-03-30 | Status: in-progress
- Update: 针对 `runtime-trace-1774872845088.jsonl` 的“仍失败”回归，确认主要失败点已从 no-progress 误判转移到 ICE 交换循环收口条件：远端已返回候选并发送 EOC（`remote_summary=host=2,eoc=1`），但因 `local_ice_gathering_complete=false` 被硬卡在循环内直至超时，随后进入 reconnect 周期性重启。已将 `exchange_remote_ice_incrementally` 改为“稳定收口 + 超时兜底”双门槛：当已提交本地候选、远端 EOC 已到达、且本地候选在稳定窗口内无新增时即可收口退出，不再把 `local_ice_gathering_complete` 作为唯一退出前置。
- Validation: `cargo test -p xbxengine api::runtime::lifecycle::tests:: -- --nocapture`、`cargo test -p xbxengine transport::rtc::session::policy::tests:: -- --nocapture`。
- Date: 2026-03-30 | Status: in-progress
- Update: 按“调度时钟与观测账本解耦”完成两项结构修复：1) session policy 内 owner/liveness/failed-terminal/ledger 的 `observed_at_ms` 统一改为 `max(snapshot.now_ms, recovery.last_observed_at_ms)`，避免 `last_observed_at_ms` 在 `Connecting/Priming/SeekingAnchor` 卡住时阻断 no-progress 与节流计时；2) 移除 `record_recovery_decision_ledger` 的 early-return 抑制，改为每个 tick 都写入 ledger（`proposal none` 也写 `input_signal=none/gate_result=no-signal/action_selected=none`），消除恢复期观测空窗。
- Validation: `cargo test -p xbxengine transport::rtc::session::policy::tests::liveness_uses_snapshot_now_when_last_observed_stalls -- --nocapture`、`cargo test -p xbxengine transport::rtc::session::policy::tests::recovery_decision_ledger_still_updates_when_proposal_is_none -- --nocapture`、`cargo test -p xbxengine transport::rtc::session::policy::tests:: -- --nocapture`。
- Date: 2026-03-30 | Status: in-progress
- Update: 基于 `runtime-trace-1774874449561.jsonl` 继续收敛“首帧前过早重连打断”问题：日志显示首帧前存在 `Reconnecting + exchange timeout` 循环，且重连重入节奏过快。已在 session policy 引入双门槛收口：1) `Recovering` 的周期性 reconnect 仅在“首帧后（`frame_count>0`）”允许；2) `no-progress` 在 `Recovering + 首帧前` 使用 15s 放宽上界（与已有“首帧前 transport 有进展放宽”并列），避免 4s 短窗反复打断握手。
- Validation: 新增回归 `recovering_pre_first_frame_without_transport_progress_uses_relaxed_liveness_timeout`、`recovering_without_first_frame_does_not_emit_periodic_reconnect`，并通过 `cargo test -p xbxengine transport::rtc::session::policy::tests:: -- --nocapture`（35/35）。
- Date: 2026-03-30 | Status: in-progress
- Update: 基于 `runtime-trace-1774875558909.jsonl` 发现新的调度面错误：在 `transport_state=New` 阶段仍触发 `livenessNoProgressTimeout` 并持续下发 reconnect 候选，导致 pending action 长期占位、后续建连阶段出现“旧 pending 消费触发重连”干扰。已收紧 liveness 生效面：仅允许在 `Connecting/Recovering`（以及 `Connected + 非 StableServing`）评估 no-progress；`New`/`Closed` 等非恢复面不再触发 reconnect。另补守护测试覆盖 `New` 态长期无进展不发 reconnect。
- Validation: `cargo test -p xbxengine transport::rtc::session::policy::tests::new_state_does_not_emit_liveness_reconnect_before_connecting -- --nocapture`、`cargo test -p xbxengine transport::rtc::session::policy::tests:: -- --nocapture`（36/36）。
- Date: 2026-03-30 | Status: in-progress
- Update: 基于 `runtime-trace-1774879931627.jsonl` 的最新回归，确认“已连接判定误收口”路径已消失，但出现新的结构性问题：`exchange_remote_ice_incrementally` 在 `candidate exchange settled` 分支（`remote_eoc_seen + idle>=1200ms`）下提前退出，随后会话仍长期停留 `transport_state=Connecting` 且 `inbound_video_bytes_total/present_fps` 持续为 0，最终进入 `livenessNoProgressTimeout -> reconnect` 循环。已按“大道至简”回退该提前收口分支，ICE 交换只保留三类退出条件：`transport connected`、`gathering complete + remote eoc`、`timeout`。
- Validation: `cargo test -p xbxengine api::runtime::lifecycle::tests:: -- --nocapture`。
- Date: 2026-03-31 | Status: in-progress
- Update: 根据最新回归日志假设“exchange timeout 过短误杀”，新增临时 A/B 实验配置：`resolve_ice_exchange_timeout_ms` 约束到 `[10s, 12s]`（最低 10s，最高 12s），用于与当前 5~8s 档位做连通率对比。
- Validation: `cargo test -p xbxengine api::runtime::lifecycle::tests:: -- --nocapture`。
- Date: 2026-03-31 | Status: in-progress
- Update: 针对“IPv4/IPv6 仅应影响顺序，不应破坏候选集合”问题收口连接层 family 处理：取消 `should_skip_remote_candidate_for_family_mismatch` 对 `host` 候选的硬过滤，改为仅保留 mismatch 观测日志，不再因 family 不匹配直接丢弃远端 host 候选。
- Validation: `cargo test -p xbxengine transport::rtc::connection::candidate_helpers -- --nocapture`、`cargo test -p xbxengine transport::rtc::connection::negotiation::tests:: -- --nocapture`。
- Date: 2026-03-31 | Status: in-progress
- Update: 基于 `runtime-trace-1774925510583.jsonl` 回放修正 IPv4/IPv6 对应策略：1) 撤销“host family mismatch 永不跳过”的临时排障改动，恢复为“仅 host 做 family 对应约束，srflx/relay 继续放行”；2) `RtcIoRuntime` 从单 `advertised_ip` 升级为有序 `advertised_ips`，本地 host 候选改为按 family 同时采集（若本机 v4/v6 都可用则双栈都上报），避免只上报 IPv4 导致远端 IPv6 host 候选无法对应；3) 本地地址映射与 relay related address 改为按 family 选择，避免 4/6 地址与端口混用；4) 补充双栈端点与 relay family 选择单测。
- Validation: `cargo test -p xbxengine transport::rtc::connection::candidate_helpers -- --nocapture`、`cargo test -p xbxengine transport::rtc::connection::negotiation::tests:: -- --nocapture`、`cargo test -p xbxengine api::runtime::lifecycle::tests:: -- --nocapture`。
- Date: 2026-03-31 | Status: in-progress
- Update: 收敛 `No route` 对调度链路的破坏面：`RtcIoRuntime::pump` 中将 `AddrNotAvailable/NetworkUnreachable/HostUnreachable`（含常见 OS errno 49/51/64/65）降级为 non-fatal send drop，仅记录警告并继续 ICE 驱动，避免“单 candidate 不可达”升级为整条连接流程失败。
- Validation: `cargo test -p xbxengine transport::rtc::connection::io_runtime::tests:: -- --nocapture`、`cargo test -p xbxengine transport::rtc::connection::negotiation::tests:: -- --nocapture`。
- Date: 2026-03-31 | Status: in-progress
- Update: 按“三原则”补齐 `prefer_ipv6` 贯通：将 `plan.negotiation.prefer_ipv6` 从 `xbox-streaming` runtime projection 透传到 tauri `StreamingRuntimeProjection`、`XbxEngineRuntimeProjectionDto` 与 `XbxEngineNegotiationRuntimeConfig`，并在 `RtcConnectionService::sync_runtime_config` 下发到 `RtcIoRuntime`；`RtcIoRuntime` 调整为“只改排序不改候选集合”，双栈 host 候选仍完整上报，`prefer_ipv6` 仅影响 host/same-type 内 IPv6 与 IPv4 的顺序。
- Validation: `cargo test -p xbxengine api::runtime::tests::start_runtime_control_consumes_execution_spec -- --nocapture`、`cargo test -p xbxengine transport::rtc::connection::io_runtime::tests:: -- --nocapture`、`cargo test -p xbxengine transport::rtc::sdp::policy::tests:: -- --nocapture`、`cargo check -p xbox-streaming`、`cargo check -p xbxrc`。
- Date: 2026-03-31 | Status: in-progress
- Update: 将 `non-fatal send drop` 升级为“窗口化聚合判定”：单次 `No route/HostUnreachable` 不再立刻失败；仅当持续窗口（>=3s）内累计 non-fatal drop 达阈值（>=6）且无任何网络进展（发送成功/收包）时，才上报 `xbxEngineRtcAllCandidatePathsUnreachable`，错误中带 `peers/dropCount/elapsedMs` 便于诊断“所有已尝试候选都不可达”。
- Validation: `cargo test -p xbxengine transport::rtc::connection::io_runtime::tests:: -- --nocapture`、`cargo test -p xbxengine transport::rtc::connection::negotiation::tests:: -- --nocapture`、`cargo test -p xbxengine api::runtime::lifecycle::tests:: -- --nocapture`。
- Date: 2026-03-31 | Status: in-progress
- Update: 收敛 `exchange_remote_ice_incrementally` 的 ICE 收口语义，消除“仅靠 timeout 收口”循环：新增稳定收口判定（`submitted local + remote EOC seen + local candidates stable >= 1.5s`）并支持在稳定窗口内受控补发一次本地 `a=end-of-candidates`；同时保留既有 timeout 硬兜底。该路径不再把 `local_ice_gathering_complete=true` 作为唯一前置，避免手动候选模式下长期 `local_gathering_complete=false` 导致交换循环反复超时。
- Validation: `cargo test -p xbxengine api::runtime::lifecycle::tests:: -- --nocapture`、`cargo test -p xbxengine api::runtime::tests::start_runtime_control_consumes_execution_spec -- --nocapture`、`cargo test -p xbxengine api::runtime::tests:: -- --nocapture`。
- Date: 2026-03-31 | Status: in-progress
- Update: 基于 `runtime-trace-1774933478849.jsonl` 的新失败面继续收口 liveness 策略：1) `should_force_liveness_reconnect` 改为“首帧前统一 15s 保守上界”，不再对“无 transport 里程碑”走 4s 快速误杀；2) `failed-terminal` 解锁条件从 `successful_action_count` 改为 `media.frame_count` 真实前进，避免“命令成功但无首帧”把终态误解锁并触发 reconnect 风暴；3) 同步更新 session policy 回归用例，覆盖 connecting/startup/seeking-anchor 首帧前阈值、command-success 无帧不重置预算、failed-terminal 仅在真实媒体进展后解锁。
- Validation: `cargo test -p xbxengine transport::rtc::session::policy::tests:: -- --nocapture`、`cargo test -p xbxengine api::runtime::tests:: -- --nocapture`、`cargo fmt`。
- Date: 2026-03-31 | Status: in-progress
- Update: 基于 `runtime-trace-1774948984370.jsonl` 的“最终卡死”回归，继续按单一恢复主链收口 reconnect 执行语义：1) `pending reconnect candidate` 从“先到先占坑”改为“可更新（新 observation/reason 覆盖旧 pending）”；2) runtime 增加 `single-flight reconnect`（`state=Reconnecting` 不消费 pending）；3) 增加 6s 最小重连间隔，窗口内仅标记 `transportReconnectCandidateDeferred:cooldown` 且保留 pending，窗口后再消费，避免自我打断形成重连风暴。
- Validation: `cargo test -p xbxengine transport::rtc::executor::peer::tests:: -- --nocapture`、`cargo test -p xbxengine transport::rtc::stack::transport_session::tests:: -- --nocapture`、`cargo test -p xbxengine api::runtime::tests::runtime_defers_pending_transport_reconnect_candidate_while_reconnecting -- --nocapture`、`cargo test -p xbxengine api::runtime::tests::runtime_applies_transport_reconnect_candidate_cooldown_and_retries_after_window -- --nocapture`、`cargo test -p xbxengine api::runtime::tests::runtime_consumes_pending_transport_reconnect_candidate_once -- --nocapture`。
- Date: 2026-03-31 | Status: in-progress
- Update: 基于 `runtime-trace-1774951036816.jsonl` 的“出画后立刻关闭”回归，确认主因是云端通过 message channel 下发 `KickForClosedGame` 后，runtime 仍沿恢复链触发 reconnect keepalive，随后被 `SessionNotActive(410)` 终止。已增加终态优先处理：`data_channel` 识别 `KickForClosedGame` 后写入终态观测，`runtime lifecycle` 在 tick 开头直接 stop 并上报 `recoverTransportSessionKickedForClosedGame`，不再进入 reconnect 分支。
- Validation: `cargo test -p xbxengine api::runtime::tests::runtime_stops_when_session_is_kicked_for_closed_game -- --nocapture`、`cargo test -p xbxengine api::runtime::tests::runtime_stops_reconnect_loop_when_keepalive_reports_session_not_active -- --nocapture`。

### Phase Plan

#### Phase 1 - Decision Ledger + Liveness Upper Bound

- 目标：先补“可解释性 + 静默上界”基础设施，不改协议栈，不调网络参数。
- 交付：
  - recovery 决策账本最小落地：至少覆盖 `decision_id/state_before/state_after/signal/action/budget_before/budget_after/command_result`。
  - trace 输出新增 recovery ledger 决策事件，确保日志能直接回答“本 tick 为何无动作”。
  - session policy 新增统一 `no-progress` 计时与硬兜底上界，替代分散条件门槛。
- 验收断言：
  - 连续 `T_no_progress` 超时后必须出现明确升级决策或终止判决；
  - 不再出现“`transportState=Connecting` 且长期没有 recovery 决策事件”的黑洞窗口。

#### Phase 2 - Single Arbitrator State Machine

- 目标：把恢复裁决点收口为单一状态机，去掉跨层平行半状态主导动作的路径。
- 交付：
  - `session/policy + recovery/coordinator + owner intent` 形成唯一裁决入口；
  - lifecycle/recovery/fallback 不再各自独立触发同类 reconnect。
- 验收断言：
  - 同一 tick 只有一个恢复裁决 owner；
  - 账本中状态跃迁可追溯，不存在互相覆盖的双轨动作。

#### Phase 3 - Escalation Chain + Budget Semantics Unification

- 目标：统一升级链和预算语义，完成网络矩阵回放验收。
- 交付：
  - 固化 `RequestKeyframe -> RequestDecoderReset -> RequestReconnectCandidate -> FailedTerminal`；
  - 预算仅限制同级重复，不阻断升级链推进；
  - 网络矩阵回放（RTT/抖动/丢包/间歇断流）验收脚本与结果归档。
- 验收断言：
  - 预算与执行结果严格一致（`succeeded/deferred/failed` 可审计）；
  - recover 成功后稳定收口时间受上界约束。
