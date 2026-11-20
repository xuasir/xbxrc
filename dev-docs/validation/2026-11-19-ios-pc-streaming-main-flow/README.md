# iOS 与 PC 串流主流程核对

日期：2026-11-19。范围：当前 `main` 工作区的 iOS 客户端与 PC Browser 主线，包含已暂存、未暂存和新增源码。PC Native WIP 的运行时按 `rust` 开发线独立演进。

两端共享 Rust 会话控制主链；宿主认证恢复、xHome 区域上下文、媒体恢复和帧反馈仍有具体差异。本轮进行源码审计与现有门禁回归，真实账号串流和设备性能沿后续实机验收。

## 主流程

公共流程为：准备目标凭据与配置 → 编译 Rust Plan → xHome 唤醒/等待注册或 xCloud 资格复核 → 创建会话/排队 → SessionReady/connect token → SDP offer/answer → 增量 ICE → Xbox DataChannel 握手/bootstrap → 视频/音频与输入 → 保活/远端终态监控 → 退出与资源清理。

- PC：`Stream.vue/useStreamExecution` → Tauri `StreamingService` → `SessionFlowService` → Browser runtime/PlayerClient → WebView WebRTC → video 或 WebGL2。
- iOS：`StreamingFeatureStore` → `StreamSessionActor` → UniFFI `XboxStreamSession` → `SessionFlowService` → libwebrtc → 自定义 Metal renderer。
- xCloud 商品到启动身份的复核在 iOS 的 `startCloudGame` 完成；PC 由 Rust 宿主的 `validate_cloud_stream` 与能力解析完成。两端都在创建会话前校验当前启动事实。
- 两端的会话创建、排队、connect token、信令 API、keepalive 与远端 terminal 使用同一公共 flow/scheduler。Swift 通过 Rust 批次接口消费远端 ICE，PC Browser 经 Tauri 消费单次 ICE 查询结果。

主要入口：[iOS store](../../../iosapp/XBXRC/Platform/Streaming/StreamingRuntime.swift)（74、149行）、[iOS actor](../../../iosapp/XBXRC/Platform/Streaming/StreamSessionActor.swift)（130行）、[iOS bridge](../../../crates/xbox-ios-bridge/src/streaming.rs)（567、616行）、[PC execution](../../../src/streaming/useStreamExecution.ts)（460行）、[PC service](../../../src-tauri/src/mods/streaming/service.rs)（698行）。

## 当前差异与影响

| 环节 | PC main | iOS 当前实现 | 影响 |
| --- | --- | --- | --- |
| 媒体运行时 | WebView WebRTC；浏览器解码，video/WebGL2 呈现，可选 FSR/锐化 | libwebrtc 解码，Metal mailbox 与单帧在途，可选 MetalFX/画面效果 | 网络协议主线相同，解码/呈现调度及平台成本分别验证 |
| 播放门禁 | Browser `mediaReady` 来自视频呈现事件，独立维护 connected milestone | 明确要求 peerConnected、controlReady、displayPipelineReady、mailbox 接收、submit、present 六项事实 | iOS 对控制与显示就绪的组合要求更细 |
| 首帧与冻结恢复 | 4秒首帧 guard；1秒媒体监督，依据网络/解码/显示证据请求关键帧、重建播放链及经公共 arbiter 重连 | stats 约1秒采样用于诊断；自动恢复主要由 peer disconnected 触发，执行 ICE restart → peer rebuild | connected 且画面停住的场景需要补齐 iOS 应用层监督与分级恢复 |
| 帧反馈基线 | 有效 rVFC RTP/receive 元数据进入 input 包；分辨率保持关闭时发送规范化原始差值，开启时对 wire 副本限幅 | 仅 `preventResolutionDrops=true` 时创建 decoder 观测与反馈队列并附入 input 包 | 默认关闭时，两端发送给服务端的帧反馈行为存在差异 |
| 帧反馈时间来源 | receiveTime 与 expectedDisplayTime；真实 processingDuration 独立记录 | 完整编码帧进入 decoder、decoder 回调与 Metal commandBuffer completion | 反馈字段含义和端到端时延口径需保留来源标记 |
| 串流凭据被拒绝 | Tauri provider 实现公共 `recover_streaming_token`，接入目标凭据续期 | iOS 启动前准备 access；session provider 返回 access 内 gsToken，凭据恢复沿 trait 默认401结果 | 会话 API 返回401后的透明续期需接入 iOS 宿主 |
| xHome 主机区域 | 跨区发现，保存 discoveryRegionName，并将本次会话区域绑定到发现结果 | Smartglass 主机事实包含主机身份、电源、地址；Plan 使用 access 的 session regions | 主机注册区域与默认区域不同的场景需要补齐上下文 |
| xHome ICE 适配 | 公共信令读取当前 configuration，按远端媒体/BUNDLE/ufrag 绑定补候选并保留来源 | 复用同一公共信令，同时 bridge 再附加 Plan.console_addrs 候选；附加项固定 mid/index=0、ufrag=None，并投影掉 source | iOS 适配层需要收敛到公共候选结果，强化代次绑定与来源诊断 |
| 输入 | Browser Gamepad API：rAF + 4ms补采；输入订阅发送，50ms重试、100ms心跳；统一键盘映射 | GameController + 现有虚拟触屏手柄；MainActor 8ms轮询，背压与中立释放保护 | 实际采样/发送节奏分别受平台调度影响；新 TouchLayout Playground 当前输出到本地 recorder |
| 麦克风与音频 | 远端播放、动态开麦与 SDP 重协商 | 当前 Rust WebRtcPlan 固定 audio recvonly；原生音频会话处理播放、路由与中断 | 远端音频播放已接入，麦克风上行属于能力差异 |
| 应用进入后台 | document hidden 时调整渲染预算与恢复监督，保留会话 | scene background 时清理串流；inactive 时先抑制输入 | 后台返回的会话连续性由各自生命周期策略决定 |

