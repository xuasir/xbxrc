# Rust/WebRTC 成熟接收闭环重评估 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: planned
- Owner: agent
- Last Updated: 2026-05-20

## Background

- 当前 Rust 连接层已经建立在 `rtc = 0.9.0` 及其配套 `rtc-rtp / rtc-rtcp / rtc-media / rtc-shared / rtc-stun / rtc-turn` 上，见 [`crates/xbxengine/core/Cargo.toml`](../../crates/xbxengine/core/Cargo.toml)。
- 当前 `transport/rtc` 主线并不是只在“调几个上层阈值”，而是在事实上拥有一整套自建接收端媒体闭环，关键收口点集中在：
  - [`connection/twcc_feedback.rs`](../../crates/xbxengine/core/src/transport/rtc/connection/twcc_feedback.rs)
  - [`connection/service.rs`](../../crates/xbxengine/core/src/transport/rtc/connection/service.rs)
  - [`stream/video_source/source.rs`](../../crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs)
  - [`stream/video_source/timeline.rs`](../../crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.rs)
  - [`stream/nack_scheduler.rs`](../../crates/xbxengine/core/src/transport/rtc/stream/nack_scheduler.rs)
  - [`policy/scheduling.rs`](../../crates/xbxengine/core/src/transport/rtc/policy/scheduling.rs)
  - [`stack/transport_session.rs`](../../crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs)
- 当前视频主轨是 `Recvonly`，见 [`connection/builder.rs`](../../crates/xbxengine/core/src/transport/rtc/connection/builder.rs)。主问题不在发送端 pacer，而在接收端 `packet -> frame -> bootstrap -> recovery -> decode -> host present` 闭环。
- 现有文档已经明确这条闭环存在大量自建能力与已知缺口，见 [`docs/isu/packet-to-frame-pipeline.md`](../isu/packet-to-frame-pipeline.md)：
  - sink/source 自己做包优先级、RTX reinject、NACK window、sample builder、H264 bootstrap gate；
  - 已知缺口包括时效性过滤、FU-A 后续分片优先级、OOS 乱序状态与 NACK 取消联动等。
- 近几轮 RFC 已经持续在修补同一条接收恢复闭环，而不是在稳定主线上做局部增强，见：
  - [`2026-04-08-scheduling-architecture-simplification.md`](./2026-04-08-scheduling-architecture-simplification.md)
  - [`2026-04-24-post-decode-latest-only-mailbox-convergence.md`](./2026-04-24-post-decode-latest-only-mailbox-convergence.md)
  - [`2026-05-12-transport-repair-and-recovery-semantic-unification.md`](./2026-05-12-transport-repair-and-recovery-semantic-unification.md)
  - [`2026-05-13-transport-await-anchor-simplification.md`](./2026-05-13-transport-await-anchor-simplification.md)
  - [`2026-05-14-dynamic-rtt-aware-recovery-timing.md`](./2026-05-14-dynamic-rtt-aware-recovery-timing.md)
- 用户反馈的关键前提修正成立：
  - 当前痛点不是“`rtc` primitives 能不能再往上搭一点”，而是“浏览器同环境稳定，而当前 native Rust 栈反复被 `关键帧闭环 / IDR 缺失 / continuation-only / feedback target pending` 打穿”。
  - 因此问题应从“库层成熟闭环”来重新评估，而不是默认继续把接收端恢复逻辑建设在当前自管链上。
