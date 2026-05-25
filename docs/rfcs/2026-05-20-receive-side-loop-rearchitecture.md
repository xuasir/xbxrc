# 基于 `rtc` 的低延迟优先 Receive-side Loop 重构 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: Phase B 完成（ReceiveEngine 接线 + purge）；Cloud/Home trace 待人工回归
- Current State: completed
- Owner: agent
- Last Updated: 2026-05-20

## Background

- 当前 Rust WebRTC 主线已经固定在 `rtc = 0.9.0` 及配套 crate 上，见 [`crates/xbxengine/core/Cargo.toml`](../../crates/xbxengine/core/Cargo.toml)。
- 当前主线虽然基于 `rtc` 建连，但接收侧已经自建了大半条 `packet -> frame -> bootstrap -> recovery -> decode -> host present` 闭环，关键模块包括：
  - [`connection/builder.rs`](../../crates/xbxengine/core/src/transport/rtc/connection/builder.rs)
  - [`connection/twcc_feedback.rs`](../../crates/xbxengine/core/src/transport/rtc/connection/twcc_feedback.rs)
  - [`connection/service.rs`](../../crates/xbxengine/core/src/transport/rtc/connection/service.rs)
  - [`stream/video_source/source.rs`](../../crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs)
  - [`stream/video_source/timeline.rs`](../../crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.rs)
  - [`stream/nack_scheduler.rs`](../../crates/xbxengine/core/src/transport/rtc/stream/nack_scheduler.rs)
  - [`policy/scheduling.rs`](../../crates/xbxengine/core/src/transport/rtc/policy/scheduling.rs)
  - [`stack/transport_session.rs`](../../crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs)
- 现有多轮任务已经证明，这条主线不是简单的 transport glue，而是长期在修补同一组接收侧闭环问题：
  - `bootstrapMissingIdr`
  - `continuationAcceptedWhileAwaitingIdr`
  - `videoRtcpFeedbackTargetPending`
  - `transportAwaitRecoveryAnchor`
  - `cleanAnchorCommitted`
- 用户方向已明确：
  - 不切换出 `rtc`
  - 最终仍要保有修改权
  - 目标是**低延迟优先的 receive-side loop**
  - 但现状总被关键帧闭环、IDR 缺失、continuation-only、feedback target 问题打穿
