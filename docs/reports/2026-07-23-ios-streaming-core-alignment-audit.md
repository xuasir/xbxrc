# iOS xHome / xCloud 核心串流对齐审计

- Date: 2026-07-23
- Status: In Progress
- RFC: [`../rfcs/2026-07-21-ios-streaming-end-to-end-contract-alignment.md`](../rfcs/2026-07-21-ios-streaming-end-to-end-contract-alignment.md)

## Scope

本轮聚焦 iOS 复用 Rust 会话、Swift/libwebrtc 接管媒体与 DataChannel 的核心闭环：会话可连接、控制面 ready、首帧与真实 Metal surface 可见。物理手柄采样、输入帧和震动进入后续 `streaming-input` 验收。

## Audit Result

共享架构保持对齐：xHome/xCloud 共同使用 Rust `SessionFlowService + SessionScheduler`，Swift 负责 libwebrtc、Metal、DataChannel 与应用生命周期。核心阻塞集中在三个宿主边界。

1. Rust signaling policy 固定输出 `a=candidate:`，iOS M137 native candidate 边界消费 `candidate:`。该格式错位覆盖 xCloud poll ICE、xHome console address 和 Teredo 派生候选，会导致 selected pair 缺失及 ICE checking/failed。
2. iOS 先前把 control/input bootstrap 全部扣到 HandshakeAck 之后；桌面会在通道 open 后预发，并在 Ack 后 replay。Ack 延迟会同步扣住 authorization、gamepad absent、keyframe request 与 input metadata。
3. `.playing` 先前只依赖 peer connected 与 first frame；真实可用核心串流还需要 Ack 后 control replay 完成。presentation trace 先前缺少 attemptId/generation/peerEpoch，无法证明首帧进入当前 Metal surface。

## Implemented Alignment

- Swift → M137 remote ICE adapter 完成 trim、EOC/异常 `UDP + tcptype` 过滤、单个 `a=` 剥离与 `candidate:` 校验。
- 异常 UDP candidate 的 `tcptype` 与 `tcptype=...` 两种 wire 写法均在 native boundary 过滤，避免无效候选中断后续 ICE 应用。
- 单 candidate add 失败按候选隔离，同批后续候选继续应用；连接终态继续由 peer state 权威决定。
- DataChannel 状态机增加 pre-handshake 与 post-handshake 两阶段；control/input open 后立即预发，Ack 后幂等 replay，每阶段发送失败可重试。
- Ack 后 control replay 完成产生单次 `controlReady`；`.playing` 同时要求 peer connected、first video frame、control ready 与当前 Metal renderer ready。
- `videoSurfaceAttached`、非零 `videoSurfaceSized`、`videoSurfaceRendererReady` 关联当前 attemptId/generation/peerEpoch。
- Rust `Plan.session.device` 的目标宽高已经进入 `XboxWebRtcPlan`、UniFFI 和 Swift DataChannel adapter；xCloud Auto 的 1280×720、xHome 默认的 1920×1080 会分别驱动 H.264 `max-fs` 与 `/dimensionschanged`。
- Rust negotiation 的 `audio_bitrate_kbps` 已进入同一 plan，并由 Swift SDP projector 写入 audio `b=AS`，默认 128 kbps 与桌面端一致。
- iOS 设置页与 `AppSettingsStore` 现已只接入有消费点的串流设置，并通过 `StreamingFeatureStore -> createScopedStreamSession -> control_plan` 进入 Rust policy，覆盖游戏语言、xCloud/xHome 分辨率、codec、IPv6、xCloud/xHome/audio bitrate 与 xHome TURN fallback；无 iOS 原生消费点的设置继续保持跳过。
- 媒体健康改为按 stats 采样 delta 认定 supply，断流和计数回退不会推进 `lastMediaAt` 或 `steadyMediaObserved`。
- `.playing` 现在要求 peer connected、control ready、首帧和当前 attempt/generation/peerEpoch 的 Metal `videoSurfaceRendererReady`；transport 关闭路径独立 drain 并清理四条 DataChannel。
- xHome 初始待机启动消费 `wake_console/require_console_ready` 并等待显式注册；Rust scheduler 在 SessionReady 后继续轮询 Failed/Closed/timeout，iOS bridge 脱敏投影终态，Swift 在 EOC 后保持低频 polling 以触发单次 cleanup。
- remote ICE 增加 batch received/applied/completed trace；DataChannel 增加 pre/post/control-ready canonical anchors。
- 项目内增加 `streaming-core` trace gate，覆盖连接、控制面、media supply、steady supply、真实 surface、唯一终态与 cleanup。`.agents` skill 目录当前为只读，gate 暂放在 `iosapp/scripts`。