- 外部库与浏览器现状（2026-05-20 复核）：
  - `webrtc-rs/webrtc` 的 docs.rs README 当前显示 `v0.17.1`，并明确：`v0.17.x` 是 Tokio 耦合实现的最终 feature release，只收 bugfix；master 继续转向基于 `webrtc-rs/rtc` 的 sans-io 架构。同时该 README 也写明项目“under active development”且应视为 early stage。[docs.rs/webrtc README](https://docs.rs/crate/webrtc/latest/source/README.md)
  - `webrtc-rs/webrtc` 的 `TrackRemote` API 仍然以 `read()/read_rtp()` 返回入站 RTP 包为主，这说明它能替你接管标准 WebRTC API 与 RTCP/ICE/DTLS/SRTP，但不会直接替你接管浏览器级的接收端 frame/decode pipeline。[TrackRemote 文档](https://docs.rs/webrtc/latest/src/webrtc/track/track_remote/mod.rs.html)
  - `str0m` README 明确支持 frame-level API 和 RTP-level API，但它自己的 feature matrix 也明确写出：相对 `libWebRTC`，它缺 `Adaptive Jitter Buffer`、capture、encode/decode、audio render、TURN、network interface enumeration。[str0m README](https://github.com/algesten/str0m)
  - `libdatachannel` README 明确其目标是以更轻量的 C/C++ 库直接连接 native app 与 browser，兼容 Chromium/Firefox/Safari，并支持 ICE/TURN/DTLS/SRTP/RTX 等媒体传输能力；Rust bindings 可用，最新 release 页面显示 `0.24.3` 发布于 2026-05-09。[libdatachannel README](https://github.com/paullouisageneau/libdatachannel)
  - Chromium/libwebrtc 的接收端并不是“RTP primitives + 用户自建恢复”，而是内建视频 jitter buffer / NACK list / keyframe request sender / retransmission waiting 这些闭环能力。官方代码中 `VCMJitterBuffer` 直接暴露 `UpdateRtt()`、`SetNackMode()`、`GetNackList(request_key_frame)`、`WaitForRetransmissions()` 等接口，并在缓冲区满时 `RecycleFramesUntilKeyFrame()`；官方 RTP 文档也明确把 RTX 作为带宽 probing 与冗余的一等路径，且在接收端不干扰媒体处理。[libwebrtc jitter buffer](https://webrtc.googlesource.com/src/%2B/42d8c93ec351b68554825b58a3dc6525a7dc84da/modules/video_coding/jitter_buffer.h) [RTP in WebRTC](https://webrtc.googlesource.com/src/%2B/HEAD/pc/g3doc/rtp.md)

## Problem Framing

- 当前真正要回答的不是“换不换 Rust WebRTC 库”，而是：
  - 我们要不要继续自己维护接收端恢复闭环？
  - 如果不想继续维护，候选库是否真的把 `packet -> frame -> jitter -> nack -> keyframe request -> decode gating` 一起接走？
- 这直接决定了评估轴必须拆成两层：
  1. `Transport/API maturity`
     - ICE / DTLS / SRTP / SDP / RTCP / RTX / TURN / browser interop
  2. `Receive-pipeline maturity`
     - adaptive jitter buffer
     - NACK list 与 keyframe request 的耦合
     - packet/frame continuity 管理
     - decode 前 bootstrap / continuity / playout hold 语义
- 如果一个候选只改善第一层，而第二层仍由本仓库自己维护，那么它并不能验证“浏览器为什么稳、native 为什么被打穿”这个核心假设。

## Goal

- 给出新的明确结论：如果目标是复用浏览器同类成熟闭环，应该优先验证哪一类库。
- 区分“transport/API 替换”与“receive pipeline 替换”两种完全不同的 spike。
- 避免继续在当前自建接收恢复闭环上投入大规模细修，除非已经明确放弃“借成熟栈止血”的方向。

## Scope

- In scope:
  - 当前 `transport/rtc` 主线中 transport 层与 receive pipeline 层的职责拆分
  - `webrtc-rs/webrtc`、`str0m`、`libdatachannel` 的成熟度与替换粒度比较
  - 必要时将 `libwebrtc-family wrapper` 作为对照组选项纳入方向评估
  - “浏览器为什么更稳”与“当前 native 栈为什么反复被 IDR/continuation 打穿”的对应关系
- Out of scope:
  - 立即启动全量替换
  - 在未验证假设前继续扩大当前接收恢复闭环
  - 改动浏览器端 render policy 或前端 UI 主线

## Plan

1. 先明确 spike 目标是“验证成熟接收闭环假设”还是“减少 transport/API 胶水”。
2. 若目标是验证浏览器同类稳定性，优先选能替掉 receive pipeline 的候选，而不是只替 transport。
3. 若必须保持 Rust-first，则接受“这不是浏览器闭环对位验证”，只做 transport/API 替换类 spike。
4. 用相同 cloud/home 目标与相同网络环境复测：IDR 获取成功率、continuation-only 停留时长、NACK->可解码帧恢复时长、feedback target pending 频率。

## Validation

- [x] 当前主链问题已按 `transport/API` 与 `receive pipeline` 两层重分层
- [x] 候选库的官方能力边界已核对
- [ ] 已定义“成熟闭环验证型 spike” 的最小成功标准
- [ ] 已定义“transport/API 替换型 spike” 的最小成功标准
- [ ] 已定义两类 spike 的退出条件与不可比边界

## Risks

- 把 `webrtc-rs/webrtc` 或 `libdatachannel` 只当作“更成熟 transport”接入，却继续沿用当前 `video_source/source/timeline/nack_scheduler`，很可能不会解决浏览器同环境稳定而 native 不稳定的核心差异。
- `str0m` 虽然可以减少部分 packet-to-frame 自建逻辑，但它自己也明确缺少 `Adaptive Jitter Buffer` 与完整 client media 能力；这与“复用浏览器类成熟闭环”的目标并不完全一致。
- 真正对位浏览器闭环的方案大概率需要接受 C++/FFI 或平台封装成本。
- 双轨并存阶段如果没有严格切断当前 recovery owner / timeline / host gate，会得到“换了 transport，没换病灶”的假结论。

## Progress

- [x] Step 1: 已完成当前代码与相关 RFC 的快速归因
- [x] Step 2: 已完成候选库的官方资料核对
- [x] Step 3: 已确认“transport 替换”和“receive pipeline 替换”是两类不同 spike
- [ ] Step 4: 待确认先执行哪一类 spike

## Execution Notes

- Date: 2026-05-19 | Status: planned
- Update: 初版 RFC 错把主要矛盾放在“当前 `rtc` 调度面太宽”。用户反馈后复核确认，真正矛盾是“浏览器同环境稳定，而当前 native Rust 栈反复被关键帧/IDR/NACK 闭环打穿”。
- Decision: 今后不再把“transport/API 替换”误写成“浏览器成熟闭环验证”。如果 spike 没有替掉 receive pipeline，就不能拿它证明或否定浏览器假设。
- Decision: `webrtc-rs/webrtc` 必须单独看待。它比当前 `rtc` 集成更完整地承载标准 WebRTC API/ICE/DTLS/SRTP/RTCP，但它的入站媒体 API 仍以 RTP 包读取为主，所以它不会天然接走当前 `source/timeline/bootstrap/recovery` 这条病灶链。
- Decision: `str0m` 在“减少 packet-to-frame 自建逻辑”上可能比 `webrtc-rs/webrtc` 更有价值，因为它有 frame-level API；但它自己明确不提供 adaptive jitter buffer 和完整 client media 能力，因此也不是浏览器闭环对位选项。
- Decision: `libdatachannel` 更像“成熟 browser interop + 轻量 native WebRTC transport”选项。它适合验证“是否当前 transport/API 胶水层过厚”，但仍不是完整 libwebrtc receive pipeline。
- Decision: 若目标真的是“复用浏览器同类成熟接收闭环”，应把 `libwebrtc-family wrapper` 作为对照组纳入，而不是只在纯 Rust 候选里比较。
- Date: 2026-05-20 | Status: planned
- Update: 结合 [`docs/isu/packet-to-frame-pipeline.md`](../isu/packet-to-frame-pipeline.md) 和近期多份 RFC，可确认当前 native 栈在包优先级、OOS/重排、NACK 取消、bootstrap salvage、keyframe response 解释、feedback target 绑定、clean-anchor gate 上都存在大量自建状态机。
- Decision: 当前方向建议从“继续收权当前 `rtc` 主线”切换为“优先做成熟闭环验证型 spike”；只有在明确拒绝 C++/FFI 或平台封装成本后，才退回 transport/API 替换型 spike。

## Candidate Assessment

### 0. 当前 `rtc` + 自建接收闭环（基线）

- 适配度：高
- 当前收益：
  - 现有 Xbox/cloud/home 互通、H264 family、RTCP feedback、TWCC 入口、SDP 调整与 data channel 语义已经沉淀在当前代码里。
  - 不需要引入新语言边界。
- 当前缺口：
  - 当前已不是薄控制面，而是完整自建 receive pipeline。
  - 过去多轮任务说明系统长期在补 `bootstrapMissingIdr / continuationAcceptedWhileAwaitingIdr / feedbackTargetPending / transportAwaitRecoveryAnchor / clean-anchor` 同一问题簇。
- 判断：
  - 如果目标是浏览器同类稳定性，这条线不应继续作为默认方向。

### 1. `webrtc-rs/webrtc`

- 适配度：中
- 当前收益：
  - 标准 WebRTC API、ICE/DTLS/SRTP/RTCP/SDP 语义更完整。
  - `v0.17.x` 至少存在一个 bugfix-only 稳定分支语义，适合“减少 API/协议胶水”的目标。
- 当前成本：
  - 官方 README 同时给出“early stage + master 转向 sans-io 新架构”的信号，维护面存在分叉风险。
  - 入站媒体 API 仍是 RTP 包级别读取；当前 `source/timeline/bootstrap/recovery` 病灶链不会自动消失。
- 判断：
  - 它适合 `transport/API 替换型 spike`，不适合直接验证“浏览器成熟闭环”假设。

### 2. `str0m`

- 适配度：中
- 当前收益：
  - 有 frame-level API，可实际减少当前 `packet -> frame` 自建逻辑。
  - 内建 NACK/TWCC/BWE/keyframe request，理论上能切掉当前一部分 source/nack/timeline 胶水。
- 当前成本：
  - README 自己明确与 `libWebRTC` 相比缺 `Adaptive Jitter Buffer`、decode、TURN 等关键 client 能力。
  - API 不是标准 `RTCPeerConnection` 风格，引入成本不低。
- 判断：
  - 它比 `webrtc-rs/webrtc` 更像“减少自建 receive pipeline”的候选，但并不等价于浏览器成熟闭环。

### 3. `libdatachannel`

- 适配度：中高
- 当前收益：
  - browser interop、ICE/TURN/DTLS/SRTP/RTX 等成熟度高，README 明确兼容 Chromium/Firefox/Safari。
  - FFI 体量比直接引入完整 Google reference library 更轻。
- 当前成本：
  - Rust 侧仍然面对 FFI、构建、打包、crash/debug 链。
  - 它主要解决的是标准 WebRTC transport/media transport，不直接承诺浏览器级 decode/jitter/render 闭环。
- 判断：
  - 它适合 `transport/API 替换型 spike`，优先级高于 `webrtc-rs/webrtc`，但仍不是浏览器闭环等价替代。

### 4. `libwebrtc-family wrapper`（对照组选项）

- 适配度：高
- 当前收益：
  - 这是唯一与“浏览器同环境为什么更稳”直接同类的候选。
  - 可直接复用成熟的 receive-side jitter / NACK / keyframe request / decode pipeline 设计，而不是只复用 transport。
- 当前成本：
  - C++/平台封装/FFI/二进制体积/构建链成本最高。
  - 需要重新设计与现有 Rust runtime stats、host present、Tauri 分发的桥接方式。
- 判断：
  - 如果目标真是“借成熟闭环止血”，这是最有信息价值的验证对象。

## Round Proposal

### Round 1：成熟闭环验证型 spike

- 目标：
  - 直接验证“浏览器类成熟接收闭环是否显著降低 `IDR 缺失 / continuation-only / feedbackTargetPending / keyframe loop` 风险”。
- 候选：
  - `libwebrtc-family wrapper`
- 范围：
  - 尽量替掉当前 `source/timeline/nack_scheduler/bootstrap gate`，而不是只替 peer connection。
  - 允许先绕开现有 trace/diagnostics，只保留最小可观测性。
- 成功标准：
  - 同环境下显著减少 `bootstrapMissingIdr / continuationAcceptedWhileAwaitingIdr / transportAwaitRecoveryAnchor` 停留时间。
  - `PLI/FIR` 请求频率和 `feedback target pending` 频率明显下降。
  - 不再需要当前大量 packet/frame 级补丁才能稳定出图。

### Round 2：transport/API 替换型 spike

- 目标：
  - 验证“是否当前 transport/API 胶水层本身就在放大问题”。
- 候选优先级：
  1. `libdatachannel`
  2. `webrtc-rs/webrtc`
  3. `str0m`（仅当愿意同时接受其非标准 API 与自带能力边界）
- 范围：
  - 可先只替 peer connection、ICE、DTLS/SRTP、RTCP/feedback target 绑定。
  - 明确保留当前 `source/timeline/decode/host` 时，这不是浏览器闭环验证。
- 成功标准：
  - 降低 `feedback target pending`、receiver binding、TWCC/PLI/FIR 执行胶水复杂度。
  - 明显减少 `connection + twcc + service` 自定义桥接层。

### Round 3：决定是否继续当前主线细修

- 继续细修当前主线的前提：
  - 已明确放弃成熟闭环替换方向，或 spike 证明替换收益不足以覆盖成本。
- 停止继续细修的前提：
  - 成熟闭环验证型 spike 明显优于当前 native 栈。

## Current Recommendation

- 当前第一建议：
  - **不要再把主要赌注压在“继续收权当前 `rtc` 主线”上。**
- 当前第二建议：
  - **优先执行“成熟闭环验证型 spike”**。如果你愿意接受 C++/FFI 或平台封装成本，就直接拿 `libwebrtc-family wrapper` 做对照组。
- 当前第三建议：
  - 如果现阶段必须保持 Rust-first，就执行 **`libdatachannel` 优先、`webrtc-rs/webrtc` 次之** 的 `transport/API 替换型 spike`，但要明确这不能验证浏览器假设。
- 当前第四建议：
  - `str0m` 不再作为默认第一 spike。它更像“换一种方式继续拥有接收端循环”，而不是“借成熟浏览器类闭环止血”。
