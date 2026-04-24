# 图片级恢复动作单轨切换 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: in-progress
- Owner: Codex
- Last Updated: 2026-04-23

## Background

- 当前恢复主链在上层长期使用泛动作 `RequestKeyframe`，下游再展开成 `PLI -> FIR -> control keyframe`。
- 当前 `session/policy`、`video_scheduling_owner`、`recovery/coordinator`、`connection/service` 共同参与“要不要关键帧”的解释，display 域也还会沿旧合同推动图片级恢复。
- 当前 trace 已经证明坏链有三类高频窗口：
  - `NonIdrVcl` continuation 已到达，但仍停在等待锚点
  - usable IDR 已到达，但 active episode / clean-anchor 语义没有前移
  - `RtcVideoFrameSource rx closed` 作为 ingress/lifecycle 硬断点，和媒体恢复叙事并列出现
- 当前 runtime trace 已补齐：
  - `response-observed` summary：`firstVideoPacketSeq`、`firstKeyframePacketSeq`、`oosDepthP75`、`headMissingActive`、`gapExpiredBeforeKeyframe`
  - `rx closed` 因果标签：`rebuildPeerConnection`、`stackStop`
- 当前系统已经具备可复用的策略资产：
  - `RecoveryProgressLevel`
  - `DynamicRepairValueTier`
  - `clean anchor`
  - `post-IDR climbing`
  - `remote profile`
- 当前主问题不是“策略不够多”，而是“动作主权、阶段判据、trace 叙事、画像策略入口”混在一起。

## Goal

- 删除泛动作 `RequestKeyframe`，将图片级恢复动作改为显式 `RequestPli` 与 `RequestFir`。
- 删除 display-domain 直驱图片级恢复的旧合同。
- 删除常规 `control keyframe` 主链，使 transport control path 成为唯一图片级恢复出口。
- 让 owner 只负责阶段与完成判据，session 只负责编排，coordinator 只负责显式动作决策，trace 直接投影显式动作名。
- 保留现有远端画像、`post-IDR climbing`、clean-anchor 宽容，统一收成 phase policy 参数层。
- 固定恢复进度链：`WaitingResponse -> ContinuationSeen -> AnchorSeen -> Decoded -> CleanAnchorCommitted -> DisplayStable`
- 固定控制动作链：`NACK -> PLI`，`FIR` 仅重保底。

## Scope

- In scope:
  - [`crates/xbxengine/core/src/transport/rtc/recovery/contract.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/contract.rs)
  - [`crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs)
  - [`crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs)
  - [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs)
  - [`crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs)
  - [`crates/xbxengine/core/src/transport/rtc/connection/service.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/connection/service.rs)
  - [`crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs)
  - [`crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs)
  - [`crates/xbxengine/core/src/media/video/ingress/budget.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/ingress/budget.rs)
  - [`crates/xbxengine/core/src/runtime_stats_sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/runtime_stats_sink.rs)
  - [`src-tauri/src/mods/xbxengine/trace_projection.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/xbxengine/trace_projection.rs)
- Out of scope:
  - 重做 renderer / pacer / host scheduling 主链
  - 新增评分器、连续浮点恢复模型、并行恢复路径
  - 重做 renderer / pacer / host scheduling 主链

## Design

### 1. 总体模型

- 控制动作链固定为 `NACK -> PLI`
- `FIR` 只在 `PLI` 持续无锚点响应时作为重保底出现
- 恢复进度链固定为：
  - `WaitingResponse`
  - `ContinuationSeen`
  - `AnchorSeen`
  - `Decoded`
  - `CleanAnchorCommitted`
  - `DisplayStable`
- phase policy 参数层统一承接：
  - `idr_protection_window_ms`
  - `post_idr_repair_budget`
  - `clean_anchor_confidence_window_ms`
  - `transport_reorder_window_packets`
  - `transport_repair_deadline_ms`
  - `pli_min_interval_ms`
  - `fir_escalation_delay_ms`
  - `decoder_reset_after_anchor_stall_ms`

规则：

- `NonIdrVcl` 是 `ContinuationSeen` 事实，不单独构成升级证据
- `cleanAnchorCommitted` 是 media recovery complete
- `DisplayStable` 是 display recovery complete
- display 只做 completion gate 与本地保供给，不再参与图片级恢复动作决策

### 2. 单轨动作合同

