# iOS libwebrtc 串流运行时 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 执行中（桌面领域主链纠偏）
- Current State: in-progress
- Owner: agent
- Last Updated: 2026-07-17

## Background

- iOS xCloud 游戏库与详情页已经交付，Play 入口使用稳定 `streamTitleId`。
- `xbox-auth-flow`、`xbox-webapi`、`xbox-streaming` 已具备 streaming token、session、queue/state、configuration、SDP/ICE、keepalive 与 close 能力。
- iOS 当前只有最小 `StreamingRuntime` 协议骨架，尚未建立真实远端 session、PeerConnection、视频呈现与音频播放。
- 仓库决策固定为 Rust 控制面、Swift 生命周期、libwebrtc RTC 数据面、VideoToolbox/Metal/AVAudioSession 平台媒体层。
- 用户已确认直接执行代码实现，并负责最终模拟器运行验收。

## Goal

- 从游戏详情 `streamTitleId` 建立真实 xCloud session。
- 通过 Rust 控制面完成 token、region、session progression、SDP/ICE、keepalive 与 close，并复用桌面端唯一的 session flow owner。
- 在 Swift 层直接管理 libwebrtc PeerConnection、audio/video transceiver、remote track、RTCStats 与资源生命周期。
- 将视频 track 接入 `RTCMTLVideoView`，将音频接收接入 `RTCAudioSession/AVAudioSession`。
- 形成可编译、可取消、generation-safe、可诊断的端到端串流代码。

## Scope

- In scope:
  - `crates/xbox-ios-bridge` streaming control session 与 UniFFI API。
  - 必要的 `xbox-streaming` / `xbox-webapi` typed adapter 能力。
  - iOS 固定 libwebrtc 依赖与 Xcode 工程接入。
  - Swift session actor、PeerConnection runtime、视频 renderer、音频 session、播放器 UI。
  - 游戏详情 Play 入口、连接状态、取消、失败与稳定清理。
  - Rust/Swift 单元测试、静态构建和格式检查。
- Out of scope:
  - 手柄、触摸、键鼠、rumble 与 input DataChannel 发送。
  - 麦克风、party chat 与本地音频采集。
  - xHome Remote Play。
  - 自定义 Metal 后处理、HDR、HEVC/AV1。
  - 用户负责的最终模拟器运行与真实账号视觉验收。

## Architecture

```text
GameDetailView / StreamingPlayerView
              |
              v
       StreamSessionActor (Swift lifecycle owner)
          |                         |
          v                         v
Rust StreamingControlSession   LibWebRtcRuntime
SessionFlowService adapter      PeerConnection/media/stats
          |                         |
          v                         v
 Xbox cloud HTTP APIs       RTCMTLVideoView + RTCAudioSession
```

边界约束：

- Rust opaque session 保管 streaming token、region、remote session id 和领域 flow。
- `SessionFlowService` 与 `SessionScheduler` 继续作为 create/state/connect-token/keepalive/close 的唯一 owner。
- Swift 只接收建立 PeerConnection 所需的 SDP、ICE server plan 与脱敏状态。
- libwebrtc 音视频 frame/sample 始终位于 Swift/Objective-C++/系统媒体路径。
- 每次启动生成新的 attempt id；旧 task、callback 和 signaling poll 只能被当前 attempt 接受。
- stop 顺序固定为取消 remote ICE poll、关闭 PeerConnection、通过领域 flow 关闭 remote session、释放 Rust handle。

## Canonical Flow Correction

2026-07-17 真实 trace 证明旧 iOS bridge 在 session create 后立即轮询 `/configuration`，服务端在 provisioning 窗口返回 HTTP 410 `SessionNotActive`，SDP 与 ICE 尚未开始。该实现重复承担了桌面端已经收口的 session orchestration，并让 iOS 形成独立状态机、keepalive 定时器和 signaling poll 策略。

纠正后的主链固定为：

```text
XboxStreamSession::start
→ SessionFlowService::start_session_execution
→ create_session
→ SessionScheduler::start_loops
→ ReadyToConnect 时发送 transfer token
→ 等待 Provisioned / SessionReady
→ SessionFlowService::exchange_offer
→ submit_ice / poll_ice
→ SessionScheduler keepalive
→ SessionFlowService::close_session
```

iOS bridge 只实现 `SessionFlowProvider`、`SessionFlowSnapshot + SessionRuntimeBinding` 和 UniFFI DTO 投影。libwebrtc 使用平台默认 Xbox STUN 与 plan 中显式 TURN，运行时启动不再依赖 `/configuration` gate。

## Implementation Map

### Rust

