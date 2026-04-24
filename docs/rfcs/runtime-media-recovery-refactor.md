# 运行时/媒体恢复链解耦 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: completed (phase4)
- Owner: Codex
- Last Updated: 2026-04-10 (update-7)

## Background

- 当前恢复链将 `RequestDecoderReset` 作为统一升级动作的一环，通过 transport recovery / scheduling / RTCP 控制面向网络侧下发。
- 媒体链已有较强的本地自愈能力（本地 decoder reset、硬解失败软回退、no-output 保护、队列背压丢帧），但高层仍过多参与“中间档”恢复决策。
- 目标是对齐 moonlight-qt 的思路：媒体链优先局部自愈，高层只在观测到“媒体自愈失败且无推进”时才做会话层兜底 reconnect。

## Goal

- 将恢复链重构为“两层模型”：
  - 媒体局部恢复层：`drop/skip/request keyframe/local decoder reset`，尽量在链路内部完成恢复和时效控制。
  - 会话兜底层：基于统一的观测快照 `RecoveryObservationSnapshot` 决策是否 `request reconnect`，并在会话重连成功后尽快 `request keyframe`。
- `RequestDecoderReset` 收紧为纯本地 decoder 维护动作，不再作为统一升级恢复动作，不再映射为 transport/RTCP 控制命令。
  - 允许来源仅限 decoder/render 本地异常与维护需求，例如 decoder 初始化/重建失败、解码器明确返回不可恢复错误、渲染设备切换或丢失后必须重建 decoder pipeline。
  - 禁止来源包括纯媒体停滞、无新帧、NACK 过期、TWCC 异常、音视频短时失衡等“仅凭高层 stall/transport 观测升级”的路径。
