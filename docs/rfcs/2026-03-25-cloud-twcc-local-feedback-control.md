# 云游戏 TWCC 本地可控反馈链路 RFC

## Status

- Completion: 未完成
- Current State: in-progress
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 最新云游戏真实 trace 显示：SDP 已协商 `transport-cc`，BWE 目标也已拉到 `25Mbps`，但 `latest_video_twcc_observation` 长时间为空，实际视频码率停在 `8~9Mbps`。
- 当前业务代码只在 `RTCMessage::RtcpPacket` 入站路径消费 `TransportLayerCc`，观测和策略都隐含依赖“收到别人发来的 TWCC”；这与 recv-side 应由本地生成并发送 feedback 的 ownership 不一致。
- 用户要求 TWCC feedback 必须由本地显式控制，并且观测口径继续统一，不能再混淆“本地发出”和“被动观察到的 RTCP”。

## Goal

- 显式接管 xbxengine 的 TWCC feedback 生成链路，不再依赖隐式默认 interceptor 行为。
- 让 TWCC feedback 发送间隔受 runtime config 控制，并把“本地发出的 feedback”直接写入统一 runtime stats/trace。
- 修正观测语义，避免错误地把非本地 ownership 的 TWCC 事件显示成 `twccFeedbackSent`。
- 给 runtime/trace 增加可验证的 build fingerprint 与 TWCC 分段断点事件，避免再次出现“新补丁是否真的运行”无法证明的问题。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/connection/*` 的 interceptor registry 与本地 TWCC 观察器
  - `crates/xbxengine/core/src/api/backend.rs`、`crates/xbxengine/protocol/src/runtime.rs`
  - `crates/xbxengine/core/src/diagnostics/*`、`src-tauri/src/mods/xbxengine/trace_projection.rs`
  - `src-tauri/build.rs`、`src-tauri/src/shell/mod.rs`、`src-tauri/src/mods/xbxengine/runtime_state.rs`
  - `docs/project-task.md`
- Out of scope:
  - BWE 规则进一步调参
  - 新一轮真实 runtime trace 采集与线上效果复盘
  - 前端性能面板新增专门的 TWCC source 展示控件

## Plan

1. 显式接入受控 interceptor registry，并把 TWCC feedback interval 绑定到 runtime config。
2. 新增本地 TWCC 出站观察器，把本地 feedback 直接写入 runtime stats。
3. 给 TWCC observation 补 `source`，统一 trace/DTO 语义，并避免 remote RTCP 覆盖本地主线观测。
4. 给 runtime trace / statsSnapshot / observabilitySnapshot 补 build fingerprint、`actualVideoBitrateSource`、`twccObservationState` 与 `rtcBuilderConfigured` / `twccRemoteStreamBound` / `twccInboundExtensionSeen|Missing`。

## Validation

- [x] `cargo check -p xbxengine`
- [x] `cargo check -p xbxrc`
- [x] `cargo test -p xbxengine diagnostics::stats --lib`
- [x] `cargo test -p xbxengine session::policy --lib`
- [x] `cargo test -p xbxengine create_raw_offer_comes_from_real_rtc_peer_connection --lib`
- [x] `cargo test -p xbxrc trace_projection --lib`
- [x] `cargo test -p xbxengine parse_twcc_binding_info --lib`
- [x] `cargo test -p xbxengine controlled_twcc_controller_emits_local_feedback_observation --lib`
- [x] `cargo test -p xbxengine twcc_feedback --lib`
- [x] `cargo test -p xbxengine request_target_remb_kbps_sends_goog_remb_rtcp --lib`
- [x] `cargo test -p xbxengine transport::rtc::connection::transport_metrics --lib`
- [x] `cargo test -p xbxengine target_remb_is_refreshed_periodically_after_initial_request --lib`
- [ ] 使用新的 runtime trace 确认存在稳定的 `twccFeedbackSent(source=local-feedback)` 样本

## Risks

- `RTCPeerConnection` 的 interceptor 类型从默认 `NoopInterceptor` 变为受控链，需要保持 connection 模块的类型联动稳定。
- 当前仍保留入站 `TransportLayerCc` 观察；若未来出现双向 TWCC，必须继续审视是否需要拆成独立的 local/remote 字段。

## Progress

- [x] Step 1: 已显式构建 NACK + RTCP report + TWCC receiver + 本地 TWCC observer 的 registry，并接入 `with_interceptor_registry(...)`
- [x] Step 2: 已将 `feedback_interval_ms` 绑定到 `TwccReceiverBuilder::with_interval(...)`，并在出站 RTCP `poll_write` 中记录本地 TWCC
- [x] Step 3: 已为 TWCC observation 新增 `source`，trace 中仅对 `local-feedback` 发 `twccFeedbackSent`，remote 路径改为 `twccFeedbackObserved`
- [x] Step 4: 已为 Tauri runtime trace 增加 build fingerprint，并把同一指纹注入 `statsSnapshot` / `observabilitySnapshot`
- [x] Step 5: 已新增结构化断点事件：`rtcBuilderConfigured`、`twccRemoteStreamBound`、`twccInboundExtensionSeen`、`twccInboundExtensionMissing`
- [x] Step 6: 已新增 `actualVideoBitrateSource` / `twccObservationState`，并让性能面板直接展示来源与状态
- [x] Step 7: 已清理主路径中的 `legacy frame pipeline` 误导性命名，并让 build fingerprint 同时携带默认值与会话实际生效的 feedback interval
- [x] Step 8: 已在 `poll_read()` 读循环补 `TWCC fallback bind`，即使 interceptor 没回调 `bind_remote_stream`，首个 RTP 到达后也会基于 remote SDP 补发 `twccRemoteStreamBound`，并继续产出 `twccInboundExtensionSeen|Missing`

## Execution Notes

- Date: 2026-03-25 | Status: in-progress
- Update: 已把 TWCC feedback 生成链路切到显式 registry，并让本地出站 `TransportLayerCc` 直接进入 runtime stats / trace 主线。
- Decision: `latest_video_twcc_observation` 继续作为 BWE 主输入，但当已有 `local-feedback` 时，不再允许 `remote-rtcp` 覆盖该主线观测。
- Update: 已补 runtime build fingerprint 和 TWCC 分段结构化事件；下一份 trace 即使仍未出本地 feedback，也能直接看出断在 builder、stream bind、RTP extension 还是 local feedback 产出阶段。
- Update: 已继续做诊断收口：`buildFingerprint` 不再只显示默认 feedback interval，而会额外附带当前会话实际生效值；现行 Rust RTC 主路径中的 `legacy frame pipeline` 命名也已改为中性主线路径命名，避免误判为旧代码绕路。
- Update: 针对最新 trace 中“`rtcBuilderConfigured` 已出现、媒体已 steady、但始终没有 `twccRemoteStreamBound`”的断点，已在 `connection/data_channel.rs::drain_peer_reads_core()` 增加 fallback 绑定逻辑：当前实际 RTP 读循环一旦收到首包，就会从 remote answer SDP 提取 `transport-cc extmap / rtcp-fb / codec`，写入 `twccRemoteStreamBound`，并继续检查该 SSRC 的 RTP 扩展是否存在，避免诊断再次卡在 builder 后静默无事件。
- Update: 结合 `runtime-trace-1774415000917.jsonl` 继续排查后，已确认最新断点从“未绑定 remote stream”前移到“video 已绑定但未产出 local feedback”。本轮已将 `connection/twcc_feedback.rs::parse_twcc_binding_info_from_answer_sdp()` 改为按 `m=` section 和 payload type 精确解析 `extmap / rtcp-fb / rtpmap`，避免 audio/video 同时声明 `transport-cc` 时误用跨 media section 的 ext id；并新增 audio/video ext id 不同的回归测试，防止再次把 video TWCC 绑定到错误扩展号。
- Update: 已继续按分层收口 TWCC ownership/diagnostics，而没有把媒体归因逻辑扩散到 session/bwe/UI 层：`connection/twcc_feedback.rs` 现已把 inbound extension 计数改为 per-SSRC，避免 audio 污染 video 诊断；无 `receiver_id` 映射的本地 feedback 也改为进入有上限的待发送队列并追加显式日志，避免静默丢弃。与此同时，`diagnostics/stats.rs` 已将 `twccObservationState` 收紧为只由 video 相关证据推进，`connection/transport_metrics.rs` 也不再让 `remote-rtcp` TWCC 观测回退复用本地 video transport bitrate，避免继续把 remote RTCP 误写成 local video 吞吐证据。
- Decision: `TransportCommand::SetTargetRembKbps` 的真实语义已明确为 receiver-side `goog-remb`。本轮已将该命令下沉到 `RtcConnectionService::request_target_remb_kbps()`，由 connection 层通过 `ReceiverEstimatedMaximumBitrate` 直接写 RTCP；若 video receiver/SSRC 尚未完成真实绑定，则先在 connection 层排队，待后续 `pump()` 中出现 video feedback 目标后自动补发。这样仍保持“session 只产生命令，connection 负责网络副作用”的边界，不把码率控制硬塞进 control/data channel。
- Update: 后续真实 trace 已证明 `twccFeedbackSent(source=local-feedback)` 与 `rtcTargetRembQueued` 都已出现，但同时暴露出两个剩余问题：1) `rtcTargetRembQueued` 尚未升为 `rtcTargetRembRequested`，说明 video feedback 目标在 connection 层缺少稳定锚点；2) `local-feedback` 的 `observedByteCount/receiveBitrateKbps` 仍退化成接近 packet-count 语义。针对这两点，本轮已继续收口：`connection/twcc_feedback.rs` 现会稳定缓存当前 video receiver/media SSRC 作为 REMB 目标，而不是每次从瞬时 `remote_twcc_streams` 重新推导；`connection/transport_metrics.rs` 也新增了更强的字节估算保护，优先用 transport bitrate * interval 反推区间字节数，并拒绝退化到 `observedPacketCount == observedByteCount` 的异常口径。
- Update: 继续顺着代码时序审查后，已确认 `register_track_open()` 之前只记录 `track_id -> receiver_id`，但不会把这个新 `receiver_id` 回填到已经存在的 video TWCC binding 中；这会导致“先收到 video RTP 建 binding、后收到 OnTrack 事件补 receiver”这一真实时序下，`preferred_video_feedback_target()` 仍然返回空值，REMB 长期停在 `Queued`。本轮已为 `register_track_open()` 增加 backfill：当 track open 到来时，会回填已有 binding 的 `receiver_id`，并同步刷新稳定的 `preferred_video_receiver_id/preferred_video_media_ssrc`。
- Update: 参考旧版 Rust `webrtc` 主线里 `spawn_video_track_stats_loop()` 的行为，本轮已在当前 rtc connection 层补回“周期性 REMB 持续刷新”。实现上仍保持分层：session policy 只计算 target/reason，connection 层在初次 `requested` 之后会按 feedback interval 周期性 refresh 当前 `active_target_remb_kbps`，并在 transport 压力（gap/nack/waitingKeyframe）下加快刷新频率。这样既保留当前 sans-io 架构，也把旧主线最关键的持续 RTCP 刺激行为迁回来了。
- Decision: 已移除 `offer_policy.rs` 中附加到 SDP 尾部的 `a=x-xbx-phase1-rtc` / `a=x-xbx-target-type` / `a=x-xbx-offer-profile` 私有属性。这组三方外不可识别的自定义字段不属于 Xbox 云端协商契约，也不在 Better xCloud 的兼容做法里；后续诊断需要依赖 trace/runtime stats，而不是往标准 SDP 中混入私有标记。
- Update: 已继续把标准 SDP 字段内容对齐到 Xbox 云端兼容口径，而不是只追求“字段存在”：1) `CodecPreference::H264High` 进入 negotiation plan 后，现按 Better xCloud 兼容语义落为 `4d` family 偏好，而不是 `64`；2) `rtc` 自有 H264 codec family 注册顺序已调整为 `4d -> 42e -> 64 -> 420`，避免把 `64` 放成首选；3) `sdp/policy.rs` 在最终 offer patch 后会对重复的标准 `a=rtcp-fb:*` 行做去重，避免继续向 Xbox 云端发送重复的 `goog-remb/transport-cc` 反馈声明。
- Risk/Blocker: 仍需下一份真实 trace 验证本地 TWCC feedback 确实持续发出且能驱动实际码率回升。