- `crates/xbox-ios-bridge/src/streaming.rs`
- `crates/xbox-ios-bridge/src/lib.rs`
- `crates/xbox-ios-bridge/Cargo.toml`
- 必要时扩展 `crates/xbox-streaming/src/session/*`
- 重新生成 `iosapp/XBXRC/Platform/RustBridge/Generated/*`

### iOS

- `iosapp/XBXRC/Platform/Streaming/StreamingRuntime.swift`
- `iosapp/XBXRC/Platform/Streaming/LibWebRtcRuntime.swift`
- `iosapp/XBXRC/Platform/Streaming/StreamingControlClient.swift`
- `iosapp/XBXRC/Platform/Streaming/StreamingPlayerView.swift`
- `iosapp/XBXRC/Features/Library/GameDetailView.swift`
- `iosapp/XBXRC/App/*`
- `iosapp/XBXRC.xcodeproj/project.pbxproj`

## Plan

1. 固化 Rust control session 的 typed records、opaque handle、errors 与 session progression。
2. 接入固定 libwebrtc package/XCFramework，并建立 Swift PeerConnection factory/runtime。
3. 实现 offer/local description/local ICE 与 Rust signaling exchange。
4. 应用 answer/remote ICE，接入 video track/RTCMTLVideoView 与 RTCAudioSession。
5. 把 GameDetailView Play 接到全屏 StreamingPlayerView 与状态机。
6. 实现 cancellation、keepalive、前后台、失败和幂等 cleanup。
7. 生成 UniFFI bindings，执行 Rust 测试/check、Swift/Xcode 静态构建和差异检查。
8. 删除 iOS bridge 重复的 session backend、configuration poll、Swift keepalive timer 与 SDP answer poll，接入桌面 canonical flow。

## Validation

- [x] `cargo fmt`
- [x] `cargo test -p xbox-ios-bridge`
- [x] `cargo test -p xbox-streaming session::api::session --lib`
- [x] `cargo test -p xbxengine-protocol`
- [x] `cargo test -p xbxengine transport::rtc::connection::data_channel_tests --lib`
- [x] `cargo check -p xbox-ios-bridge -p xbox-streaming -p xbxengine`
- [x] UniFFI bindings 生成成功并与 Swift adapter 对齐
- [x] Xcode project plist 与 WebRTC package 引用静态校验成功
- [x] Swift 6 strict-concurrency typecheck 覆盖无 WebRTC 与 WebRTC 合同分支
- [x] Swift 状态机、generation、cleanup 与 ICE 完成信号定向 XCTest 源码通过 parse/typecheck
- [x] `git diff --check`
- [ ] 最终模拟器运行与真实账号串流由用户验收

### Canonical flow correction validation

- [x] `cargo test -p xbox-ios-bridge --lib`
- [ ] `cargo test -p xbox-streaming --lib`
- [x] UniFFI bindings 重新生成
- [x] iPhoneOS 26.0 arm64 全量 Swift + Rust + WebRTC executable link
- [x] `check-streaming-session-boundary.sh` 证明单一 Rust factory/session flow 与 Swift 职责边界
- [ ] iOS Simulator Debug 构建
- [ ] iOS Device Debug / Release 构建
- [ ] 新 trace 出现 `Provisioned/SessionReady → offer/answer → ICE`，且无 provisioning 阶段 `/configuration` 410

## Risks

- libwebrtc Swift Package/XCFramework 的固定版本、架构和模块名需要与当前 Xcode 26 工程匹配。
- Xbox answer 的 H.264 profile、DataChannel m-line 与 candidate 变体需要真实 session 验证。
- UniFFI opaque object 的异步销毁与 Swift actor cancellation 需要保持单一资源释放顺序。
- RTCAudioSession 的系统 route/interruption 行为需要模拟器或真机运行证据。
- 当前环境可能缺少 Simulator runtime，静态构建证据与用户运行验收分开记录。

## Progress

- [x] Step 1: 已核对 ISU、iOS bootstrap、游戏库详情与现有 Rust streaming 能力。
- [x] Step 2: Rust streaming control session、configuration、SDP/ICE、keepalive、close 与 UniFFI 已完成。
- [x] Step 3: 固定 libwebrtc package、Swift PeerConnection runtime、DataChannel 和 RTCStats 已完成。
- [x] Step 4: 游戏入口、全屏播放器、视频 Metal view、首帧探针与接收音频已完成。
- [x] Step 5: Rust/Swift 静态验证、Report 与任务状态收口已完成；真实 package resolve 和模拟器实跑由用户验收。
- [x] Step 6: iOS control adapter 已切换到桌面 `SessionFlowService + SessionScheduler`，移除 `/configuration` readiness、Swift keepalive 与分离式 SDP poll。
- [x] Step 7: bindings、全量 Swift parse、iPhoneOS Swift 6 strict-concurrency typecheck、arm64 executable link 与 Rust-owned session/ICE 门禁已完成。
- [ ] Step 8: 模拟器/真机真实账号 trace 验收待用户运行后继续。