- 保持既有 BWE 门控与 `pending reconnect candidate` 语义稳定，不引入额外重连抖动。
- 补齐日志/指标语义：可区分“本地 decoder 维护动作”和“网络/会话恢复动作”，方便线上判障。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/session/policy.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/hard_stall.rs`
  - `crates/xbxengine/core/src/transport/rtc/policy/scheduling.rs`
  - `crates/xbxengine/core/src/transport/rtc/policy/recovery.rs`
  - `crates/xbxengine/core/src/transport/rtc/projection/recovery.rs`
  - `crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs`
  - `crates/xbxengine/core/src/api/runtime/lifecycle.rs`
  - `crates/xbxengine/core/src/transport/rtc/session/recovery_ramp_guard.rs`
  - `crates/xbxengine/core/src/media/video/decode/video_decode.rs`
  - `crates/xbxengine/core/src/media/video/ingress/*`
  - `crates/xbxengine/core/src/media/video/render/*`
  - `crates/xbxengine/core/src/transport/rtc/recovery/nack_outcome.rs`
  - 相关测试文件：`escalation.test.rs`、`session/policy.test.rs`、`video_decode.test.rs` 等。
- Out of scope:
  - 前端 UI 与输入路由模型。
  - 非 RTC/媒体链路相关的其它恢复路径。

## Plan

1. **阶段 1：语义去耦与动作边界收口**
   - 先明确 owner：
     - reconnect / BWE / recovery proposal 的主决策继续收口在 `transport/rtc/session/policy.rs`。
     - `stack/transport_session` 与 `api/runtime/lifecycle` 只负责执行与桥接，不新增并行恢复决策。
   - 在 `recovery/escalation` 中重定义恢复动作归属：
     - 将 `RequestDecoderReset` 从“升级链动作”改为“仅本地维护动作”或替换为内部信号，不再参与 transport recovery epoch 推进和升级预算门控。
     - 同步收敛 `RequestKeyframeAndDecoderReset`、`StartupLowQualityRetry` 等组合动作：阶段 1 内必须改写为“本地 reset 信号 + 可选 keyframe 请求”或直接退场，禁止再经 scheduling 映射出 transport decoder reset。
   - 在 `policy/scheduling` 中调整映射：
     - `map_recovery_action_to_transport_commands()` 不再输出 `TransportCommand::RequestDecoderReset`。
     - 仅保留 `RequestKeyframe` 与 `RequestReconnectCandidate` 的对外映射；组合动作也不得旁路产出 decoder reset 命令。
   - 在 session/stack/runtime 中清理遗留路径：
     - `session/policy`、planner、`policy/recovery` 不再规划、暂存、消费 decoder reset 为恢复升级动作。
     - `recovery/hard_stall`、`session/recovery_ramp_guard`、`projection/recovery` 等直接依赖 reset 语义的路径同步改为本地维护语义或显式忽略，不保留“半解耦”中间态。
     - runtime 生命周期不再将 decoder reset 作为 recovery escalation 的 milestone。
   - 阶段 1 完成标准：
     - 仓库内不存在任何从 recovery proposal 直达 `TransportCommand::RequestDecoderReset` 的路径。
     - `transport_recovery_epoch`、budget ledger、ramp-up / hard-stall 判断不再把 decoder reset 视为 transport 升级动作。
     - 在阶段 1 结束时，即便阶段 2/3 尚未开始，现有 reconnect / BWE 主线仍可稳定工作且测试可独立验证。

2. **阶段 2：媒体链“早判坏/早放弃”策略前移**
   - 在 ingress/NACK/帧恢复判定中引入 deadline/价值驱动：
     - 缺包预计晚于播放截止时间时直接 skip，不再进入 NACK 重试。
     - 对低价值非关键帧提高放弃倾向，优先保障关键帧参考链。
     - `NackScheduler` 继续保留并强化 `deadline / max-age / budget` 机制，但其职责明确为“帮助及时恢复”，而不是“尽量保住每个包”。
   - 在 decode/render 队列中强化时效优先：
     - 明确旧帧主动丢弃策略，避免排队导致尾延迟扩张。
     - 提前标记 unrecoverable，阻断“已无时效价值帧”继续占用恢复预算。
     - NACK、帧重组、解码提交都要受播放时效约束，而不是只看理论上是否仍可恢复。

3. **阶段 3：观测驱动的会话兜底与重连后的关键帧恢复**
   - 引入或收束到统一的 `RecoveryObservationSnapshot`：
     - 媒体阶段：`Ingress`、`Reassembly`、`Decode`、`Render/Playout` 状态。
     - 会话阶段：`RtcConnectivity`、`ReconnectInFlight`、`StableServing`。
     - 关键计时：`last_media_progress_at`、`last_video_decode_ok_at`、`last_keyframe_requested_at`、`last_keyframe_decoded_at`。
     - 关键计数：`local_decoder_reset_count_in_window`、`keyframe_request_count_in_window`、`nack_skip_count_in_window`。
   - 将兜底决策改为观测驱动：
     - 会话层只依据 `RecoveryObservationSnapshot` 判断是否进入 reconnect。
     - 必须满足“媒体链已执行局部自愈尝试且长时间无推进”，且观测表明不再处于合理的 keyframe 恢复窗口。
   - 在重连成功路径中显式补齐关键帧请求：
     - reconnect settled -> `RequestKeyframe(reason=reconnectSettled)`，受节流/在飞保护，避免重复刷屏。

4. **阶段 4：观测语义与测试体系重建**
  - 日志/指标重构：
    - 新增或重命名事件维度，显式区分 `local_decoder_maintenance` 与 `network_session_recovery`。
    - 移除“decoder reset 作为 recovery escalation 里程碑”的旧口径。
  - 单元与集成测试覆盖：
    - 恢复动作映射测试（确保 decoder reset 不再映射为 transport/RTCP 命令）。
     - decode 本地 reset 触发源测试（仅 decoder/render 本地异常可触发）。
     - decoder reset 禁止来源测试（纯媒体停滞、NACK 过期、TWCC 波动、音视频短时失衡不能再触发 reset）。
     - deadline/预算差异化策略测试（关键帧 vs 非关键帧）。
     - reconnect 兜底条件测试与 BWE 回归测试。

## Validation

- [ ] 任意恢复提案经过 scheduling 后，不会产生 decoder reset 的 transport/RTCP 命令。
- [ ] `RequestKeyframeAndDecoderReset` / `StartupLowQualityRetry` 等组合动作已被移除、改写或内化，不再形成 transport reset 旁路。
- [ ] `RequestDecoderReset` 的允许来源与禁止来源边界已落地：仅 decoder/render 本地异常可触发，纯 stall/NACK/TWCC/AV sync 波动不能再触发。
- [ ] `session/policy` 仍是 reconnect / recovery / BWE 的唯一主决策 owner，`stack/runtime` 未重新长出并行恢复判定。
- [ ] `transport_recovery_epoch`、budget ledger、ramp-up / hard-stall 语义在阶段 1 后保持一致，不存在“文义解耦、状态机未解耦”的灰区。
- [ ] 丢包压力下关键帧与非关键帧的恢复预算和放弃策略表现出预期差异。
- [ ] 短时视频停顿能优先通过媒体链局部自愈解决，而不会直接触发 reconnect。
- [ ] 音频仍活但视频短暂停滞时，不会因 decoder reset 中间档移除而过早触发 reconnect。
- [ ] reconnect 触发频率在典型网络环境下不异常增加。
- [ ] reconnect settled 后能在可预期的短窗口内拉起关键帧（无异常抑制或丢失）。
- [ ] BWE 与 `pending reconnect candidate` 的行为与原有约束一致。
- [ ] 新增日志/指标字段能够区分本地维护与会话恢复动作，且与文档描述一致。

## Risks

- 语义半解耦风险：
  - 部分路径仍然把 `RequestDecoderReset` 当作升级动作计入预算或 epoch，导致状态机行为灰色地带。
- 重连抖动回归：
  - 兜底判定条件调整不当时，可能在弱网下频繁 reconnect。
- 可观测口径断层：
  - 旧告警规则依赖的字段或语义发生变化，导致告警盲区或误报。

## Progress

- [ ] 阶段 1：语义去耦与动作边界收口
  - 当前状态：进行中，主路径已切断 transport decoder reset 映射，组合动作已退场，本地维护执行链已分轨；`hard_stall` 与 `policy/recovery` 的语义收口已继续推进（非 reconnect 动作统一归属 Local 域、hard stall 采样回退口径去除 reconnect 命名残留），`projection / runtime 口径 / 阶段 1 完整验证` 仍需继续收口。
- [x] 阶段 2：媒体链“早判坏/早放弃”策略前移
  - 当前状态：已完成。已落地 ingress backlog 过期帧主动清理、decode 后 stale frame 丢弃、NACK 低价值/参考帧近 deadline 放弃、render queue 价值优先替换与 stale 置换，并补齐对应单测。
- [x] 阶段 3：观测驱动的会话兜底与重连后的关键帧恢复
  - 当前状态：已完成。`session/policy` 已使用统一 `RecoveryObservationSnapshot` 驱动 transport-await reconnect fallback（仅在“已执行局部自愈 + 长时间无推进 + 超出 keyframe 恢复窗口”时放行），并在 `runtime/lifecycle` 的 reconnect settled 路径补齐 `request keyframe` 的在飞/冷却保护。
- [x] 阶段 4：观测语义与测试体系重建
  - 当前状态：已完成。`diagnostics/stats` 已新增“`local_decoder_maintenance` vs `network_session_recovery`”决策摘要口径；`recovery/runtime_state` 已将 decoder-reset 窗口口径更名为 `local-maintenance-window`，并将 decoder-reset 失败代价下调为中等，移除“decoder reset 作为 recovery escalation 高代价里程碑”的旧语义残留。

## Execution Notes

- Date: 2026-04-09 | Status: planned
- Update: 初始 RFC 建立，自上而下确定“两层模型 + 观测驱动兜底 + 重连后关键帧恢复”的执行路线。
- Decision: 优先完成语义去耦与调度收口，再逐步下沉媒体策略与补齐测试/观测。
- Update: 根据评审补齐实施 owner 与阶段 1 边界：明确 `session/policy` 是 reconnect / recovery / BWE 唯一主决策收口点，并将 `coordinator/hard_stall/recovery_ramp_guard/projection` 一并纳入本 RFC 的语义去耦范围。
- Decision: 阶段 1 必须同时处理组合动作 `RequestKeyframeAndDecoderReset` / `StartupLowQualityRetry` 的退场或内化，避免仅移除单一映射后仍残留 transport reset 旁路。
- Risk/Blocker: 若只改 `scheduling` 与 `runtime` 而不同步收敛 `session/policy`、`hard_stall`、`ramp-up`、ledger/projection，中间态会落入“半解耦”灰区，不能视为阶段 1 完成。
- Date: 2026-04-09 | Status: in-progress
- Update: 开始阶段 1 落地。已在 `policy/scheduling` 切断 `RecoveryAction::RequestDecoderReset -> TransportCommand::RequestDecoderReset` 的映射，并将 `RequestKeyframeAndDecoderReset` / `StartupLowQualityRetry` 收口为仅发 `RequestKeyframe`。
- Update: `recovery/coordinator` 不再在 startup fast reset 路径产出 `RequestKeyframeAndDecoderReset`；`session/policy` 与 `session/recovery_ramp_guard` 已同步改为不再把组合动作/decoder reset 作为 transport 侧升级命令处理。
- Update: 已同步修订 `session/policy.test.rs` 的阶段 1 合同断言，改为校验“不再下发 transport decoder reset，但 ledger 仍保留本地维护动作语义”。
- Update: 阶段 1 执行链继续收口为“双通道”模型：新增 `SessionCommand::LocalDecoderReset` 作为本地维护命令，`RtcSessionPolicy`/`SessionActor`/`RtcTransportSessionBridge` 已切到 `transport command` 与 `local decoder maintenance` 分轨执行；`TransportCommand::RequestDecoderReset` 仅保留兼容壳，不再由 scheduling 主路径产出。
- Update: 阶段 2 已启动。`media/video/ingress/scheduler.rs` 现会在入队前清理已失时的旧 backlog 帧；`media/video/decode/video_decode.rs` 新增 decode 后 stale frame 直接丢弃，避免无时效价值帧继续占用 decode/render 队列。
- Risk/Blocker: 当前环境缺少 `cmake` 与 `pkg-config`，`cargo test -p xbxengine ...` 在依赖构建阶段失败，暂时只能完成代码修改与静态 diff 核对，待补齐本机构建依赖后再做完整回归。
- Date: 2026-04-09 | Status: in-progress
- Update: 阶段 1 继续收口：`policy/recovery::resolve_runtime_reconnect_reason_domain()` 已改为“仅 `RequestReconnectCandidate` 走 reconnect reason-domain 解析，其他动作统一 `Local` 域”，避免本地维护动作借道连接域语义；并新增对应单测。
- Update: `recovery/hard_stall` 常量语义清理：`HARD_STALL_RECONNECT_MS` 重命名为 `HARD_STALL_SAMPLE_AGE_FALLBACK_MS`，明确其用途是 hard stall 采样回退窗口而非 reconnect 决策阈值，减少阶段 1 文义残留。
- Date: 2026-04-09 | Status: in-progress
- Update: 阶段 2 收尾完成：`video_source/nack` 新增 low-value 与 supply near-deadline guard（`estimatedArrivalNearDeadlineLowValue` / `estimatedArrivalNearDeadlineSupply`），`media/video/pacer/actor` 新增 render queue 价值优先保留与 stale 置换策略（`rendererQueueRejectLowerValue` / `rendererQueueReplaceStale`）。
- Update: 阶段 2 关键单测通过：`supply_near_deadline_guard_triggers_with_tighter_window`、`render_queue_keeps_existing_higher_priority_frame`、`render_queue_replaces_existing_stale_frame_even_if_priority_is_lower`。
- Date: 2026-04-09 | Status: in-progress
- Update: 阶段 3 启动：在 `transport/rtc/session/policy.rs` 引入统一 `RecoveryObservationSnapshot` 采集骨架，覆盖媒体阶段（ingress/reassembly/decode/render）、会话阶段（connected/reconnecting/stable-serving）与关键计时/计数（media progress、decode/keyframe 时间戳、local reset/keyframe request/nack skip 窗口计数）。
- Decision: 先以“采集与聚合不改默认行为”方式并入主线，下一步再将 reconnect fallback 的触发条件切换为快照门控，确保与现有 recovery 合同测试逐步对齐。
- Date: 2026-04-09 | Status: completed
- Update: 阶段 3 完成：`RtcSessionPolicy` 已将 `RecoveryObservationSnapshot` 接入 reconnect fallback 门控（当前仅作用于 `TransportAwaitRecoveryKeyframe -> RequestReconnectCandidate` 路径），并新增快照门控单测：`recovery_observation_snapshot_allows_transport_await_reconnect_when_local_self_healing_exhausted`、`recovery_observation_snapshot_blocks_transport_await_reconnect_when_keyframe_window_not_exhausted`。
- Update: reconnect settled 关键帧保护完成：`request_keyframe_after_reconnect_settled` 新增在飞/冷却判定，并新增运行时单测：`reconnect_settled_keyframe_is_deferred_when_keyframe_is_already_in_flight`、`reconnect_settled_keyframe_is_deferred_during_cooldown_window`。
- Date: 2026-04-09 | Status: completed
- Update: 阶段 4 完成：`diagnostics/stats::build_latest_decision_summary()` 引入恢复动作家族语义，输出 `local_decoder_maintenance:*` 与 `network_session_recovery:*`；补充 `latest_decision_summary_marks_local_decoder_maintenance_family` 等测试，确保面板/日志可直接区分本地维护动作与会话恢复动作。
- Update: 旧口径清理：`recovery/runtime_state` 将 `decoder-reset-window` 更名为 `local-maintenance-window`，并将 `requestDecoderReset` 的 `recovery_failure_cost` 从 `high` 收敛为 `medium`，避免继续把 decoder reset 记为网络会话升级动作。
- Date: 2026-04-10 | Status: in-progress
- Update: 补充 `video_source/timeline` 的“旧 reorder debt 退场”最小修复：当出现更高 `clean anchor`（`on_clean_keyframe_submitted`）时，允许退休早于 anchor 的硬级 `ReorderPending` debt；当链路已进入稳定 continuation（稳定窗 + clean streak）时，也允许退休早于当前 continuation 且达到最小年龄阈值的硬级 `ReorderPending` debt，避免旧 debt 长期占用 `has_pending_gap_risk()` 把链路卡在 `repairing/recovering`。
- Update: 同步收紧 clean-anchor 后的 soft reentry 预算语义：`gap-reorder-pending / nack-candidate / repair-in-flight / resolved` 这类观测只借用保护窗口，不再提前消耗 continuation submit budget；预算只在真正的 delta 重入/过期软化时消费，避免“关键帧已到，但后续几帧 non-IDR continuation 还没开始建链就先被观测流量耗尽预算”。
- Update: 新增 `timeline` 回归测试覆盖两条退场路径与“新 debt 不误退休”约束，确保修复不放宽当前活跃 debt 的硬语义。
- Update: 新增 soft reentry 回归测试，锁住“观测不耗预算、预算只被实际恢复动作消费”的语义，确保 clean keyframe 后的 continuation 建链窗口仍然可用。
- Risk/Blocker: 当前环境仍缺少 `cmake`/`pkg-config`，`cargo test -p xbxengine transport::rtc::stream::video_source::timeline::tests` 在 `audiopus_sys`/`ffmpeg-sys-next` 构建阶段失败；本轮仅完成代码与测试补充，待依赖就绪后复跑验证。
- Date: 2026-04-10 | Status: in-progress
- Update: 本轮继续推进“恢复契约统一”落地：引入 `Recovered = !ingressWaiting && !transportAwaitUnresolved && mediaHealthyBaseline` 的跨层硬门，并在 owner/coordinator/session 统一使用，避免并行判据漂移；旧标签 `bootstrapInFlight` 已统一替换为 `recoverySustaining`。
- Risk/Blocker: `video_source::timeline.test` 仍存在部分历史断言与新阶段语义不一致（`Healthy` vs `SustainingRecovery`），需要进一步清理后再将本 RFC 的 Completion 切换为已完成。