- 浏览器标准 WebRTC 的参考价值不在于某个阈值，而在于 receive-side ownership 边界。官方接收侧主线可参考：
  - `RtpVideoStreamReceiver2`
  - `NackRequester`
  - `PacketBuffer / H26xPacketBuffer`
  - `H264SpsPpsTracker`
  - `RtpFrameReferenceFinder`
  - 相关 jitter / NACK / keyframe request 代码
  - 参考源码：
    - [rtp_video_stream_receiver2.cc](https://webrtc.googlesource.com/src/%2B/refs/heads/main/video/rtp_video_stream_receiver2.cc)
    - [nack_requester.cc](https://webrtc.googlesource.com/src/%2B/eba683130070bc202058580b657393f937ca8a8c/modules/video_coding/nack_requester.cc)
    - [h264_sps_pps_tracker.cc](https://webrtc.googlesource.com/src/%2B/f6f642d7fa49f40eda902ae8fc4eae8cb7d427b7/modules/video_coding/h264_sps_pps_tracker.cc)
    - [jitter_buffer.h](https://webrtc.googlesource.com/src/%2B/42d8c93ec351b68554825b58a3dc6525a7dc84da/modules/video_coding/jitter_buffer.h)
- 本 RFC 的判断是：
  - 当前主要问题不是“阈值不够准”，而是 **pre-decode receive-side ownership 过宽且跨层外提过多**。
  - 想同时拿到标准 WebRTC 的稳定性和本仓库想要的低延迟，必须**彻底重构 receive-side loop**，而不是继续在现有 recovery 叙事上修补。

## Goal

- 在保留 `rtc` 连接主线和修改权的前提下，重构接收侧主线，使其：
  - 在 decode 前具备接近标准 WebRTC receiver 的稳定性
  - 在 decode 后继续保持本仓库的低延迟优先策略
- 明确切断以下旧耦合：
  - pre-decode decodability 与 post-decode host/display 价值判断
  - NACK / keyframe request 与全局 recovery owner 叙事
  - feedback target 可用性与全局恢复原因
- 删除旧兼容层，而不是围绕旧状态机做向下兼容。

## Scope

- In scope:
  - `transport/rtc` 接收侧 pre-decode 主线重构
  - `packet -> frame -> bootstrap -> nack -> keyframe request -> decode gate` 的新职责边界
  - decode 后 latest-only / host present 低延迟主线与前半段的解耦
  - 相关 trace / diagnostics / tests 的重写
- Out of scope:
  - 替换 `rtc`
  - 引入第二套 runtime 或浏览器端实现
  - 保持旧 reason label、旧 recovery 状态名、旧 DTO 兼容
  - 保留旧 receive-side loop 的运行时双轨共存

## Hard Decisions

### 1. 不做向下兼容

- 不保留 `transportAwaitRecoveryAnchor` 的主叙事地位。
- 不保留旧的 `cleanAnchorCommitted` 对 pre-decode 的控制权。
- 不保留 `videoRtcpFeedbackTargetPending` 作为 recovery reason。
- 不保留旧的 `WaitKeyframe / ContinuationOnly / owner health` 跨层解释链。
- 不保留 `SchedulingPolicyEngine` 对 picture recovery 动作的审批语义。

### 2. receiver-local ownership 收回模块内部

- decode 前只允许 receiver-local 模块回答：
  - 当前包是否应缓冲
  - 当前 frame 是否可组装
  - 是否值得发 NACK
  - 是否必须请求关键帧
  - 当前 access unit 是否可喂 decoder
- host present、display stability、latest-only、mailbox overwrite 不得参与上述判断。

### 3. 低延迟继续保留，但换位置

- decode 前只保留**窄窗口、短等待、快 fallback** 的低延迟策略。
- decode 后保留 **latest-only、stale drop、host deadline、display pacing** 的低延迟策略。
- 不再在 pre-decode 阶段用 display/host 价值判断去驱动 decodability 和 keyframe request。

## Design

### 1. 新架构

新的接收主线拆成四层：

1. `RtcTransportCapability`
2. `RtcReceiveCore`
3. `DecodeIngressAdapter`
4. `PostDecodeLatencyController`

#### 1.1 `RtcTransportCapability`

职责：

- 承载 `rtc` 连接、track、RTCP 发送、TWCC feedback、receiver/media SSRC 绑定
- 提供窄接口：
  - `send_nack(seqs)`
  - `send_pli()`
  - `send_fir()`
  - `send_remb(kbps)`
  - `video_feedback_ready()`
  - `latest_rtt_ms()`

约束：

- 只暴露 transport capability，不再暴露 recovery reason。
- `feedback target unavailable` 只作为能力失败返回，不再进入全局恢复状态机。
- `connection/service.rs` 与 `twcc_feedback.rs` 保留为 transport plane，不再持有 receive-side owner 语义。

#### 1.2 `RtcReceiveCore`

职责：

- 接收原始 RTP / RTX
- 包重排与 packet buffer
- frame assembly
- H264 SPS/PPS/IDR bootstrap
- 本地 NACK requester
- 本地 keyframe requester
- decode gate

输入：

- RTP/RTX packet
- RTT sample
- transport capability ready/unready
- decoder hard reset acknowledgement

输出：

- `AssembledFrameReady`
- `RequestNack`
- `RequestKeyframe`
- `ReceiverState`
- `ReceiverObservation`

#### 1.3 `DecodeIngressAdapter`

职责：

- 把 `RtcReceiveCore` 产出的可解码 access unit 喂给 decode actor
- 只处理 decoder safety、decode reset、解码失败回灌
- 不再回写 display/host 语义到 receive core

#### 1.4 `PostDecodeLatencyController`

职责：

- latest-only mailbox
- presentation role
- host present deadline
- stale frame drop
- local visible / displayed 统计

约束：

- 它可以继续激进低延迟。
- 但它不再反向决定 pre-decode 的 `NACK / PLI / FIR / wait-keyframe` 逻辑。

### 2. `RtcReceiveCore` 里的最小状态机

旧设计里的 `Healthy / Broken / Recovering / SustainingRecovery / Stalled` 过宽。  
新设计只保留 receiver-local 状态：

- `Priming`
- `Receiving`
- `Repairing`
- `WaitingKeyframe`

状态含义：

- `Priming`
  - 还未建立可解码上下文
- `Receiving`
  - 正常收包、能产出可解码 frame
- `Repairing`
  - 存在 seq gap，正在等待重排或本地 NACK
- `WaitingKeyframe`
  - 当前链已不可经济修复，必须等待新 keyframe

约束：

- 不引入 `AwaitAnchor / ContinuationOnly / DisplayStable` 这类跨层状态。
- `WaitingKeyframe` 只表达 receiver-local decodability，不再表达全局恢复 owner。

### 3. Packet Buffer 与组帧

当前 `timeline.rs` 既维护 gap，又维护 chain debt，又维护 clean-anchor/building phase，职责过宽。  
重构后：

- `PacketBuffer`
  - 只维护 seq continuity、RTX reinject、reorder window、frame packet membership
- `FrameAssembler`
  - 只维护 frame completeness、missing packet span、组帧结果
- 不再保留旧 `VideoTimelineState` 作为全局事实源

删除项：

- `chain_debt_reason`
- `stable_recovery_started_at_ms`
- `recovery_chain_building_deadline_ms`
- `clean_anchor_ingress_observed_at_ms`
- `pending_clean_anchor_rtp_ts`
- 以及围绕这些字段存在的 gate 逻辑

替代原则：

- packet/frame continuity 是 receiver-local 事实
- decode 后播放稳定性是 post-decode 事实
- 两者不再共享同一主状态对象

### 4. H264 Bootstrap 改成 Tracker-first

保留并扩大当前已经存在的 parameter-set salvage 思路，见 [`source.rs`](../../crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs)。

新规则：

1. 建立 `H264BootstrapTracker`
   - 缓存 committed SPS/PPS
   - 识别 IDR 是否携带或可由缓存参数集补全
2. 对 IDR：
   - 缺 SPS/PPS 先尝试 prepend 缓存参数集
   - 仍不可解则直接进入 `WaitingKeyframe` 并请求 keyframe
3. 对非 IDR continuation：
   - 只有在 tracker 明确认为当前参考链可解时，才允许继续喂 decoder
   - 不再因为“当前仍有画面输出”就扩大 continuation admission
4. 对带丢包 keyframe：
   - 直接本地丢弃并请求 keyframe
   - 不再通过全局 recovery 叙事升级

删除项：

- `continuationAcceptedWhileAwaitingIdr` 作为全局 blocker/owner
- `cleanAnchorCommitted` 作为 continuation admission 前提

### 5. NACK 改成 Receiver-local Requester

当前 [`nack_scheduler.rs`](../../crates/xbxengine/core/src/transport/rtc/stream/nack_scheduler.rs) 混入了：

- frame importance
- playout deadline
- host/display 语义投影
- value tier / chain broken 解释

重构后保留的输入只允许有：

- seq gap
- frame packet span
- RTT
- reorder/OOS 统计
- keyframe/reference/delta 的接收器本地优先级

删除项：

- 基于 host/display 状态的 NACK admission
- `frame_playout_deadline_at_ms` 参与 pre-decode 的主裁决
- `SkippedChainBroken` 这类把 receiver-local miss 直接抬成全局坏链的解释
- `firstAttemptSurvivalWindowMs` 与 host side 价值合同耦合

新的 NACK 原则：

1. 先等一个很小的 reorder 窗
2. 再发首个 NACK
3. 最多允许一次补充重试
4. 超过 receiver-local 经济窗口后，直接请求 keyframe

默认低延迟目标：

- Home/LAN:
  - reorder wait: `3~6ms`
  - first NACK: `4~8ms`
  - retry: `8~16ms`
  - keyframe fallback: `35~60ms`
- Relay/WAN:
  - reorder wait: `6~12ms`
  - first NACK: `8~15ms`
  - retry: `20~40ms`
  - keyframe fallback: `80~140ms`
- Cloud:
  - reorder wait: `8~16ms`
  - first NACK: `12~22ms`
  - retry: `40~80ms`
  - keyframe fallback: `160~260ms`

这些值只服务 receiver-local decodability，不再为 host present 或 display stabilize 服务。

### 6. Keyframe Request 改成接收器局部动作

旧设计里 `PLI/FIR` 经过 `SchedulingPolicyEngine -> SessionActor -> TransportCommand` 审批，导致 picture recovery 与全局 owner 耦合过深。

重构后：

- `RtcReceiveCore` 直接产出 `RequestKeyframe { pli | fir }`
- `RtcTransportCapability` 负责执行
- 执行失败只回给 receive core 本地计时器，不再进入全局 recovery 叙事

动作原则：

- 默认主路径是 `PLI`
- `FIR` 只在以下情况下升级：
  - 连续 `PLI` 无响应
  - 远端画像明确适合 `FIR`
  - 或 transport 明确无法走正常 picture loss 路径

删除项：

- `transportAwaitRecoveryAnchor`
- `coalesced:keyframeInFlight`
- `keyframe family gate`
- `localRepairPending` 持有 picture recovery owner 的旧语义

### 7. `feedback target` 降级为 transport capability

当前 `videoRtcpFeedbackTargetPending` 会被系统读成恢复原因。  
重构后明确改为：

- `FeedbackReady`
- `FeedbackWarming`
- `FeedbackUnavailable`

用途只剩：

- 能不能发出 NACK / PLI / FIR
- 若不能，是否暂时等待或直接切更重动作

禁止：

- 将其投影为 `transportAwaitRecoveryAnchor`
- 将其投影为 owner health
- 将其投影为全局恢复状态

### 8. decode 后低延迟主线保留

以下能力保留并继续强化：

- latest-only mailbox
- post-decode stale drop
- presentation role
- host present deadline
- local drop over queueing

但它们统一留在 decode 后：

- [`2026-04-24-post-decode-latest-only-mailbox-convergence.md`](./2026-04-24-post-decode-latest-only-mailbox-convergence.md)
- [`2026-04-30-post-decode-value-aware-latest-mail-alignment.md`](./2026-04-30-post-decode-value-aware-latest-mail-alignment.md)

硬边界：

- decode 后可以很激进地低延迟
- decode 前不再用这些语义去影响 NACK / keyframe / bootstrap

## Keep / Delete

### Keep

- `rtc` 连接主线
- `MediaEngine` / codec / SDP / ICE / DTLS / SRTP 基础设施
- TWCC/RTCP feedback 写出能力
- H264 parameter-set salvage 思路
- decode 后 latest-only / host mailbox / present deadline
- 现有 runtime stats 基础设施与 trace 写出口

### Delete Or Collapse

- `transportAwaitRecoveryAnchor` 主状态
- `cleanAnchorCommitted` 对 pre-decode 的 gate
- `VideoTimelineState` 的链债务/恢复建链/稳定期主语义
- `feedbackTargetPending` 作为 recovery reason
- `SchedulingPolicyEngine` 对 picture recovery 的审批职责
- 基于 host/display 的 pre-decode NACK admission
- `continuationAcceptedWhileAwaitingIdr` 作为 owner/blocker 叙事
- decode 前 `value_tier / risk_tier / action_stage / evidence_scope` 那套跨层恢复解释

## Plan

1. 先切 transport capability 与 receive core 边界，冻结旧 receive-side loop 的新增修补。
2. 新建 `RtcReceiveCore`，完成 packet buffer / frame assembler / H264 tracker / local NACK requester / local keyframe requester。
3. 切断 `timeline / session / scheduling` 对 pre-decode picture recovery 的控制权。
4. 接上 decode ingress 与现有 post-decode latency controller，完成单线切换。
5. 删除旧状态机、旧 reason label、旧测试合同，重写 diagnostics 和 trace。

## Validation

- [ ] Cloud / Home / Relay 三类链路上，`bootstrapMissingIdr` 停留时长显著下降
- [x] `continuationAcceptedWhileAwaitingIdr` / `receiverLocalContinuation` 不再作为全局 owner/blocker 驱动 session PLI
- [x] `videoRtcpFeedbackTargetPending` 不再进入 recovery 叙事
- [ ] NACK/PLI/FIR 请求频率下降且更集中在 receiver-local 逻辑
- [ ] 首帧获取成功率、恢复后重新出图成功率高于当前主线
- [ ] decode 后 latest-only / host present 低延迟体感不倒退
- [x] session/coordinator 不再下发 `RequestPli`/`RequestFir`；旧 transport-await 集成测试已删除
- [ ] Cloud/Home trace 指标对比（需人工回归）

## Risks

- 这是彻底重构，不是局部补丁；短期内会打碎大量现有测试与 trace 口径。
- 如果 receiver-local 边界没有收紧到底，旧语义会继续回流，最后得到“新名字下的旧系统”。
- pre-decode 低延迟窗口如果收得过猛，会放大坏网下的 keyframe 请求频率；如果放得过宽，又会回到当前拖沓问题。
- decode 后低延迟与 decode 前稳定性解耦后，某些旧“恢复完成”事件名将失去意义，需要接受前端/诊断层口径重写。

## Progress

- [x] Step 1: 已完成现有 receive-side loop 与浏览器标准 WebRTC 接收侧的边界比对
- [x] Step 2: 已明确“保留 `rtc` + 重构 receive-side ownership”作为主方向
- [x] Step 3: 已拆出 `RtcTransportCapability` 与 `RtcReceiveCore` 模块边界（receive 仍委托既有组帧实现）
- [x] Step 4: 已完成 media_pipeline 单线切换；scheduling PLI/FIR 审批已切断；旧集成测试合同已移除/忽略
- [x] Step 5: Report 见 [`reports/2026-05-20-receive-side-loop-rearchitecture.md`](../reports/2026-05-20-receive-side-loop-rearchitecture.md)
- [x] Step 6: `stream/video_source/` 物理迁入 `receive/`；`trace_ledger.rs`；生产路径剔除 cloud NACK admission / pre-decode clean-anchor 状态机；`cargo build -p xbxengine --lib` 绿

## Implementation Notes (WebRTC 三步对齐)

- Date: 2026-05-25 | **解码/bootstrap**：`resolve_inspection_admission` 在 `WaitingKeyframe` / `bootstrapMissingIdr` 下禁止 `receiverLocalContinuation`；decode 侧禁止对 missing-IDR continuation 开窗；硬件 nominal no-output 先 VT reset 再 software fallback。
- Date: 2026-05-25 | **关键帧恢复**：NACK escalation / blocking admission 强制 PLI（`bootstrapMissingIdr` 不再走 soft）；trace 增加 `keyframeRequestSent`。
- Date: 2026-05-25 | **呈现合同**：macOS `resolve_video_pipeline_plan` 按 surface 路由——`MacOsCvPixelBuffer` → NativeDirect，CPU 面 → GpuDirect/wgpu。

## Execution Notes

- Date: 2026-05-20 | Status: Phase B + 目录对齐完成；`ingress_loop`/`nack_maintenance` 仍 >500 行，巨石拆分与分层测试 follow-up
- Date: 2026-05-20 | Status: Phase B completed（ReceiveEngine 接线、NACK/timeline 瘦身、reason purge）；trace 回归待人工
- Date: 2026-05-20 | Phase 0: `cargo test -p xbxengine --lib transport::rtc` 基线 804 passed / 3 failed（既有失败，非本重构引入）；冻结旧 receive-side 修补
- Update: 用户明确要求“改就改彻底，不考虑向下兼容”。本 RFC 因此不再讨论替换 `rtc`，而是把执行方向固定为“基于 `rtc` 重做 receive-side loop”。
- Decision: 旧的“Rust/WebRTC 成熟接收闭环重评估”RFC 只保留为背景文档，不再作为执行依据。
- Decision: 新主线采用“receiver-local decodability loop + post-decode low-latency loop”二段结构。
- Decision: 当前 `transportAwaitRecoveryAnchor / cleanAnchorCommitted / feedbackTargetPending / owner health` 这条 pre-decode 跨层链路将在重构中删除。
- Risk/Blocker: 旧 trace/diagnostics/test 合同覆盖面大，执行期需要接受一次明显的命名和结构断代。
