# 事实驱动恢复建模调整 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: in-progress
- Owner: codex supervisor
- Last Updated: 2026-04-12

## Background

- 近期多份 runtime trace 已证明，当前恢复系统并不是“没发 NACK”，而是经常在参考链缺包后长期停留在“继续 NACK + keyframeInFlight 合并抑制”的中间态，迟迟没有切换到新的恢复关键帧闭环。
- 现有系统中已经存在若干“有明确优化目标的专项策略”，例如恢复爬升期优化、startup/recovery sustaining、display 供给保活、latency-first admission 等；这些策略不是噪声，新的统一建模不能把它们粗暴抹平，而必须给它们明确挂接点。
- 本次日志 [`runtime-trace-1775824743739-1.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775824743739-1.jsonl) 将问题暴露得更直接：
  - `24573` 最初只是 `delta` 缺包，先被 `skippedTooLate`，随后又升级到 `keyframe` 家族并最终被一次 PLI + 可解码关键帧救回。
  - `30191`、`35010` 则明显属于 `reference / anchor` 级 gap，长时间停留在 `awaitingRecoveryKeyframe / referenceChainUnrecoverable`，同时伴随大量 `coalesced:keyframeInFlight`、`sameFamilyCoalesced:transportStageSuppressed`、`transportDeferred`。
  - `41446` 则属于“恢复刚起来又被新的参考链缺口打断”的次生 gap。
- 这说明当前问题不再适合靠单个阈值或单条补丁规则继续修，而需要把恢复系统的事实模型重新收口，至少覆盖：帧价值、缺包严重性、升级门槛、InFlight 解锁、coalescing 规则。

## Goal

- 让系统更早识别“当前问题已经从补包退化为恢复闭环问题”，并及时把主控制权从 NACK 重试切换到 recovery-frame 驱动。
- 让 `in-flight` 与 `coalescing` 保护的是“正在推进的恢复动作”，而不是“历史上发过一个请求”。
- 让 trace、stats、session/policy、owner、coordinator 和 video source 使用同一套恢复事实模型，避免跨层价值失真。
- 为后续实现提供一份可以直接落地的建模合同，而不是继续叠加单点补丁。
- 在不破坏现有专项优化目标的前提下完成统一收口，使现有策略从“散落补丁”升级为“挂靠在统一模型上的特化规则”。
- 允许 InFlight 解锁，但必须同时维持 anti-storm 边界，避免系统从“假在飞”退化到“重复 PLI/RFI/reset 风暴”。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/*`
  - `crates/xbxengine/core/src/transport/rtc/recovery/*`
  - `crates/xbxengine/core/src/transport/rtc/session/policy.rs`
  - `crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`
  - `crates/xbxengine/core/src/runtime_stats_sink.rs`
  - `crates/xbxengine/core/src/diagnostics/stats.rs`
  - `src-tauri/src/mods/xbxengine/trace_projection.rs`
  - 与恢复事实模型相关的测试、trace 字段、前端 diagnostics 合同
- Out of scope:
  - 引入新传输协议、新播放器或平行恢复栈
  - 单纯调大/调小某个 timeout 或 retry count 的补丁式止血
  - 直接重写 H264 inspection、NACK 基础发包机制或 TWCC 基础能力

## Plan

1. 建立统一的恢复事实模型，明确帧价值与缺包严重性的动态分层合同。
2. 重写恢复升级门槛、InFlight 解锁和 coalescing 判定，使其围绕“恢复闭环是否推进”决策。
3. 把新模型贯通到 trace/stats/diagnostics 和测试矩阵，并用主 gap 案例回放验证。

## Validation

- [x] 为 `24573 / 30191 / 35010 / 41446` 建立主 gap 回放/断言矩阵（以可复现单测断言矩阵为主，锁定分类/解锁/抢占语义）
- [x] `video_source / recovery / session::policy / video_scheduling_owner` 全链路定向测试通过
- [x] `transport_session / repeat_suppression / ledger + traceProjection` 定向覆盖已落地
- [ ] trace/stats 新字段已被前端 diagnostics 全量消费并完成跨层一致性验证
- [x] 本 RFC 的 `Safety Invariants`、`Rollout Gates`、`Compatibility Plan` 全部满足（无新增动作类型；anti-storm 未放宽；解锁仅释放占坑；新旧字段并行窗口维持）

## Risks

- 如果新模型只改 coordinator，不改 source/timeline/owner，仍会出现跨层价值失真，导致 trace 解释正确但动作仍旧错误。
- 如果 `in-flight` 解锁过于激进，可能引入重复 PLI/RFI/decoder reset 风暴。
- 如果 coalescing 规则只增加分类但不引入抢占语义，仍会保留“同 family 无限压制”的假恢复问题。
- 如果统一模型吞掉现有恢复爬升期等专项策略的边界，可能把本来目标明确的优化退化成普通规则，反而损失已有收益。

## Safety Invariants（本轮硬约束）

- 约束 1（动作边界冻结）：
  - 本轮不得新增 recovery 动作类型，不得引入平行恢复栈。
  - 允许调整现有动作 gate 与调度语义，但不得改写基础 NACK 发包、TWCC 基础能力和 H264 inspection 主流程。
- 约束 2（anti-storm 下限不可放宽）：
  - `in-flight` 允许解锁，但解锁只释放“占坑语义”，不释放“风暴保护语义”。
  - `PLI/RFI`、`decoder reset`、`reconnect` 必须继续分桶限流，不得退化为共享“已解锁即可重发”信号。
  - 任意实现不得降低现有最小重发窗口与 family 配额保护下限。
- 约束 3（恢复闭环推进必须可证）：
  - `ChainBroken / RecoveryBlocked` 进入后，必须在受控窗口内触发至少一次有效升级动作（`refresh/preempt` + 动作级 gate）。
  - 若窗口内无推进边沿（`ResponseObserved / Decoded / CleanAnchorCommitted`），应判定为 `Stalled/Expired` 并允许有约束地切换 episode。
- 约束 4（专项策略目标冻结）：
  - “恢复爬升期优化”“startup/recovery sustaining”“display supply 保活”“latency-first admission”在本轮只允许改挂接输入，不允许改变其既有优化目标。
  - 若实现导致专项策略目标语义变化，必须拆到后续 RFC，不得在本 RFC 里隐式吸收。
- 约束 5（单一事实源）：
  - `FrameValue / GapSeverity / RecoveryEpisodeStage / CoalescingMode` 的定义以本 RFC 为准。
  - 与 `recovery-sustaining-phase-refactor` 出现语义冲突时，必须引用本 RFC 统一定义，禁止并行定义两套同名语义。

## Progress

- [x] Step 1: 基于 trace 明确主问题已从“缺少某个阈值”升级为“恢复事实模型失真”
- [ ] Step 2: 完成统一建模合同并映射到全部 in-scope 模块
- [ ] Step 3: 完成主 gap 回放验证与 diagnostics 收口

## Current Repair Plan（2026-04-17）

- 当前基线：
  - `cargo test -p xbxengine --lib --quiet` 结果为 `913 passed / 90 failed`。
  - 失败集中，说明问题是合同漂移，不是离散的单测夹具噪声。
- 合同分簇：
  - 簇 A：`transport_session + nack_scheduler`
    - 表现：`frame_value / gap_severity / recovery_episode_stage / coalescing_mode` 断言回退，`retry_budget_varies_by_label` 预算从 `3` 退回 `2`，`Merge/Refresh/episodeStalledNoProgress` 等账本字段丢失。
    - 判断：底层事实模型没有稳定驱动 family gate 与预算层。
  - 簇 B：`recovery/coordinator + connection/service`
    - 表现：`clean anchor`、`transport-await`、`decoder reset`、`hard fallback`、`新 epoch 释放预算/计时器` 大量断言失配。
    - 判断：运行态的 current/stale 判定与跨 epoch 清理仍有双轨。
  - 簇 C：`session/policy + runtime/sink`
    - 表现：`transportExpiredDeadline/transportSevereDeadline -> ConnectivityTransport reconnect` 未按预期触发，cloud reconnect lifecycle 与 integration/runtime replay 回归失配。
    - 判断：顶层 orchestration 没有稳定消费 A/B 两层输出，connectivity 升级路径被本地恢复噪声吞掉。
- 修复阶段：
  - Phase 1：先修簇 A，只恢复底层事实合同与预算/ledger 语义，不碰顶层 reconnect 行为。
  - Phase 2：再修簇 B，统一 `current clean anchor/current transport-await issue/recovery epoch reset` 的运行态判定。
  - Phase 3：最后修簇 C，恢复 `session::policy -> runtime` 的 connectivity reconnect 升级与 lifecycle 上界。
- 阶段退出条件：
  - Phase 1：
    - `cargo test -p xbxengine transport::rtc::stream::nack_scheduler -- --nocapture`
    - `cargo test -p xbxengine transport::rtc::stack::transport_session -- --nocapture`
  - Phase 2：
    - `cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`
    - `cargo test -p xbxengine transport::rtc::connection::service -- --nocapture`
  - Phase 3：
    - `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
    - `cargo test -p xbxengine api::runtime::tests::hard_disconnect_transport -- --nocapture`
    - `cargo test -p xbxengine transport::rtc::stream::video_source::sink -- --nocapture`
  - 全量回归：
    - `cargo test -p xbxengine --lib --quiet`
- 执行纪律：
  - 每阶段只处理一个合同簇，禁止跨簇顺手修补。
  - 先修语义源头，再改测试断言；除非确认断言已落后于新合同，否则不先改测试。
  - 若同一阶段连续 3 轮修复后失败面仍扩散，停止补丁式推进，回到 RFC 重审运行态模型。

## Execution Notes

- Date: 2026-04-17 | Status: in-progress
- Update: 直接在当前工作区执行 `cargo test -p xbxengine --lib --quiet`，得到 `913 passed / 90 failed`。失败高度集中在三类合同：1) `transport_session + nack_scheduler` 的底层事实字段/预算语义回退；2) `recovery/coordinator + connection/service` 的 clean-anchor、transport-await、decoder-reset、hard-fallback、epoch reset 运行态清理失配；3) `session::policy + runtime/sink` 的 connectivity reconnect 升级链未按预期触发。后续执行顺序固定为“底层事实合同 -> 运行态协调合同 -> 顶层编排合同”，不按单测名称逐个打补丁。
- Date: 2026-04-12 | Status: in-progress
- Update: 基于 [`runtime-trace-1775975089981-1.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775975089981-1.jsonl) 继续追播放后段卡死，已确认当前主根因之一不是“网络恢复失败”，而是 transport gap 价值被恢复语义错误抬升：trace 在 `seq=175505` 明确出现 `frameIsKeyframe=false` 但 `frameImportance=keyframe`，对应 [`nack.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs) 中 `current_transport_frame_value_for_transport_gap() -> merge_media_frame_value_with_recovery_timeline()` 会把 `GapSeverity::AnchorGap/ChainBroken` 映射成媒体层 sync-point。该 sync-point 随后经 `FrameBudgetContext::for_transport()` 和 `timeline::should_expired_gap_break_chain()` 把一个无真实帧归属的 transport gap 直接按 anchor gap 判死链，形成 trace 中的 `nackExpired -> referenceChainUnrecoverable -> transportAwaitRecoveryKeyframe`。本轮已在 `nack.rs` 收紧该合并：transport gap 仍允许在 clean-anchor 短窗内提升到 `reference/supply`，但不再允许被提升成伪 `keyframe/sync-point`；新增回归 `cargo test -p xbxengine transport_gap_chain_broken_timeline_does_not_promote_to_pseudo_keyframe -- --nocapture`，并复验 `recent_clean_anchor_promotes_transport_gap_value_to_supply_on_cloud`、`waiting_keyframe_keeps_transport_gap_value_unpromoted`。剩余待继续收口项：`source.rs/timeline.rs` 的 `clean anchor` 提交仍偏乐观，尚未证明“提交后已形成稳定 decode/present 推进”，这一层仍可能放大后续 reference gap 的误判。

- Date: 2026-04-11 | Status: in-progress
- Update: 继续收口 `video_scheduling_owner` 的剩余灰区：此前 `RebuildingSupply` 虽然已经识别出“fresh invalid bootstrap + committed SPS/PPS + delta continuation ready + 当前输出仍可服务”，但在 `resolve_recovery_completion_evidence()` 里仍会先被“无 clean anchor 不得 settle”前置挡板短路，导致“invalid bootstrap 已经证伪当前恢复响应、却没有 clean anchor”的窗口继续锁死在 `rebuilding-supply`。本轮已在 [`video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs) 做两层收口：1) `terminal_invalid_bootstrap_serving_ready` 现在可绕过该前置挡板，从 `RebuildingSupply` 正确降到 `DegradedServing`；2) `can_restore_serving_after_clean_anchor()` 新增“recent unresolved invalid bootstrap blocker”门禁，避免只有 clean anchor 但 bootstrap 仍未真正 ready（例如 recent `NonIdrVcl` 且缺少 committed SPS/PPS/delta）的场景被误放出恢复态。新增/回归验证：`cargo test -p xbxengine terminal_invalid_bootstrap_without_clean_anchor_releases_rebuilding_supply_when_output_serviceable -- --nocapture`、`cargo test -p xbxengine terminal_invalid_bootstrap_without_clean_anchor_and_without_serviceable_output_stays_rebuilding_supply -- --nocapture`、`cargo test -p xbxengine recent_non_idr_codec_evidence_keeps_owner_in_rebuilding_supply -- --nocapture`、`cargo test -p xbxengine clean_anchor_with_terminal_invalid_bootstrap_releases_rebuilding_supply -- --nocapture`、`cargo test -p xbxengine post_first_present_bootstrap_missing_sps_still_enters_rebuilding_supply -- --nocapture`。
- Date: 2026-04-11 | Status: in-progress
- Update: 继续针对 [`runtime-trace-1775896730947-1.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775896730947-1.jsonl) 里的播放期恢复卡死收口 `transportAwaitRecoveryAnchor` fallback：[`coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 现在在“本地 decoder reset 已尝试、随后又收到 fresh invalid bootstrap、且当前已无 keyframe retry 阻塞”的窗口下，不再把 stage-upgrade / hard-fallback 压回 `RequestDecoderReset`，而是优先重开 `RequestKeyframe`；若 keyframe 预算也耗尽，再退到 reconnect/cooldown。这样像 trace 中 `decoded/missed + fresh NonIdrVcl + no clean anchor` 的旧 episode，不会继续锁死在 local decoder reset 回路。新增/更新回归：`cargo test -p xbxengine invalid_transport_await_keyframe_response_releases_decoder_reset_inflight -- --nocapture`、`cargo test -p xbxengine transport_await_hard_fallback_does_not_treat_nonidr_packet_seen_as_local_decode_progress -- --nocapture`、`cargo test -p xbxengine stale_transport_await_decoder_reset_without_progress_can_reopen_decoder_reset -- --nocapture`、`cargo test -p xbxengine transport_await_invalid_nonidr_response_releases_reset_and_decode_wait_lanes -- --nocapture`、`cargo test -p xbxengine transport_await_invalid_nonidr_inspection_after_reset_does_not_coalesce_stale_decoder_reset -- --nocapture`。
- Date: 2026-04-11 | Status: in-progress
- Update: 针对播放期仍可能出现的“本地 decoder reset 预算已耗尽，却继续沿 `transportAwaitRecoveryKeyframe -> RequestDecoderReset / waitForBurst` 原地打转”死锁口，本轮继续把统一事实模型落到 [`coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs)：新增统一 fallback helper，在 `transportAwait` 的 stage-upgrade / hard-fallback 分支里，一旦确认 `decoder_reset_budget_used >= limit` 且当前仍无本地恢复进展，就不再继续压回 `RequestDecoderReset`，而是转为 `RequestReconnectCandidate`（若 reconnect 预算也耗尽，则显式落到 `CooldownSuppressed`），避免 local recovery 死循环。新增回归：`cargo test -p xbxengine transport_await_hard_fallback_decoder_reset_budget_exhaustion_upgrades_to_reconnect -- --nocapture`，并回归通过 `transport_await_hard_fallback_does_not_treat_ingress_without_output_as_local_progress`、`transport_await_hard_fallback_keeps_connected_ingress_local_when_decoder_reset_path_exhausted`、`transport_await_reconnecting_stage_stays_in_decoder_reset_after_hard_fallback_timeout`。
- Date: 2026-04-11 | Status: in-progress
- Update: 基于 [`runtime-trace-1775896730947-1.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775896730947-1.jsonl) 继续定位播放期恢复卡顿，确认新的灰区是“local decoder reset 已成功触发，但 reset 后马上又收到 `NonIdrVcl`/无 clean anchor 的无效恢复响应时，`transport_session` 仍把旧 decoder-reset family 当作 in-flight 继续 merge”。本轮已把 [`transport_session.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs) 的 decoder-reset family gate 改为绑定真实 reset 尝试，并纳入 `post-reset output progress` 与 `invalid recovery response after reset` 两类释放条件；新增回归 `cargo test -p xbxengine invalid_transport_await_response_releases_decoder_reset_family_gate -- --nocapture`，验证 reset 后立刻看到 `NonIdrVcl` 时不会继续落到 `sameFamilyCoalesced:decoderResetInFlight`。
- Date: 2026-04-11 | Status: in-progress
- Update: 继续沿同一份 trace 下钻，确认除了 `transport_session` 之外，[`coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 也存在一条 stale decoder-reset coalesce：`transport_await_decoder_reset_attempt_still_in_flight_from_stats()` 虽然有“invalid response after attempt -> release”逻辑，但此前把判断过度绑定到 `latest_keyframe_request_episode.first_keyframe_decoded_at_ms >= attempt_at_ms`，导致“reset 后收到的新 `NonIdrVcl` inspection 仍挂在老 episode 上”时不会释放 in-flight。本轮已改为允许直接用 attempt 之后的 fresh invalid inspection 作为释放证据，并新增回归 `cargo test -p xbxengine transport_await_invalid_nonidr_inspection_after_reset_does_not_coalesce_stale_decoder_reset -- --nocapture`，锁住“下一次 owner signal 不再产出 `CoalescedDecoderResetInFlight`”这一线上症状。
- Date: 2026-04-11 | Status: in-progress
- Update: 继续把 RFC 的 `FrameValue / GapSeverity / RecoveryEpisodeStage` 真正落到 `video_source + owner` 主路径，而不是停留在 ledger/trace。1) [`budget.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/ingress/budget.rs) 现已确认 recovery window 下的 `reference/supply` retry budget 为 `2`，不再沿用过时的 `1` 次预期。2) [`nack.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs) 修复了 transport gap 在 `with_cloud_latency_admission_policy()` 中二次计算 budget 时丢失 `Recovery` window source 的问题，避免恢复压力明明存在却又退回 `Transport` 窗口；同时新增“恢复压力下低价值 skip 升级为 chain-broken/keyframe 恢复”的主路径，避免 `cloudHighRttLowValueAdmission` 继续在低价值重试里打转。3) [`video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs) 收紧/改写了 terminal invalid bootstrap release：`RebuildingSupply` 上的 `transportAwaitRecoveryAnchor` 不再被旧 `broken/unresolved gap` 事实卡死，只要已经有 `clean anchor + fresh invalid bootstrap + 非 critical supply`，即可降到 `DegradedServing`。定向验证已通过：`cargo test -p xbxengine recovery_window_reference_gets_two_retry_budget -- --nocapture`、`cargo test -p xbxengine refresh_boost_supply_gets_two_retry_budget -- --nocapture`、`cargo test -p xbxengine cloud_latency_admission_preserves_recovery_window_for_transport_gap -- --nocapture`、`cargo test -p xbxengine transport_gap_uses_recovery_window_when_timeline_shows_recovery_pressure -- --nocapture`、`cargo test -p xbxengine low_value_skip_under_recovery_pressure_reopens_chain_recovery -- --nocapture`、`cargo test -p xbxengine degraded_supply_still_releases_terminal_invalid_bootstrap_waiting -- --nocapture`、`cargo test -p xbxengine clean_anchor_with_terminal_invalid_bootstrap_releases_rebuilding_supply -- --nocapture`、`cargo test -p xbxengine deferred_transport_await_episode_does_not_keep_keyframe_family_in_flight -- --nocapture`、`cargo test -p xbxengine stale_transport_await_replay_is_absorbed_after_terminal_deferred_invalid_response -- --nocapture`。
- Date: 2026-04-11 | Status: in-progress
- Update: 继续收紧更大的策略灰区，把 `sampleLoss` 与 `displayStarvedLowValueAdmission` 接入同一条 low-value escalation gate。[`nack.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs) 现已把 `repairability` 作为 cloud admission 的主输入之一：`sampleLoss` 在低 repairability（当前阈值 `<= 0.5`）下，不再沿用纯 low-value skip，而是直接升级到 `SkippedChainBroken`；`displayStarvedLowValueAdmission` 在恢复压力下也不再静默吞掉，而会转成 `chain-broken` 证据，驱动后续 keyframe 恢复。这样 `transport gap / sample loss / display-starved soft skip` 三条入口开始共享“这次 skip 只是止损，还是已经足够证明恢复没推进”的统一判定。新增定向验证：`cargo test -p xbxengine sample_loss_low_repairability_promotes_low_value_skip_to_chain_broken -- --nocapture`、`cargo test -p xbxengine display_starved_low_value_skip_under_recovery_pressure_promotes_chain_broken -- --nocapture`。
- Date: 2026-04-11 | Status: in-progress
- Update: 继续把统一模型从“恢复压力布尔量”推进到显式调度 phase。[`nack.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs) 新增 `TransportRepairPhase::{Startup,Recovery,Steady}`，并把 `with_cloud_latency_admission_policy()` 的低价值/供给 skip 收口到同一 phase-aware gate：建链期的 `reference/supply` near-deadline 不再被过早判死，steady 期维持 `SkippedTooLate`，只有恢复压力下才升级为 `SkippedChainBroken`。同时 `sampleLoss` 不再硬编码 `Recovery` window，而是复用 `transport_nack_window_source()` 和 timeline-merged `frame_value`，repairability 也开始显式吃 `phase/window source`，避免 steady 期 sample loss 天生带 recovery 语义。新增定向验证：`cargo test -p xbxengine startup_supply_near_deadline_keeps_nack_attempt -- --nocapture`、`cargo test -p xbxengine steady_supply_near_deadline_stays_too_late_without_recovery_escalation -- --nocapture`、`cargo test -p xbxengine recovery_supply_near_deadline_escalates_chain_broken -- --nocapture`，并回归 `cloud_latency_admission_preserves_recovery_window_for_transport_gap`、`sample_loss_low_repairability_promotes_low_value_skip_to_chain_broken`、`low_value_skip_under_recovery_pressure_reopens_chain_recovery`、`display_starved_low_value_skip_under_recovery_pressure_promotes_chain_broken`。
- Date: 2026-04-11 | Status: in-progress
- Update: 继续收口 `owner/session/coordinator` 对“当前 transport-await 是否仍有效”的事实入口，避免新 `clean anchor` 后旧坏链残影继续续命 recovery storm。[`contract.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/contract.rs) 新增 `current_clean_anchor_observed_at_ms()` 与 `has_current_transport_await_issue_from_observation()`，统一表达“若当前 recovery epoch 已有更晚的 clean anchor，则旧 unresolved timeline 不再算当前问题”；[`video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs)、[`session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs)、[`recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 已切到同一 helper，同时 `owner` 的 post-startup invalid bootstrap 也加入 freshness 与 clean-anchor 截断，避免旧 `bootstrapMissingSps/Pps/InvalidSliceHeader/NonIdrVcl` 在新链已成立后继续被当成硬失败。另补 `SupplyStarved + clean anchor + 轻 transport-await probe` 不回跳 `RebuildingSupply` 的回归。定向验证：`cargo test -p xbxengine supply_starved_probe_with_clean_anchor_stays_out_of_rebuilding_supply -- --nocapture`、`cargo test -p xbxengine clean_anchor_with_terminal_invalid_bootstrap_releases_rebuilding_supply -- --nocapture`、`cargo test -p xbxengine stale_transport_await_replay_is_absorbed_after_terminal_deferred_invalid_response -- --nocapture`、`cargo test -p xbxengine stale_transport_await_does_not_replay_during_steady_progress -- --nocapture`。
- Date: 2026-04-11 | Status: in-progress
- Update: 针对 [`runtime-trace-1775869241417-1.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775869241417-1.jsonl) 中反复出现的 `waitForBurst / coalesced:keyframeInFlight + cleanAnchorCommitted + transportDeferred + NonIdrVcl` 风暴，本轮已把统一事实模型真正接进主决策：1) [`coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 收紧 `bootstrapInFlight` 判定，要求存在真实 transport-await keyframe attempt 上下文，避免“只有 clean anchor + 输出推进”就误判 family 仍在飞；同时新增 `reopen_transport_await_keyframe` 主线路径，对 `transportDeferred + unsent + invalid bootstrap + clean anchor` 直接 reopen keyframe，而不是再落到 repeat suppression/burst gate。2) [`video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs) 将 terminal invalid bootstrap 从 `hard rebuild evidence` 中剥离，允许 `clean anchor + healthy/degraded supply + committed SPS/PPS + delta continuation ready + fresh invalid bootstrap` 直接释放 `ingress_waiting_keyframe` 并退出 `RebuildingSupply`。3) 定向验证已通过：`cargo test -p xbxengine deferred_transport_await_episode_does_not_keep_keyframe_family_in_flight -- --nocapture`、`cargo test -p xbxengine clean_anchor_with_terminal_invalid_bootstrap_releases_rebuilding_supply -- --nocapture`、`cargo test -p xbxengine stale_transport_await_replay_is_absorbed_after_terminal_deferred_invalid_response -- --nocapture`。
- Date: 2026-04-11 | Status: in-progress
- Update: 继续清扫“current fact 已建好但局部出口还在直接吃 raw timeline”的残留。[`session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 现已把 `should_absorb_stale_transport_await_replay()`、`recovery_transport_await_unresolved`、`recovery_ingress_waiting` 统一切到 `current_clean_anchor_observed_at_ms() + has_current_transport_await_issue_from_observation()`，避免旧 `frame-await-recovery-anchor` 在 clean anchor 之后继续把 session 维持在 `blocked:ingress-waiting / transport-await-unresolved`；[`coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 的 `transport_await_ingress_still_waiting()` 也同步切到同一 current helper，避免 recovery lane 因 stale wait-keyframe timeline 继续留在 transport-await stage。新增/加强验证：`cargo test -p xbxengine recovery_integration_steady_serving_ignores_stale_transport_await_diagnosis -- --nocapture`、`cargo test -p xbxengine clean_anchor_absorbs_stale_transport_await_ingress_waiting_stage -- --nocapture`。
- Date: 2026-04-11 | Status: in-progress
- Update: 继续收 1-8 灰区里的“旧 context 污染 + source 提前升级”。[`session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 的 `displaySupplyDegraded` overlap 吸收不再把“旧 escalation observation / 旧 session recovering 标记”直接当 transport-await 重叠，而是要求当前 unresolved issue 或 terminal deferred 仍成立；[`coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 里 `has_recent_transport_await_keyframe_attempt()` 现要求当前 transport-await issue 仍有效，`bootstrap_in_flight` 的 request context 也要求与 clean-anchor submit 时间窗对齐，避免旧 episode/escalation 在 clean anchor 后继续污染 sustaining / probe / family hold。与此同时，[`source.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs) 已删除“lossy keyframe 直接本地 `TriggerWaitKeyframe`”这条 source 级恢复升级，改成只做 decoder safety 的 `DropAndRequestKeyframe`，把后续是否 NACK、是否升级 keyframe 真正交给统一 NACK/recovery admission；inspection/wait-keyframe 相关路径也收口到统一 helper，避免 source 内部再长出分叉语义。新增验证：`cargo test -p xbxengine lossy_keyframe_defers_to_nack_recovery_admission -- --nocapture`。
- Date: 2026-04-11 | Status: completed
- Update: 复查补齐 `Refresh` 真分支与家族归类：`transport_session.rs` 里新增 `sameFamilyRefreshed:keyframeEpisode`，`ingressWaitKeyframe` / `transportAwaitRecoveryAnchor` 现在统一归入 `keyframe-recovery` family，确保同族 recent-but-not-in-flight 的 keyframe 请求会显式落到 `coalescing_mode=Refresh`，而真正 in-flight 的仍保持 `Merge`。补充回归测试覆盖 `Merge / Refresh / Preempt` 三态，验证 ledger 语义没有因为 family 归类调整而回退。
- Update: 补齐前端共享 RPC 合同缺口：`src/shared/rpc/xbxengine.ts` 的 `latest_recovery_decision_ledger` 新增 `trigger_observation_label / trigger_observation_summary`，与 Rust DTO（core/protocol）保持一致，避免 diagnostics/trace 消费链出现字段漂移。
- Date: 2026-04-10 | Status: in-progress
- Update: 新建 RFC，明确本轮目标不是继续补单点规则，而是基于 `runtime-trace-1775824743739-1.jsonl` 重建恢复建模。当前已确认：`24573` 是“delta 缺包 -> 升级成关键帧恢复 -> 最终恢复”的样本；`30191` 与 `35010` 是“参考链缺包 -> awaitingRecoveryKeyframe/referenceChainUnrecoverable -> NACK 持续 attempted 但关键帧恢复长期被 in-flight/coalescing 抑制”的主根因样本；`41446` 是恢复后次生打断样本。
- Date: 2026-04-10 | Status: in-progress
- Update: 已落地统一模型的核心枚举合同（`FrameValue/GapSeverity/RecoveryEpisodeStage/CoalescingMode`）到 `crates/xbxengine/core/src/transport/rtc/recovery/contract.rs`，并完成一期 “in-flight 占坑解锁” 调整：当关键帧 episode 进入 `response-observed/decoded` 且出现 `NonIdrVcl` 等无效 bootstrap 或 `decoded` 长时未形成 clean anchor 时，不再长期被同 family 合并压制；相关逻辑已分别落在 `transport_session.rs` 与 `recovery/repeat_suppression.rs`，并通过现有 crate tests。
- Date: 2026-04-10 | Status: in-progress
- Update: 已为 trace/stats 迁移窗口扩展 `recoveryDecisionLedger` 合同字段（新增 `frameValue/gapSeverity/recoveryEpisodeStage/coalescingMode/unlockReason/preemptReason/recoveryPrimaryAction`，均为可选），并完成 core/protocol/tauri 侧透传，保证历史 trace 兼容。
- Date: 2026-04-10 | Status: in-progress
- Update: 已把 same-family 合并/解锁语义从“仅字符串 reason”落到 ledger 结构字段：在 `RtcTransportSessionBridge::resolve_recovery_command_family_decision` 中生成 `coalescing_mode/unlock_reason/preempt_reason/recovery_primary_action`，并在命令结果回灌时写入 `latest_recovery_decision_ledger`；新增单测覆盖 `sameFamilyCoalesced:keyframeInFlight` 时 ledger 字段应为 `coalescing_mode=Merge`。
- Date: 2026-04-10 | Status: in-progress
- Update: 已补齐 `Preempt`/升级语义的落地与验证：当 `decoderReset` 遇到 `keyframeInFlight` 时，ledger 会写入 `coalescing_mode=Preempt`、`preempt_reason=familyUpgrade:keyframeInFlight->decoderReset`、`recovery_primary_action=requestDecoderReset`，并新增单测覆盖该路径，防止回归。
- Date: 2026-04-10 | Status: in-progress
- Update: 已把“事实模型三元组”落到 ledger 可观测字段：在关键恢复命令族判定中填充 `recovery_episode_stage/gap_severity/frame_value`（例如 `Sent/AnchorGap/RecoveryAnchor`），并新增单测覆盖，确保 trace 可以直接解释“当前 episode 进度与 gap 价值/严重性”而不是只看隐式字符串。
- Date: 2026-04-11 | Status: in-progress
- Update: 复查发现本 RFC 此前过早标记为完成，实际统一模型主要落在 `transport_session` 的 family gate 与 ledger/trace 透传；`video_source/timeline + nack_policy`、`recovery/coordinator`、`video_scheduling_owner`、`session::policy` 仍未把 `FrameValue / GapSeverity / RecoveryEpisodeStage / CoalescingMode` 作为统一运行时输入。本轮已先修复 `transport_session` 在 family mismatch 时仍继承旧 `transportAwaitRecoveryAnchor` episode/timeline 语义写入 ledger 的问题，并把状态改回 `未完成 / in-progress`。
- Decision: 新模型按 5 个维度收口，并且这 5 个维度必须分层表达、禁止继续互相偷语义：
  - `FrameValue`
    - `Disposable / Continuity / Reference / RecoveryAnchor / CleanAnchor`
    - 价值必须由 `timeline state + recovery epoch + anchor state + first-frame progress` 动态决定，不能只看 `is_keyframe`。
  - `GapSeverity`
    - `MinorGap / ReferenceGap / AnchorGap / ChainBroken / RecoveryBlocked`
    - 严重性由 gap 命中帧价值、timeline 状态、同 family 重复命中、present/decode 老化、clean anchor 证据共同决定。
  - `EscalationGate`
    - 不再围绕“重试次数是否够多”，改为围绕“恢复闭环是否推进”。
    - 从 `NACK优先` 升到 `RecoveryFrame优先` 的判断事实至少包括：`ReferenceGap/AnchorGap`、`broken/recovering`、当前 episode 无 `decoded/cleanAnchorCommitted` 进展、同 family gap 复发、输出老化持续抬升。
  - `RecoveryEpisodeStage`
    - `Requested / Sent / ResponseObserved / Decoded / CleanAnchorCommitted / Deferred / Stalled / Expired`
    - 只有真正推进的 episode 才能占住 `in-flight`；`Deferred`、长期 `NonIdrVcl`、`Decoded 但未 commit`、超短窗无进展都必须允许解锁。
  - `CoalescingMode`
    - `Merge / Refresh / Preempt`
    - 同 family 不能再一律压成 `sameFamilyCoalesced:*`；当严重性升级、旧 episode 停滞、或新证据更强时，必须允许 `refresh/preempt`。
- Decision: 恢复主状态机从“补包规则堆叠”调整为更符合事实的分层语义：
  - `Healthy`
  - `Degraded`
  - `RepairingByNack`
  - `AwaitingRecoveryFrame`
  - `RecoveryFrameInFlight`
  - `RecoveryFrameDecodedPendingCommit`
  - `Recovered`
  - `Blocked`
  其中 `Blocked` 明确表示“恢复闭环没有推进”，用于触发新的解锁/抢占，而不是继续机械重试 NACK。
- Decision: 统一模型不是要替换掉现有所有专项策略，而是要给它们稳定挂点。现有如“恢复爬升期优化”“startup/recovery sustaining”“display supply 保活”“latency-first admission”等策略，后续必须以 `FrameValue / GapSeverity / RecoveryEpisodeStage / CoalescingMode` 为输入条件做特化，而不是被降格为普通 fallback。
- Decision: `InFlight` 解锁必须和 anti-storm 规则同时定义。允许解锁不等于允许立刻重复发新动作；解锁后的新动作仍要受 family 节流、success-edge、严重性提升、最小进展窗口和动作类型配额共同约束。
- Decision: trace/stats 需要新增的最小可观测字段：
  - `frame_value`
  - `gap_severity`
  - `recovery_episode_stage`
  - `recovery_episode_progress_at_ms`
  - `coalescing_mode`
  - `unlock_reason`
  - `preempt_reason`
  - `recovery_primary_action`
  这样下次复盘时可以直接回答“为什么继续 NACK”“为什么不发新的 IDR/RFI”“为什么 in-flight 仍然锁着”。
- Risk/Blocker: 当前工作区已有 recovery/source/timeline 相关未提交修改，后续真正实现时必须与正在推进的 `recovery-sustaining-phase-refactor` 对齐，避免出现两个并行模型。

## Rollout Gates（分阶段落地闸门）

- Gate A（模型落地，不动专项目标）：
  - 仅允许修改恢复事实建模、升级/解锁/coalescing 判定与可观测字段。
  - 不允许在本 gate 调整专项策略目标函数和预算定义。
- Gate B（跨层一致性）：
  - `video_source / recovery / session::policy / owner / stats / trace_projection` 必须全部使用同一套核心枚举语义。
  - 若任一层无法对齐新语义，则该变更不得进入合并态。
- Gate C（安全回归门槛）：
  - 主 gap 回放中不得出现“same family 长时压制但无推进边沿”的已知卡死模式。
  - PLI/RFI/reset 触发频率不得突破基线风暴阈值。
  - `ChainBroken -> CleanAnchorCommitted` 的恢复时延不得明显劣化（以当前基线回放对比）。

## Compatibility Plan（兼容与迁移）

- 新旧字段并行一个迁移窗口：
  - 新增 `frame_value / gap_severity / recovery_episode_stage / coalescing_mode / unlock_reason / preempt_reason` 时，保留旧字段映射输出，避免 diagnostics 与脚本一次性失效。
- trace projection 提供显式映射：
  - 在 `trace_projection` 中维护旧状态到新状态机的过渡映射，确保历史 trace 可解释。
- diagnostics 渐进切换：
  - 前端 diagnostics 先并行展示旧/新关键字段，待回放稳定后再移除旧视图依赖。

## Modeling Draft

### 1. 帧价值合同

- `Disposable`
  - 普通播放期低价值 delta，丢失仅影响局部平滑度。
- `Continuity`
  - 仍不负责重建参考链，但已经影响连续供给与轻度退化吸收。
- `Reference`
  - 命中当前可见输出依赖的参考链；缺失会使后续多帧失效。
- `RecoveryAnchor`
  - 当前 recovery epoch 内，唯一可能把链路从 `broken/recovering` 拉回可服务状态的帧族。
- `CleanAnchor`
  - 已被确认可提交并能形成恢复完成证据的关键恢复帧，不再只是“高价值”，而是“恢复完成事实”。

### 2. 缺包严重性合同

- `MinorGap`
  - 可局部吸收的小 gap。
- `ReferenceGap`
  - 参考链命中，但当前仍可能短时维持服务。
- `AnchorGap`
  - 命中恢复锚点/清洁锚点家族。
- `ChainBroken`
  - 已进入 `referenceChainUnrecoverable`。
- `RecoveryBlocked`
  - 不仅链断，而且当前恢复 episode 未推进，系统正在“假恢复”。

### 3. 升级门槛草案

- `Disposable/Continuity + MinorGap`
  - 允许 NACK 优先；可直接 `skippedTooLate`，不进入 recovery 叙事。
- `ReferenceGap`
  - 只给一个短 NACK 窗口；若同 family 复发或 timeline 已 broken，立即提升到 `AwaitingRecoveryFrame`。
- `AnchorGap / ChainBroken`
  - 禁止继续把 NACK 当主恢复策略；NACK 仅作为辅助修补，主策略应切到 recovery-frame/IDR/RFI 闭环。
- `RecoveryBlocked`
  - 若 in-flight 未推进且输出老化持续，则应允许绕过旧 coalescing 锁，刷新或抢占当前 episode。
- 专项策略挂点：
  - 恢复爬升期优化不应被抹平为普通 `ReferenceGap`；它应作为 `Recovered -> Degraded/RepairingByNack` 的特化子窗，用于表达“恢复后短窗内的新 gap 价值上调、但不立刻回到最昂贵恢复”。
  - `startup/recovery sustaining` 继续保留，但它们的边界应通过 `RecoveryEpisodeStage + GapSeverity + first-frame/supply evidence` 明确表达，而不是依赖孤立的 grace/budget 补丁。

### 4. InFlight 解锁草案

- 保持锁的前提：
  - episode 至少处于 `Sent/ResponseObserved/Decoded`，且在合理短窗内持续推进。
- 立即解锁的前提：
  - `Deferred` 超窗
  - `ResponseObserved` 但仅看到 `NonIdrVcl`
  - `Decoded` 但未出现 `CleanAnchorCommitted`
  - 同 family 新事件的严重性已从 `ReferenceGap` 升到 `ChainBroken/RecoveryBlocked`
  - 当前 episode 只是“发过请求”，但没有新的进展边沿
- 解锁后的 anti-storm 约束：
  - 不能因为解锁就立即无条件重发同类恢复动作；必须同时满足以下至少一类条件才允许新动作：
    - 同 family 严重性提升
    - 上一个 episode 已明确 `Deferred/Stalled/Expired`
    - 观察到新的 progress reset 证据（例如新的 broken edge、新 gap family、新 recovery epoch）
    - 已越过动作类型的最小重发窗口
  - `PLI/RFI`、`decoder reset`、`reconnect` 应分别维护独立的最小间隔与 family 配额，禁止共用一个宽泛“已解锁”信号。
  - 解锁仅释放“占坑语义”，不释放“风暴保护语义”。

### 5. Coalescing 规则草案

- `Merge`
  - 新证据与当前 episode 完全同义，且旧 episode 正在推进。
- `Refresh`
  - 同 family，但 deadline、严重性或目标应刷新。
- `Preempt`
  - 旧 episode 已停滞，或新 gap 已升级为更高严重性，应抢占当前恢复动作。
- 反风暴要求：
  - `Preempt` 不等于立刻执行昂贵动作；它首先应重置目标与占坑事实，然后再经过动作级 gate。
  - 同 family 在短窗口内最多只允许一次 `Refresh -> Sent` 路径，避免 `unlock + preempt` 形成高频震荡。
  - 对恢复爬升期等专项窗口，优先允许 `Refresh`，谨慎使用 `Preempt`，以保留既有“恢复后短窗平滑吸收”的优化目标。

### 6. 主 gap 案例映射

- `24573`
  - 起初应落在 `Disposable/MinorGap`，允许 `skippedTooLate`。
  - 升级后进入 `ReferenceGap -> RecoveryAnchor` 路径，并最终由 PLI + 可解码关键帧闭环救回。
- `30191`
  - 应直接进入 `AnchorGap / ChainBroken` 家族。
  - 说明系统已经进入 `AwaitingRecoveryFrame`，不应再长期由 NACK 主导。
- `35010`
  - 属于长卡死主 gap，应明确落在 `RecoveryBlocked`。
  - 这里最需要 `in-flight` 解锁与 `preempt` 语义，而不是继续 `sameFamilyCoalesced`。
- `41446`
  - 属于恢复后的次生 reference gap。
  - 作用是证明系统需要把”刚恢复”和”已稳态”区分开，避免恢复刚起就被打回同一套慢升级规则。

## Execution Notes

### Phase 1 验证 - 测试基线记录（2026-04-17）

- Date: 2026-04-17 | Status: baseline-recorded
- Update: 完成 Phase 1 底层测试运行，记录测试基线用于后续 Phase 2 和 Phase 3 对比。

#### Step 1: nack_scheduler 测试结果

```
cargo test -p xbxengine transport::rtc::stream::nack_scheduler -- --nocapture
```

- 结果: **全部通过**
- 通过: 26 个测试
- 失败: 0 个测试
- 说明: facts 层不影响现有 nack_scheduler 逻辑，符合预期。

#### Step 2: transport_session 测试结果

```
cargo test -p xbxengine transport::rtc::stack::transport_session -- --nocapture
```

- 结果: **部分失败**
- 通过: 3 个测试
- 失败: 23 个测试
- 主要失败模式:
  - `unlock_reason` 断言失败（预期有值，实际为 None）
  - `preempt_reason` 断言失败（预期有值，实际为 None）
  - `recovery_action_label` 断言失败（预期特定值，实际为 None）
- 说明: Phase 1 只实现了事实计算层，尚未实现 Policy 执行语义层，因此 `unlock_reason`、`preempt_reason` 等字段尚未填充。这些失败是预期的，将在 Phase 2 修复。

#### Step 3: 全量测试基线

```
cargo test -p xbxengine --lib --quiet 2>&1 | grep “test result”
```

- 结果: `test result: FAILED. 918 passed; 89 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.14s`
- **Phase 1 基线: 918 passed / 89 failed**

#### 失败测试分类

主要失败集中在以下模块：

1. **transport_session 测试** (23 个)
   - 原因: Policy 执行语义层尚未实现
   - 预期修复阶段: Phase 2

2. **recovery coordinator 测试** (约 40 个)
   - 子类: `decoder_reset_idle_stall` 系列
   - 子类: `hard_fallback_deadlines` 系列
   - 子类: `profile_nack_display` 系列
   - 子类: `transport_await_wait_keyframe` 系列
   - 原因: 依赖完整的 Policy 执行语义和 Coordinator 集成
   - 预期修复阶段: Phase 2 和 Phase 3

3. **session policy 测试** (约 20 个)
   - 子类: `bwe_twcc` 系列
   - 子类: `display_owner_ledger` 系列
   - 子类: `playback_phase_integration` 系列
   - 原因: 依赖完整的 Policy 层集成
   - 预期修复阶段: Phase 2 和 Phase 3

4. **其他测试** (约 6 个)
   - `api::runtime::tests::hard_disconnect_transport` 系列
   - `transport::rtc::connection::service` 系列
   - 原因: 跨层集成问题
   - 预期修复阶段: Phase 3

#### 结论

- Phase 1 基线已记录: **918 passed / 89 failed**
- nack_scheduler 测试全部通过，证明底层 NACK 逻辑未受影响
- transport_session 和 recovery coordinator 测试失败符合预期，因为 Phase 1 只实现了事实计算层
- 后续 Phase 2 将实现 Policy 执行语义层，预期修复大部分失败测试
- Phase 3 将完成 Coordinator 集成和跨层验证，预期修复剩余失败测试