- [`crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs) 的 `RecoveryAction` 改为：
  - `RequestPli`
  - `RequestFir`
  - `RequestDecoderReset`
  - `RequestReconnectCandidate`
  - 保留 `CooldownSuppressed` 等非执行动作
- 删除 `RequestKeyframe`
- 任何模块仍输出泛化 `requestKeyframe` 都视为语义回退

### 3. 动作边界

- `progress < AnchorSeen`
  - `NACK`
  - `RequestPli`
  - `RequestFir`
- `progress >= AnchorSeen && < Decoded`
  - `RequestDecoderReset`
- lifecycle/connectivity 域
  - `RequestReconnectCandidate`

规则：

- `RequestPli` 是唯一常规图片级恢复动作
- `RequestFir` 只在 `PLI` 节流后持续缺 `AnchorSeen` 时出现
- display 事实不得直接映射到 `RequestPli` 或 `RequestFir`

### 4. 前后差异

改前：

- 上层统一产出 `RequestKeyframe`
- transport 下游再决定 `PLI/FIR/control`
- display/owner/session/coordinator 多层都能影响“要不要关键帧”
- trace 主叙事是 `requestKeyframe`
- 画像策略散落在 `session / owner / source / nack`

改后：

- 上层直接产出 `RequestPli / RequestFir / RequestDecoderReset / RequestReconnectCandidate`
- transport 只执行显式动作，不再做泛动作二次解释
- display 退出图片级恢复动作链
- owner 只管阶段与 completion gate，session 只管编排，coordinator 只管显式动作决策
- trace 主叙事直接等于显式动作名
- 画像策略统一从 phase policy 参数层进入 `nack / budget / owner-completion gate`

### 5. 文件级改法

#### A. `recovery/contract.rs`

- 固化 `RecoveryProgressLevel`
- 提供 `progress -> allowed actions`
- 保持 `ContinuationSeen / AnchorSeen / Decoded / CleanAnchorCommitted / DisplayStable` 的结构化投影
- 为 `session/policy`、`coordinator`、`trace` 提供统一事实口径

#### B. `recovery/escalation.rs`

- 删除 `RequestKeyframe`
- 所有等待锚点分支改为产出 `RequestPli`
- `PLI` 达到节流上限且仍缺锚点时产出 `RequestFir`
- `label()`、contract、测试断言全部改为显式动作名

#### C. `recovery/coordinator.rs`

- 输入继续消费结构化恢复事实
- 输出显式 `RequestPli / RequestFir / RequestDecoderReset / RequestReconnectCandidate`
- `clean anchor`、`decoded` 只推进 progress，不再回写泛化 keyframe 动作

#### D. `session/policy.rs`

- 删除所有 `RecoveryAction::RequestKeyframe` 分支
- 删除所有 display-domain `reason -> RequestKeyframe` 分支
- 只消费：
  - `RecoveryProgressLevel`
  - `fault domain`
  - `phase policy`
  - `coordinator` 返回的显式动作
- ramp-up、startup guard、reconnect fallback 继续保留，但只作用于显式动作

#### E. `video_scheduling_owner.rs`

- 删除图片级恢复动作建议输出
- owner 只保留阶段、completion gate、diagnostics 所需事实
- `DisplaySupplyCritical`、`HostPresentStalled` 继续存在于 diagnostics，不再进入图片级恢复动作链
- `clean anchor / DisplayStable` 只做完成判据

#### F. `stream/video_source/nack.rs` 与 `media/video/ingress/budget.rs`

- 不重写现有四档动态修复价值：
  - `Anchor`
  - `Continuation`
  - `Supply`
  - `Disposable`
- 将 phase policy 参数统一接入这两层
- 保留 `post-IDR climbing`
- 保留 clean-anchor 宽容
- `NACK` 继续承担低延迟 transport repair
- `NACK` 不再承担升级到泛化 keyframe request 的语义
- `nack_skip_last_n` 改为 phase policy 驱动的乱序容忍参数，按近期 OOS 深度分位桶动态调节，并保留短节流避免窗口振荡

#### G. `connection/service.rs`

- 删除常规 `control keyframe` 主链
- 拆出显式入口：
  - `request_video_pli()`
  - `request_video_fir()`
- control channel keyframe 只保留给非主链场景
- cloud 场景的 TWCC feedback interval 改为 warmup/stable 两段式参数：
  - 未形成 video feedback 证据时沿配置值
  - 已看到 video remote binding 或 inbound extension 时收紧到 50ms
  - 已形成稳定 `local-feedback` 后回到 100ms

#### H. `transport_session.rs`

- `TransportCommand` 与执行分发改为调用显式 `PLI/FIR` 入口
- 删除围绕泛化 `RequestKeyframe` 的 observation / command kind 映射

#### I. `runtime_stats_sink.rs` 与 `trace_projection.rs`

- 删除泛化 `requestKeyframe` 主叙事
- trace 直接投影：
  - `requestPli`
  - `requestFir`
  - `requestDecoderReset`
  - `requestReconnectCandidate`
- 保留 `response-observed` summary 增强字段
- 保留 `rx closed` 的 `rebuildPeerConnection / stackStop` 因果标签
- 在 picture recovery episode 中补 `firstFrameLatencyTrace`，统一记录：
  - `controlReadyToPliSentMs`
  - `pliSentToFirstIdrPacketMs`
  - `firstIdrPacketToFirstDecodeMs`

### 6. 删除清单

- 删除 `RecoveryAction::RequestKeyframe`
- 删除 `connection/service.rs` 中常规 `control keyframe` 主链
- 删除 `session/policy.rs` 中 `display -> RequestKeyframe` 分支
- 删除 `video_scheduling_owner.rs` 中任何图片级恢复动作建议输出
- 删除 trace 主链上的泛化 `requestKeyframe` 叙事

## Plan

1. 固化 `RecoveryProgressLevel` 与 phase policy 接入点
2. 改 `RecoveryAction` 与相关 contract，删掉 `RequestKeyframe`
3. 改 `connection/service` 与 `transport_session`，收口 transport 图片级恢复出口
4. 改 `session/policy`、`owner`、`coordinator`，删 display->keyframe 与泛化 keyframe 语义
5. 改 `nack/budget`、trace / runtime stats / tests，收口 phase policy 与显式动作叙事

## Validation

- [ ] `cargo test -p xbxengine transport::rtc::recovery::contract -- --nocapture`
- [ ] `cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`
- [ ] `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
- [ ] `cargo test -p xbxengine transport::rtc::policy::video_scheduling_owner -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::connection::service -- --nocapture`
- [ ] `cargo test -p xbxengine transport::rtc::stream::video_source::nack -- --nocapture`
- [ ] `cargo test -p xbxengine recovery_integration -- --nocapture`
- [ ] `cargo test -p xbxrc trace_projection -- --nocapture`
- [ ] `rg -n "RequestKeyframe|requestKeyframe" crates/xbxengine/core/src/transport/rtc src-tauri/src/mods/xbxengine/trace_projection.rs` 仅允许保留历史兼容注释，不允许出现在新主链逻辑
- [ ] `cargo test -p xbxengine recovery_integration -- --nocapture` 中不再存在 `displaySupplyCritical -> RequestKeyframe`
- [ ] 用新 runtime trace 验证 `NonIdrVcl -> IDR -> decoded -> cleanAnchorCommitted` 会被识别为同一恢复序列推进
- [ ] 用新 runtime trace 验证 `PLI -> firstKeyframePacketSeq -> first decode` 的三段时延
- [ ] 用新 runtime trace 验证 `rx closed` 会进入 `rebuildPeerConnection / stackStop` 因果链，不再污染 media progress
- [x] `cargo test -p xbxengine dynamic_nack_skip_last_n_uses_oos_percentile_buckets -- --nocapture`
- [x] `cargo test -p xbxengine dynamic_nack_skip_last_n_is_rate_limited -- --nocapture`
- [x] `cargo test -p xbxengine twcc_feedback_interval_uses_warmup_and_stable_targets_for_cloud_video -- --nocapture`
- [x] `cargo test -p xbxengine keyframe_request_episode_packet_seen_and_decoded_resolve_verdict -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::stream::video_source::source -- --nocapture`
- [x] `cargo check -p xbxengine`

## Risks

- 单轨切换会同时影响 enum、command routing、trace、tests，改动面集中且深。
- 如果 `control keyframe` 主链删得不彻底，运行时会继续保留图片级恢复双出口。
- 如果 display-domain 到图片级恢复动作的旧分支删不净，复杂度会以新枚举名继续存在。
- 如果 phase policy 没有统一接到 `nack/budget/owner completion gate`，现有画像经验会继续散落在局部 if-else 里。

## Progress

- [x] Step 1: 已确定单轨目标是 `PLI` 主路径、`FIR` 重保底、删除 `RequestKeyframe`
- [x] Step 2: 已完成动作枚举主线收口、连接层命名收口、control-path keyframe 兼容轨删除
- [x] Step 3: 已完成 transport 出口到显式 `PLI/FIR` 入口的切换，手动请求与 delayed prime 统一走 PLI outcome 模型
- [ ] Step 4: 完成 `nack/budget`、trace 与测试收口

## Execution Notes

- Date: 2026-04-22 | Status: planned
- Decision: 本 RFC 自洽描述本次改造目标，直接承载背景、目标、动作单轨、progress、phase policy、删除清单、文件改法与验证矩阵，不再依赖另一份 RFC 提供改造目标。
- Decision: `RequestKeyframe` 直接删除，不保留双轨兼容期。
- Decision: display 域退出图片级恢复动作链，只保留 diagnostics、本地保供给、completion gate 职责。
- Date: 2026-04-23 | Status: in-progress
- Update: 已将 `RuntimeStatsSink` 写入 API 统一收口为 `record_picture_recovery_episode_*`，并补 `firstFrameLatencyTrace` 三段时延，直接把 `control ready -> PLI sent -> first IDR packet -> first decode` 写回 episode `transport_detail`。
- Update: 已将 cloud TWCC feedback interval 收口为 warmup/stable 两段式：video binding 前沿配置值，binding 后 50ms，稳定 `local-feedback` 后 100ms。
- Update: 已将 `RtcVideoFrameSource` 的 `nack_skip_last_n` 从固定值改成近期 OOS 深度驱动的分位桶参数，当前分档为 `2/4/6`，并加 200ms 刷新节流。
- Update: 已将 recovery close 收口为两层 gate：`cleanAnchorCommitted -> DisplayStable`。`ramp_guard` 不再保留独立 settle 语义，只消费 owner 已给出的 `stable-serving/degraded-serving` 结果；`stableServingSettled` 仅作为 `DisplayStable` 的落账事件名。
- Update: fresh output / host present serviceable 窗口已统一从 `500ms/1500ms` 收敛到 `300ms`，与低延迟目标一致。
- Update: 已将本地媒体输出链按恢复阶段扩容：`staleAfterDecode` slack 提升到 `24/48/72ms + 48ms recovery bonus`，decode 输出队列从稳定期 `3` 帧扩到恢复期 `5` 帧，pacer 恢复队列扩到 `5` 帧、hard cap 扩到 `8` 帧，render pending 队列在 recovery/priming 期允许 `2` 帧。
- Update: `HostCadencePhaseHint` 对恢复期的收紧已降一级：常态 `Starved` 仅收紧到 `2` 帧，`Priming` 保持 `3` 帧；recovery window 不再被 host starved 二次压回极浅队列。
- Update: runtime session watchdog 已把 `outputQueueOverflow/staleAfterDecode/hostPresentStalled` 收口为本地 `media self-healing` 信号，只有 `NACK expired`、视频包长期中断、TWCC 新鲜度丢失这类 transport 硬证据成立时才升级 `RequestVideoKeyframe/RequestReconnect`。
- Validation: `cargo check -p xbxengine`、`cargo test -p xbxengine transport::rtc::connection::service -- --nocapture`、`cargo test -p xbxengine transport::rtc::stream::video_source::source -- --nocapture`、`cargo test -p xbxengine dynamic_nack_skip_last_n_uses_oos_percentile_buckets -- --nocapture`、`cargo test -p xbxengine dynamic_nack_skip_last_n_is_rate_limited -- --nocapture`、`cargo test -p xbxengine twcc_feedback_interval_uses_warmup_and_stable_targets_for_cloud_video -- --nocapture`、`cargo test -p xbxengine keyframe_request_episode_packet_seen_and_decoded_resolve_verdict -- --nocapture`、`cargo test -p xbxengine session::recovery -- --nocapture`、`cargo test -p xbxengine media::video::pacer::actor -- --nocapture`、`cargo test -p xbxengine recovery_signals_hold_keyframe_on_local_pipeline_stall_without_transport_evidence -- --nocapture`、`cargo test -p xbxengine enqueue_decoded_frame_recovery_window_uses_deeper_queue_budget -- --nocapture`、`cargo test -p xbxengine recovery_window_frames_allow_deeper_local_buffer_before_release -- --nocapture` 已通过。