## Evidence

已通过：

- `cargo fmt --all -- --check`
- `cargo test -p xbox-ios-bridge -p xbox-streaming -p xbxengine-protocol --lib`：44 + 97 + 7 passed
- `find iosapp/XBXRC iosapp/XBXRCTests -name '*.swift' -print0 | xargs -0 xcrun swiftc -parse`
- `bash iosapp/scripts/check-streaming-session-boundary.sh`
- `python3 -B -m unittest discover -s iosapp/scripts/tests -p 'test_*.py'`：11 passed
- iPhoneOS 26.1 / Swift 6 strict typecheck：App 与 XCTest 使用真实 WebRTC M137 Device framework、UniFFI bridging header 通过；该门禁捕获并修复了 Metal renderer ready sink 的可选 trace context 编译错误。
- `python3 -B -m unittest discover -s .agents/skills/analyze-ios-runtime-trace/tests -p 'test_*.py'`：3 passed
- `python3 -m py_compile iosapp/scripts/check-streaming-core-trace.py iosapp/scripts/tests/test_check_streaming_core_trace.py`
- `sh iosapp/scripts/generate-rust-bindings.sh`
- `git diff --check`

第二轮修复后追加通过：

- `cargo test -p xbox-ios-bridge -p xbox-streaming -p xbxengine-protocol --lib`：49 + 103 + 7 passed
- `cargo fmt --all -- --check`
- 全量 Swift `swiftc -parse`
- `sh iosapp/scripts/check-streaming-session-boundary.sh`
- `python3 -B -m unittest discover -s iosapp/scripts/tests -p 'test_*.py'`：11 passed

隔离 Device `build-for-testing` 已复用本机缓存的 M137 WebRTC artifact，并将 DerivedData、SourcePackages 和 module cache 放入 `/tmp`。源码级 App/XCTest strict typecheck 已通过；Xcode 仍尝试写入用户 SwiftPM diagnostics cache，同时 CoreSimulatorService 失效，本轮沙箱外重跑的自动审批服务返回 503，完整 XCTest build 保留为外部 gate。

现有 6 份 Simulator trace 共 849 rows，schema、sequence、privacy、budget、startup/library coverage 通过；`check-streaming-core-trace.py --strict` 的唯一失败为 `missing-streaming-attempt`，这些 trace 未包含 `ios-streaming` 事件，无法承担 xHome/xCloud 核心串流验收。

## Remaining Acceptance

- xHome 真机：验证 console/LAN 或 TURN candidate 成功应用、peer connected、control ready、首帧、steady media 与 Metal surface。
- xCloud 真机：验证 poll remote ICE、selected pair、peer connected、control ready、首帧、steady media 与 Metal surface。
- 两条 trace 均需在主动停止后通过 `streaming-core` gate，并保持敏感字段扫描为零。
- 两条 trace 需覆盖 remote session Failed/Closed/timeout 到 Swift terminalSelected 和 DataChannel/peer/access cleanup 的唯一终态。
- target width/height 已从 Rust plan 投影到 Swift，覆盖 xCloud 720p、xHome 1080p 和可配置 1440p profile；真实设备仍需验证远端实际输出是否接受对应 dimensionschanged 与 H.264 frame limit。

当前代码已收敛核心 P0/P1。最终完成条件由 xHome 与 xCloud 两份 fresh 真机 trace 共同证明。