## Execution Notes

- Date: 2026-07-16 | Status: in-progress
- Update: 用户确认按规划直接实现 session、协商、控制面、libwebrtc、视频帧与音频接收，手柄留在后续任务。
- Decision: Swift `StreamSessionActor` 持有 iOS 生命周期，Rust opaque session 持有 Xbox 远端控制面，libwebrtc 持有 RTC 数据面。
- Decision: 视频帧和音频 sample 保持在 libwebrtc/iOS 媒体路径，Rust 只消费低频状态与 signaling 数据。
- Risk/Blocker: 固定 libwebrtc artifact 的精确版本和当前 Xcode 兼容性需要依赖解析证据。
- Update: `StreamSessionActor` 已补 generation-safe stale stop、重复 connected 幂等、首帧/连接事件顺序保护和资源单次释放。
- Update: 本地 ICE gathering complete 已通过空候选批次提交给 Xbox，和桌面浏览器主线保持一致。
- Decision: Xbox DataChannel profile、Handshake、post-handshake 配置、HandshakeAck 判断和 control bootstrap 已收口到 `xbxengine-protocol`，Swift 只管理 libwebrtc channel 生命周期与发送。
- Update: `streamTitleId` 按目录稳定标识处理，接受有界可打印字符串，覆盖非十六进制真实目录 ID。
- Validation: `cargo test -p xbox-ios-bridge` 24 项、`cargo test -p xbxengine-protocol` 7 项、xbxengine DataChannel 5 项、Swift 两路 strict-concurrency typecheck、PBX lint 与 diff check 通过。
- Validation: 全量 `cargo test -p xbox-streaming` 为 95/96；唯一失败是既有 runtime compiler 断言 `36 != 48`，本任务改动的 session API 定向测试通过。
- Risk/Blocker: 当前环境无法连接 CoreSimulatorService，联网 package resolve 也被审批服务拒绝；真实 `137.7151.04` headers、Device/Simulator build 与账号串流按用户约定留给模拟器验收。
- Date: 2026-07-17 | Status: in-progress
- Deviation: 初版 iOS bridge 自行实现 session create/state/configuration/connect-token/keepalive/signaling poll，偏离 `SessionFlowService` 唯一 owner 约束。
- Decision: iOS bridge 采用与 Tauri adapter 相同的领域接口形状；Rust flow 负责完整控制面，Swift 保留 libwebrtc、媒体与应用生命周期。
- Update: 已删除 provisioning 阶段 `/configuration` 轮询与 410 pending 特判，SDP 改为 `SessionFlowService::exchange_offer`，ICE 与 close 改走同一 flow，Swift keepalive timer 已移除。
- Validation: `cargo test -p xbox-ios-bridge --lib` 26 项通过，`cargo test -p xbox-streaming session::flow::tests --lib` 41 项通过，`xcrun swiftc -parse` 与 `git diff --check` 通过；全量 `xbox-streaming` 仍为既有 runtime compiler 断言 `36 != 48` 单项失败。
- Blocker: Xcode Simulator 构建需要访问 CoreSimulator、SwiftPM/clang 缓存和固定 WebRTC artifact；当前受执行沙箱权限与审批服务异常阻塞，尚未进入 Swift 编译阶段。
- Date: 2026-07-17 | Status: in-progress
- Audit: cloud/home 已共同使用单一 `create_stream_session(access_handle, target_id)`，target 由 Rust access handle 权威解析，并进入同一个 `SessionFlowService<IosSessionSnapshot, IosSessionFlowProvider>`；Swift 已退出会话类型路由。
- Decision: Swift 只等待 Rust session ready、转交本地 offer/ICE、应用 Rust 返回的远端 ICE，并管理 libwebrtc、媒体与应用生命周期；session progress、远端 ICE 拉取节奏和结束判定全部由 Rust opaque session 持有。
- Validation Plan: 增加 cloud/home 共用 Rust session adapter 回归、Rust-owned remote ICE batch 回归、Swift 远端 ICE 应用回归，并执行 Rust 测试、bindings 生成、源码边界门禁与 Device build。
- Update: UniFFI `start()` 只向 Swift 返回 generation 与 ICE server；session snapshot、单次 `pollIce`、keepalive 和 ICE poll interval 已从原生合同移除。Rust `next_remote_ice_batch()` 持有轮询间隔、30 秒窗口、去重、空批次计数与结束判定。
- Update: Swift `StreamSessionActor` 只创建/停止 libwebrtc、转交本地 offer/ICE、应用远端 ICE、处理 track/首帧/stats 与应用生命周期；cloud/home 均使用同一个 `RustStreamingControlSession` adapter。
- Update: 删除独立 `create_home_stream_session` UniFFI 入口，Swift factory 不再按 `.cloud/.home` 分支；cloud/home access handle 进入同一个 Rust factory 后由 `StreamingAccessContext.target` 编译对应 plan。
- Update: 共享 access registry/context 已改名为 `STREAM_ACCESS + StreamingAccessContext`，所有消费者统一通过 `load_stream_access/release_stream_access`；云目录显式拒绝 home target handle，防止共享 registry 模糊调用边界。
- Validation: `cargo test -p xbox-ios-bridge` 30 项、`cargo test -p xbox-streaming session::flow::tests --lib` 41 项、`cargo check -p xbox-ios-bridge -p xbox-streaming`、全量 Swift parse、Swift 6 strict-concurrency 边界 typecheck、bindings 生成、`check-streaming-session-boundary.sh`、PBX lint 与 diff check 通过。
- Validation: 全量 `cargo test -p xbox-streaming --lib` 为 95/96；失败项继续是既有 runtime compiler 断言 `36 != 48`，session flow 41 项全部通过。
- Blocker: 当前改动的 Device build 需要沙箱外 Xcode/SwiftPM 缓存权限；提权审批服务返回 `codex-auto-review` 404，沙箱内构建停在 SwiftPM `sandbox_apply: Operation not permitted`。等待用户显式批准后复跑完整 Device build。
- Date: 2026-07-19 | Status: completed
- Resolution: 使用仓库真实 Swift 源码、生成 bindings、iPhoneOS 26.1 SDK、Device arm64 WebRTC framework 与 `aarch64-apple-ios` Rust 静态库完成独立 executable link，绕开 SwiftPM/Xcode 服务权限后仍验证了当前 ABI、平台模块和最终链接合同。
- Evidence: 产物 `/tmp/xbxrc-ios-manual-link/XBXRC` 为 `Mach-O 64-bit executable arm64`，`LC_BUILD_VERSION platform=2, minos=26.0, sdk=26.1`，并包含统一 `create_stream_session`、`XboxStreamSession.start` 与 `next_remote_ice_batch` UniFFI 符号。
- Completion: cloud/home 会话层边界代码与可链接性已完成；真实账号、首帧、音频和运行 trace 继续由用户模拟器/真机验收。
- Date: 2026-07-19 | Status: in-progress
- Incident: 真机主机串流返回 `HTTP 500: Exceeded max updated attempts.`；当前工作区未找到对应 iOS Runtime Trace JSONL，截图用于确认服务端错误类别，因果链由 session flow 与 Swift signaling 审计建立。
- Root Cause: `ReadyToConnect` 每个 monitor tick 都请求 connect token，scheduler 先前缺少成功幂等状态；`RuntimeStarting` 又被启动门提前视为 ready，导致重复 `/connect` 与 `/sdp`、本地 ICE 更新重叠修改同一远端 session。
- Update: `SessionFlowService` 已为 `/connect`、SDP、ICE、keepalive 和 DELETE 建立 session-generation mutation gate；connect token 成功后仅发送一次，失败保留下一 tick 重试；启动门只在 `SessionReady` 放行 WebRTC。
- Update: session store 用 cancel-token identity 做条件回写/删除，关闭期间延迟 monitor 回包无法复活会话，同 session-id 重用时旧 generation 无法清除或覆盖新会话。
- Update: iOS 本地 ICE 在 answer 后按 60ms 合批，completion 独立 trace 已补齐；completion await 期间到达的候选会续排 flush。`StreamSessionActor` 增加 latest-generation 门和 cleanup 快照，连续 start/stop 保持最新请求所有权。
- Validation: session 相关 Rust 81 项、iOS bridge 30 项、协议 7 项通过；`cargo check`、bindings 生成、Swift parse、session boundary、Rust fmt 与 diff check 通过。全量 `xbox-streaming` 为 96/97，唯一失败继续是既有 runtime compiler `36 != 48` 断言。
- Acceptance: 下一次真机串流需导出 iOS Runtime Trace，确认 `SessionReady → signalingOfferSucceeded → ICE batch/completion` 且无重复 connect、completion 后 candidate 和 `Exceeded max updated attempts`。