播放门禁的 present 是当前运行时事件口径。iOS `framePresented` 由 Metal commandBuffer 完成回调推进；物理屏幕 scanout 和真实 glass-to-glass 延迟需要设备测量。

## 关键代码证据

| 结论 | 源码位置 |
| --- | --- |
| 公共 xHome 当前 configuration 查询与媒体传输绑定 | [signaling flow](../../../crates/xbox-streaming/src/session/signaling/flow.rs)，198行；[signaling state](../../../crates/xbox-streaming/src/session/signaling/state.rs)，58行起 |
| iOS 播放六事实门禁 | [StreamingSessionState](../../../iosapp/XBXRC/Platform/Streaming/StreamingSessionState.swift)，16行 |
| iOS 断线恢复与 peer rebuild | [StreamSessionActor](../../../iosapp/XBXRC/Platform/Streaming/StreamSessionActor.swift)，712、797行 |
| PC 首帧保护与媒体监督 | [browser runtime](../../../src/streaming/runtime/browser-runtime.ts)，1175、1933行；[supervision](../../../src/streaming/runtime/browser-supervision.ts)，74行 |
| iOS 按开关创建反馈链 | [LibWebRtcRuntime](../../../iosapp/XBXRC/Platform/Streaming/LibWebRtcRuntime.swift)，666行；[input channel](../../../iosapp/XBXRC/Platform/Streaming/XboxStreamDataChannels.swift)，399、432行 |
| PC 持续采集常规帧反馈 | [PlaybackService](../../../src/player/app/media/PlaybackService.ts)，415行；[InputService](../../../src/player/app/input/InputService.ts)，402行；[反馈规范化](../../../src/player/app/input/frame-feedback.ts)，7行 |
| iOS 反馈观测边界 | [StreamingFrameFeedback](../../../iosapp/XBXRC/Platform/Streaming/StreamingFrameFeedback.swift)，19、243、265行；[Metal completion](../../../iosapp/XBXRC/Platform/Streaming/StreamingMetalRenderer.swift)，1012行 |
| PC401恢复与 iOS provider | [Tauri provider](../../../src-tauri/src/mods/streaming/service.rs)，346行；[iOS provider](../../../crates/xbox-ios-bridge/src/streaming.rs)，360行；[trait默认恢复](../../../crates/xbox-streaming/src/session/flow.rs)，254行 |
| PC跨区发现与区域绑定 | [query service](../../../src-tauri/src/mods/data/services/streaming_query_service.rs)，90行；[context](../../../src-tauri/src/mods/streaming/context.rs)，158、250行 |
| iOS主机事实与附加候选 | [HomeHostFacts](../../../crates/xbox-ios-bridge/src/cloud_access.rs)，61行；[control_plan](../../../crates/xbox-ios-bridge/src/streaming.rs)，1101行；[固定mid候选](../../../crates/xbox-ios-bridge/src/streaming.rs)，1365行 |
| 麦克风方向与后台清理 | [iOS WebRtcPlan](../../../crates/xbox-ios-bridge/src/streaming.rs)，1253行；[PC开麦](../../../src/streaming/runtime/browser-runtime.ts)，2333行；[iOS scene](../../../iosapp/XBXRC/Platform/Streaming/StreamingRuntime.swift)，497行；[PC visibility](../../../src/streaming/runtime/browser-runtime.ts)，837行 |

## 建议顺序

1. 补齐 iOS 首帧与 connected 状态下的冻结监督，沿公共恢复裁决和现有 libwebrtc/Metal 主线执行关键帧请求、局部恢复、ICE restart 与 peer rebuild；保留动作预算、冷却与效果探针。
2. 对齐帧反馈基线：常规模式消费原始观测，分辨率保持模式处理 wire 副本；同时保留 decoder ingress、decode completion、GPU completion 的独立时间来源。
3. 接入 iOS session provider 的401目标凭据续期，并贯通账户/凭据代次。
4. 复用公共 xHome 跨区发现与远端传输候选绑定，收敛 bridge 追加地址适配；保留当前 configuration 与候选来源诊断。

后续实机证据按同一会话、Peer/协商代次和稳定观察窗口对齐六维：网络健康、恢复链健康、media supply、steady supply、submit/present P95/P99、帧供给 delta。本轮六维运行结果均保持待测，现有 stats 与分析器回归证明代码和诊断合同的覆盖范围。

## 本轮验证与产物

- `sh iosapp/scripts/check-streaming-session-boundary.sh`：通过，确认共享 Rust session 与唯一 libwebrtc/Metal 装配。
- `python3 -B -m unittest discover -s iosapp/scripts/tests -p 'test_check_streaming_core_trace.py'`：14项通过，验证 trace 分析器的确定性回归。
- 检查结果及源码指纹见 [checks.json](checks.json)。
- 本轮保留本目录审计与检查摘要；单元测试的临时目录由 `TemporaryDirectory` 上下文回收。本轮持久临时构建、日志、截图、结果包为0，清理释放量为0。
- 生产代码沿审计开始时的工作区版本，账本和证据目录登记本次核对结论。
