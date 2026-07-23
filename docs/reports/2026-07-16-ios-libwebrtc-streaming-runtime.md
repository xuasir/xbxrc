# iOS libwebrtc 串流运行时 Report

> 2026-07-17 correction: 真实 trace 暴露初版 iOS bridge 提前轮询 `/configuration` 并重复实现桌面 session flow。本报告中的 configuration readiness、iOS 自建 keepalive 与分离式 SDP poll 已由后续纠偏替换；当前权威执行状态以对应 RFC 的 `Canonical Flow Correction` 为准。

> 2026-07-19 boundary closure: cloud/home 已统一为单一 `create_stream_session + SessionFlowService + SessionScheduler`。Swift 已退出会话类型路由、session snapshot、keepalive 与 ICE 轮询策略，只保留 libwebrtc、媒体/应用生命周期及远端 ICE 应用。

## Summary

- Related RFC: [`docs/rfcs/2026-07-16-ios-libwebrtc-streaming-runtime.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-07-16-ios-libwebrtc-streaming-runtime.md)
- 已完成 iOS xCloud/xHome 统一 Rust 会话层、SDP/ICE 协商、libwebrtc 音视频接收、Metal 视频呈现、音频会话和生命周期清理代码。手柄、触摸、键鼠与麦克风保持后续范围。

## Delivered

- `XboxStreamSession` Rust/UniFFI 控制对象通过 `SessionFlowService + SessionScheduler` 覆盖 cloud/home session 创建、queue/state、ReadyToConnect transfer token、SDP、ICE、keepalive、connected 与 close。
- cloud/home access handle 统一进入 `create_stream_session`，Rust `StreamingAccessContext.target` 负责编译目标 plan；Swift factory 无目标分支。
- Rust `next_remote_ice_batch` 持有远端 ICE 轮询间隔、30 秒窗口、去重、空批次计数与结束判定，Swift 只向 libwebrtc 应用候选。
- Swift `StreamingFeatureStore + StreamSessionActor` 覆盖详情页 Play、generation、取消、后台/登出、失败、重启和幂等资源释放。
- libwebrtc runtime 覆盖 H.264/Opus 优先、audio/video recvonly transceiver、offer/answer、ICE、RTCStats、四条 Xbox DataChannel 和首帧探针。
- `RTCMTLVideoView` 直接消费远端 `RTCVideoTrack`；`RTCAudioSession` 管理远端音频 playout 与系统音频路由。
- Xbox DataChannel profile、Handshake、post-handshake 配置、HandshakeAck 和 control bootstrap 统一复用 `xbxengine-protocol`，Swift 只承担平台生命周期与发送。

## Changes

- 新增 `crates/xbox-ios-bridge/src/streaming.rs`，通过宿主 provider/snapshot adapter 复用 `xbox-streaming` canonical flow。
- 新增 `crates/xbxengine/protocol/src/stream_data_channel.rs`，桌面 xbxengine 与 iOS bridge 共用同一协议 payload。
- 新增 `StreamingContracts.swift`、`StreamSessionActor.swift`、`LibWebRtcRuntime.swift`、`StreamingPlayerView.swift` 与 `XboxStreamDataChannels.swift`。
- Xcode 工程固定 `stasel/WebRTC` `137.7151.04`，加入 WebRTC product、`-ObjC` 和 package 校验脚本。
- 修复 stale stop 关闭新会话、重复 connected 导致控制面失败、启动错误轮询悬挂、非十六进制 `streamTitleId` 被拒绝、本地 ICE 缺少结束批次等问题。

## Validation

- `cargo fmt --all -- --check` 通过。
- `cargo test -p xbox-ios-bridge`：24 项通过。
- `cargo test -p xbxengine-protocol`：7 项通过。
- `cargo test -p xbxengine transport::rtc::connection::data_channel_tests --lib`：5 项通过。
- `cargo test -p xbox-streaming session::api::session --lib`：1 项通过。
- `cargo check -p xbox-ios-bridge -p xbox-streaming -p xbxengine` 通过。
- UniFFI Swift、C header 与 modulemap 重新生成成功。
- 全部 App/XCTest Swift 源码 parse 通过；iPhoneOS Swift 6 strict-concurrency typecheck 在无 WebRTC 分支和 WebRTC 合同 stub 分支均通过。
- `plutil -lint iosapp/XBXRC.xcodeproj/project.pbxproj` 与 `git diff --check` 通过。
- 全量 `cargo test -p xbox-streaming` 为 95/96；失败项是仓库既有 `rust_owned_cloud_aligns_video_pipeline_but_keeps_sidecar_recovery_profile` 的 `36 != 48` 断言。
- `cargo test -p xbox-ios-bridge`：30 项通过；`cargo test -p xbox-streaming session::flow::tests --lib`：41 项通过。
- `check-streaming-session-boundary.sh` 通过，UniFFI 不再导出 session snapshot、单次 ICE poll、keepalive、目标专用 session factory 或目标专用 release。
- iPhoneOS 26.0 arm64 全量 Swift + Rust + WebRTC executable link 成功；Mach-O 与 UniFFI 符号门禁通过。
- 2026-07-19 真机 `HTTP 500: Exceeded max updated attempts.` 复盘后，connect token 成功幂等、`SessionReady` 启动门、远端 session mutation 串行、session-generation 条件回写/删除和 iOS ICE completion 续排已完成。
- 最新回归：session 相关 Rust 81 项、iOS bridge 30 项、协议 7 项通过；bindings、Swift parse、边界门禁、fmt 与 diff check 通过。全量 `xbox-streaming` 为 96/97，保留既有 runtime compiler `36 != 48` 单项失败。

## Risks

- 当前环境的 Xcode/SwiftPM 沙箱服务不可用；真实 M137 Device framework 已用于独立 iPhoneOS 编译和链接。
- CoreSimulatorService 当前不可用，模拟器/真机真实账号会话和音视频体验由用户按约定验收。
- 首轮采用 `RTCMTLVideoView` 与 libwebrtc 默认音频 playout，精确 present 指标、自定义渲染和 PCM tap 留到后续性能阶段。

## Follow-up

- 使用真实账号分别启动 cloud 游戏与 home 主机串流，验证 queue、首帧、音频、退出、后台和快速重启，并导出 iOS Runtime Trace。
- 后续独立任务接入 GCController/input DataChannel、触控/键鼠、rumble 与麦克风。
