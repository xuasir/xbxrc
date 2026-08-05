# iOS 零拷贝 Metal 渲染主线替换 `RTCMTLVideoView` RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: in_progress
- Owner: agent
- Last Updated: 2026-08-05

## Background

- 当前 iOS 串流播放路径在 [StreamingPlayerView.swift](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamingPlayerView.swift) 内将远端 `RTCVideoTrack` 直接挂到 `RTCMTLVideoView`。
- 最新 Simulator home 串流 trace 已证明 session、offer/answer、ICE、control ready、`videoSurfaceRendererReady`、`firstVideoFrame` 与 `framesDecoded` 持续增长都成立，故障面收敛到最终 presentation / visible present，而非协商、会话或 media supply。
- 当前 [StreamSessionActor.swift](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamSessionActor.swift) 的 `displayHealthSnapshot` 仍为 `unsupported`，iOS 端缺少真实 submit/present 证据，无法回答“首帧是否真正显示到屏幕”。
- 产品后续明确需要低延迟上屏、latest-only 本地丢帧避免积压、超分和后处理效果链。`RTCMTLVideoView` 将 decode 后调度、present 时机、纹理导入与 telemetry 封装在 libwebrtc 内部，无法承载这组目标。
- 用户已确认渲染主线直接移除 `RTCMTLVideoView`，切到零拷贝自定义 Metal renderer。

## Goal

- iOS App 主播放路径完全移除 `RTCMTLVideoView`。
- 固定 decode 后渲染主线为 `RTCVideoRenderer -> latest-only mailbox -> render scheduler -> Metal presenter/effect pipeline`。
- 固定 Device 零拷贝合同为 `RTCCVPixelBuffer/CVPixelBuffer -> CVMetalTextureCache -> MTLTexture -> CAMetalLayer/MTKView`，实现 0 CPU 像素拷贝。
- Simulator 对 libwebrtc 软件解码产生的 I420 buffer 提供受条件编译约束的三平面 Metal 上传兼容路径。
- 将 `.playing` 门禁从“surface ready + first frame”升级为“control ready + peer connected + first frame accepted + first real present”。
- 补齐 `displayHealthSnapshot` 与 presentation trace，使 iOS 能输出 submit/present、drop、displayed delta 与长尾事实。

## Scope

- In scope:
  - [iosapp/XBXRC/Platform/Streaming/StreamingPlayerView.swift](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamingPlayerView.swift)
  - [iosapp/XBXRC/Platform/Streaming/LibWebRtcRuntime.swift](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/LibWebRtcRuntime.swift)
  - [iosapp/XBXRC/Platform/Streaming/StreamSessionActor.swift](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamSessionActor.swift)
  - [iosapp/XBXRC/Platform/Streaming/StreamingRuntime.swift](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamingRuntime.swift)
  - iOS 新增渲染模块：自定义 `RTCVideoRenderer`、frame mailbox、display scheduler、Metal presenter、effect pipeline 骨架
  - `iosapp/scripts/check-streaming-core-trace.py` 与相关 tests
  - iOS presentation trace / display health DTO
- Out of scope:
  - Rust 控制面、session flow、SDP/ICE orchestration 主线改写
  - 自建 VideoToolbox decoder 或绕过 libwebrtc 的第二套 decode pipeline
  - 音频 playout、麦克风、输入 DataChannel 和 rumble
  - HDR、HEVC、AV1、时域超分、motion vector / depth history
  - 跨平台统一抽象到 Rust native video presenter

## Design

### 1. 目标主线

```text
RTCVideoTrack
  -> IosZeroCopyVideoRenderer
  -> LatestFrameMailbox(latest-only)
  -> MetalRenderScheduler
  -> MetalEffectPipeline
  -> CAMetalLayer / MTKView
  -> present telemetry / displayHealthSnapshot
```

### 2. Device 零拷贝合同与 Simulator 兼容边界

Device 主线固定遵守以下合同：

1. libwebrtc 继续承担远端视频解码，iOS App 只消费 decode 后 frame。
2. Device 主线只接受 `RTCCVPixelBuffer`，底层像素载体固定为 `CVPixelBuffer`。
3. 禁止 `toI420()`、`UIImage`、`CGImage`、`CGContext`、CPU colorspace convert、CPU resize、CPU memcpy。
4. mailbox 只保存 frame 引用和元数据，不保存 CPU 像素副本。
5. Metal 侧通过 `CVMetalTextureCacheCreateTextureFromImage` 把 `CVPixelBuffer` 映射成 `MTLTexture`。
6. effect pass、超分 pass、present pass 都在 GPU 上完成。

Simulator 兼容路径固定遵守以下合同：

- 仅在 `targetEnvironment(simulator)` 下接受 libwebrtc 软件解码产生的 `RTCI420BufferProtocol`。
- mailbox 强引用 I420 buffer，render tick 按 `dataY/U/V + strideY/U/V` 上传三个 `.r8Unorm`、`.shared` Metal texture。
- I420 texture 继续进入同一套 latest-only mailbox、command buffer submit/present 和 telemetry 合同。
- trace 固定写入 `renderer=metalSimulatorI420Compatibility` 与 `compatibilityPath=simulatorI420Upload`，使运行态能够区分兼容路径。
- Device 编译产物保持 `CVMetalTextureCache` 零拷贝路径，未知 buffer 与无效 pixel format 继续进入结构化 drop/diagnostic。

### 3. 组件职责

#### 3.1 `IosZeroCopyVideoRenderer`

- 实现 `RTCVideoRenderer`
- 接收 `RTCVideoFrame`
- 提取 `RTCCVPixelBuffer`
- 生成 `DecodedFrameRef`
- 写入 `LatestFrameMailbox`
- 产出 `videoFrameReceived` / `videoFrameRejected` / `mailboxOverwrite` trace

#### 3.2 `DecodedFrameRef`

- 持有 `CVPixelBuffer`
- 持有 `width/height/rotation/timestampNs`
- 记录 `attemptId/generation/peerEpoch`
- 持有 `receiveAtMs/acceptedAtMs`
- 携带 colorspace / pixel format / plane metadata

#### 3.3 `LatestFrameMailbox`

- 状态固定为 `inflightCurrent + latestCandidate`
- 最新 steady frame 可以覆盖旧 candidate
- 恢复锚点、clean anchor、post-IDR ramp-up frame 享有更高保留价值
- scheduler / drawable 忙时只保留最新候选，不排空历史

#### 3.4 `MetalRenderScheduler`

- 持有 display tick 与 immediate render tick
- `submit` 后立即请求一次 render tick
- 单拍最多消费一个 candidate
- drawable 忙时记录 retained / skipped / overwrite 事实
- 负责 submit/present 时间戳、present epoch 与 displayed delta

#### 3.5 `MetalEffectPipeline`

- 输入：由 `CVMetalTextureCache` 导入的 decode texture
- pass 0：YUV plane sampling / color conversion / rotation / scale
- pass 1：可选 sharpen / SR / post-processing
- pass 2：present to drawable
- 第一阶段只要求 pass 骨架和 clear direct path，SR 算法在后续 phase 增量接入

### 4. `RTCMTLVideoView` 退场策略

- App target 主路径完全删除 `RTCMTLVideoView` 的创建、attach、delegate 和 trace 依赖。
- [StreamingPlayerView.swift](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamingPlayerView.swift) 改为承载自定义 Metal view。
- 当前 `videoSurfaceAttached` / `videoSurfaceSized` / `videoSurfaceRendererReady` 事件保留语义，底层来源切到新 presenter。
- `.playing` 门禁从 `videoSurfaceRendererReady` 升级为 `displayPipelineReady + firstFrameSubmitted + firstFramePresented`。

### 5. Presentation telemetry 合同

`displayHealthSnapshot` 至少补齐：

- `submitCount`
- `presentCount`
- `lastSubmitAtMs`
- `lastPresentAtMs`
- `submitToPresentP50Ms`
- `submitToPresentP95Ms`
- `displayedFrameDelta`
- `mailboxOverwriteCount`
- `dropCountByReason`
- `currentDrawableBusy`

trace 需要新增或替换为：

- `displayPipelineReady`
- `videoFrameAcceptedToMailbox`
- `videoFrameDropped`
- `displaySubmit`
- `displayPresent`
- `displayPresentSkipped`
- `displayFirstPresent`
- `displayHealthSnapshot`

### 6. 与仓库既有主线的对齐

- decode 前继续由 libwebrtc / RTP / NACK / TWCC / receive recovery 保持顺序与修复语义。
- decode 后执行模型对齐仓库既有 latest-only mailbox、local drop、host cadence、display scheduling 母线。
- clean anchor、DisplayStable、recovery epoch 的权威事实保持在现有 receive / session / policy 边界，iOS presenter 只消费这些事实作为显示价值输入。

## Current vs Target

改造前：

- `RTCVideoTrack -> RTCMTLVideoView`
- decode 后调度与 present 黑盒化
- `displayHealthSnapshot=unsupported`
- `.playing` 无法证明真实 present
- 超分与效果链缺少可控入口

改造后：

- `RTCVideoTrack -> 自定义 renderer -> mailbox -> scheduler -> Metal presenter`
- 0 CPU 拷贝导入和 GPU effect pipeline 固定化
- submit/present/overwrite/drop 均有结构化 telemetry
- `.playing` 绑定 first real present
- 后续 SR / low-latency / local drop 在同一条主线上迭代

## Plan

1. 固定 iOS 零拷贝合同、trace 事件、display telemetry DTO 与 `.playing` 新门禁。
2. 新增 `IosZeroCopyVideoRenderer + DecodedFrameRef + LatestFrameMailbox`，从 `RTCVideoTrack` 接帧。
3. 新增 `MetalRenderScheduler + Metal presenter`，跑通 `CVPixelBuffer -> CVMetalTextureCache -> present`。
4. 接入 latest-only 策略、drop reason 与 present cadence telemetry。
5. 从 App 主路径删除 `RTCMTLVideoView`，切换到自定义 Metal view。
6. 补齐 trace gate、Swift tests、手工 Simulator/Device trace 验收。

## Validation

- [x] `find iosapp/XBXRC iosapp/XBXRCTests -name '*.swift' -print0 | xargs -0 xcrun swiftc -parse`
- [x] iPhoneSimulator / iPhoneOS Swift 6 `strict-concurrency=complete`、`warnings-as-errors` typecheck 覆盖新 presenter 与 renderer
- [x] `python3 -B -m py_compile iosapp/scripts/check-streaming-core-trace.py iosapp/scripts/tests/test_check_streaming_core_trace.py`
- [x] 新增 presentation context 与 trace gate 定向测试
- [x] `git diff --check`
- [x] fresh Simulator trace：出现 `displayFirstPresent`，`displayHealthSnapshot.presentCount > 0`
- [ ] fresh Device xHome trace：`peerConnected -> controlReady -> firstVideoFrame -> displayFirstPresent -> steadyMediaObserved`
- [ ] fresh Device xCloud trace：`peerConnected -> controlReady -> firstVideoFrame -> displayFirstPresent -> steadyMediaObserved`

## Risks

- libwebrtc 在 Device 上交付非 `RTCCVPixelBuffer` 时会进入结构化 drop 与显式诊断；Simulator I420 通过独立 compatibility marker 接入受控上传路径。
- `CVMetalTextureCache` 导入、plane format、colorspace/range 映射如果处理不完整，可能出现黑屏、偏色、拉伸或 UV 错位。
- 去掉 `RTCMTLVideoView` 后，现有 `videoSurfaceRendererReady` 只证明尺寸有效；新的 first present 合同需要同步替换 gate 与 trace analyzer。
- `CAMetalLayer`/`MTKView` 的 drawable 生命周期、display link 节拍与 App 前后台切换需要单独验证，防止 retained old drawable 把问题转移到 presenter。

## Progress

- [x] Step 1: 固定零拷贝合同与 display telemetry 字段
- [x] Step 2: 新增自定义 `RTCVideoRenderer` 与 mailbox
- [x] Step 3: 新增 Metal presenter 与 scheduler
- [x] Step 4: 删除 `RTCMTLVideoView` 主路径
- [x] Step 5: 补 trace gate 与定向测试
- [ ] Step 6: fresh Simulator / Device trace 验收

## Execution Notes

- Date: 2026-07-24 | Status: planned
- Update: 用户确认 iOS 串流渲染主线直接移除 `RTCMTLVideoView`，并要求渲染链按 0 拷贝实现。
- Decision: iOS 渲染主线固定为 `RTCCVPixelBuffer/CVPixelBuffer -> CVMetalTextureCache -> Metal presenter`，主路径禁止 CPU fallback。
- Decision: decode 后执行模型对齐仓库既有 latest-only mailbox 与 low-latency display scheduling，不引入 FIFO backlog。
- Decision: `.playing` 升级为真实 present 合同，`displayHealthSnapshot` 从 `unsupported` 升级为事实投影。
- Risk/Blocker: 当前现场黑屏已收敛到 presentation；自定义 presenter 落地前，iOS 端仍缺真实 present 证据与 first present gate。
- Date: 2026-07-27 | Status: in_progress
- Update: 已新增 `StreamingMetalRenderer.swift`，用 `MTKView + RTCVideoRenderer + latest-only mailbox + CVMetalTextureCache` 替换 App 主路径里的 `RTCMTLVideoView`。
- Update: `StreamingFeatureStore`、`StreamSessionActor` 和 trace gate 已切到 `displayPipelineReady -> firstFrameAccepted -> firstFrameSubmitted -> displayFirstPresent` 合同，`displayHealthSnapshot` 改为消费真实 submit/present telemetry。
- Validation: `plutil -lint iosapp/XBXRC.xcodeproj/project.pbxproj`、全量 Swift parse、`python3 -m py_compile iosapp/scripts/check-streaming-core-trace.py iosapp/scripts/tests/test_check_streaming_core_trace.py`、`python3 -m unittest iosapp/scripts/tests/test_check_streaming_core_trace.py`、`git diff --check` 已通过。
- Risk/Blocker: `xcodebuild` 在当前沙箱环境下仍被 CoreSimulator 和 SwiftPM/Xcode 缓存写权限拦住，真机/模拟器运行态 trace 还需要在本机 Xcode 环境补最后验收。
- Date: 2026-07-27 | Status: in_progress
- Update: 根据 Xcode build 诊断修正 renderer 初始化顺序与 Swift 6 completion-handler 发送边界；scheduler 在属性声明处完成初始化，Metal completion 只向主 actor 传递完成状态与 Sendable 帧引用，renderer/scheduler 的跨线程合同已显式声明。
- Validation: 全量 App 源码已在 iOS Simulator 26.1 SDK、真实 WebRTC M137 framework、Swift 6 `strict-concurrency=complete` 与 `warnings-as-errors` 下成功生成 `XBXRC.swiftmodule`；截图中的 5 个编译诊断全部清零。
- Date: 2026-07-27 | Status: in_progress
- Root Cause: 现场 trace 中 `peerConnected`、`controlReady`、`displayPipelineReady` 与 `firstVideoFrame` 均成立，但 `submitCount=0`、`presentCount=0`；Simulator 软件解码实际交付 I420，renderer 当时只接受 `RTCCVPixelBuffer`，帧在进入 mailbox 前被丢弃。SwiftUI 对相同 presentation context 的重复 update 还会清空 mailbox、telemetry 与首帧标记。
- Update: Simulator 新增条件编译的 I420 三平面 Metal 上传路径，沿真实 command buffer 完成 `videoFrameAcceptedToMailbox -> displaySubmit -> displayPresent -> displayFirstPresent`；相同 presentation context 更新改为幂等，避免重复 reset。Device 继续使用 `RTCCVPixelBuffer -> CVMetalTextureCache` 零拷贝主线。
- Validation: trace gate Python 14 项测试、全量 Swift parse、Simulator/Device Swift 6 严格 typecheck、Python compile、PBX plist lint、streaming session boundary gate 与 `git diff --check` 均通过。完整 `xcodebuild` 的沙箱外审批因自动审批服务 503 未完成，fresh Simulator xCloud trace 仍待运行态验收。
- Review: frame identifier 改为跨 context 单调递增，GPU completion 只允许更新当前 context，避免旧 command buffer 污染新会话 mailbox/telemetry；presentation sink 的读取与调用统一进入 MainActor；Simulator pipeline-ready 改为按首帧实际 storage 校验 I420 或 CVPixelBuffer 资源。
- Validation: 上述并发与 readiness 修正后，arm64 iOS Simulator 和 arm64 iPhoneOS 的全 App Swift 6 strict typecheck 再次通过。
- Date: 2026-07-31 | Status: in_progress
- Evidence: 最新 Simulator xCloud trace 已出现 `renderer=metalSimulatorI420Compatibility`，并在 `peerConnected` 后收到 `inboundVideoBytes=321937 / packetsReceived=311 / receiveBitrateBps≈2.56Mbps`；`framesDecoded=0` 且缺少 `firstVideoFrame / displaySubmit / displayPresent`，证明 RTP 已到达、renderer 尚未收到 libwebrtc 解码帧。
- Root Cause: iOS recvonly video transceiver 调用了 `rtpSenderCapabilities(forKind:)`；桌面合同使用 `RTCRtpReceiver.getCapabilities('video')`，WebRTC M137 同时提供 `rtpReceiverCapabilitiesForKind:`。sender/receiver capability 错配会让 offer 偏好与 `RTCDefaultVideoDecoderFactory` 的真实接收能力脱节。
- Update: video codec preferences 改由 receiver capabilities 产生，H264/RTX/RED/ULPFEC 排序继续作用于真实可接收集合；音频 sendrecv 继续使用 sender capabilities。解码职责保持在 libwebrtc，Metal renderer 继续只消费 `RTCVideoFrame`。
- Review: runtime 保存实际挂载 probe 的 `RTCVideoTrack` wrapper，重复 attach 与 close 均对同一 wrapper 精确解绑；attachment active gate 阻止 close 后迟到 `didStartReceiving` 重新挂载。stats collection 增加 single-flight 与 generation，native callback 未返回时不再堆积请求，旧 peer callback 无法污染新会话投影；first-frame probe 增加线程安全的一次性门禁。
- Validation: `python3 -B -m unittest iosapp/scripts/tests/test_check_streaming_core_trace.py`（14 passed）、全量 Swift parse、`plutil -lint iosapp/XBXRC.xcodeproj/project.pbxproj`、`iosapp/scripts/check-streaming-session-boundary.sh` 与 `git diff --check` 通过。完整 Simulator xcodebuild 申请因审批服务 503 未执行。
- Date: 2026-08-04 | Status: in_progress
- Evidence: fresh Simulator xCloud 已输出 `peerConnected -> controlReady -> firstVideoFrame`，随后 Thread 43 `IncomingVideoStream` 在 `_dispatch_assert_queue_fail` 触发 `EXC_BREAKPOINT`；`videoFrameAcceptedToMailbox / displaySubmit / displayFirstPresent` 尚未发生。
- Root Cause: `StreamingMetalVideoSurfaceView` 继承 `MTKView` 并直接实现 `RTCVideoRenderer`，UIKit/MainActor 隔离随类型继承；libwebrtc 在私有视频串行队列同步调用 `renderFrame`，Swift 6 executor check 因队列不匹配主动中止。
- Update: 将 `RTCVideoRenderer` 拆为独立 `StreamingVideoFrameIngress`，后台回调通过线程安全 latest-only slot 合并待处理帧，再以单个 MainActor drain 交给现有 decoded mailbox 与 Metal presenter；surface 只保留 UI/drawable/present 职责。ingress 覆盖帧计入 `ingressCoalesced` drop telemetry，context 切换同步清空 pending frame。
- Review: Swift 6 warnings-as-errors typecheck 同时识别出 `didStartReceivingOn` 第一个参数标签未满足 ObjC optional delegate 签名；现已改为 `_ peerConnection`，恢复原生 receiver track callback，answer/stats 主动扫描继续作为 fallback。
- Validation: arm64 iPhoneSimulator 全 App 在 Swift 6 `strict-concurrency=complete`、`warnings-as-errors` 下 typecheck 通过；全量 Swift parse、streaming trace gate 14 项、session boundary、PBX/Info.plist lint 与 `git diff --check` 通过。XCTest 全量 typecheck 仍包含既有 CloudLibraryStore actor 标注和两个显式 return 诊断；完整 xcodebuild 的沙箱外审批服务返回 503。
- Date: 2026-08-05 | Status: in_progress
- Evidence: 真机与 Simulator 都在连接成功进入播放时崩溃；代码审计发现 drawable 尚不可用时 `handleSkippedPresent(requeueAsLatest: true)` 会立即 `requestTick`，同一帧在 MainActor 上无节拍自旋。旧实现的 CADisplayLink callback 每拍还会额外创建 Task，放大主线程队列压力。
- Root Cause: retry 调度缺少“等待下一 display tick”边界；requeue 同时会用失败旧帧覆盖已到达的新 candidate，破坏 latest-only；scheduler/displayLink 在 surface dismantle 后缺少显式 invalidate，重复会话会遗留 run-loop owner。
- Update: drawable/command failure 的 requeue 改为等待 CADisplayLink；scheduler 固定 MainActor 并直接 draw，active gate 阻止离屏 draw；dismantle 显式 invalidate；mailbox 在已有新 candidate 时丢弃失败旧帧；失败 command 清理 pending submit telemetry，成功 completion 在 drawableBusy 复位后再拉取 pending frame。
- Validation: 修复后 arm64 iPhoneSimulator 全 App Swift 6 `strict-concurrency=complete`、`warnings-as-errors` typecheck 通过；全量 Swift parse、streaming trace gate 14 项、session boundary、PBX/Info.plist lint 与 `git diff --check` 通过。沙箱外完整 xcodebuild 审批服务返回 400，fresh 真机/Simulator 运行态继续承担最终验收。
- Date: 2026-08-05 | Status: in_progress
- Evidence: fresh Simulator xCloud trace `runtime-trace-ios-1785895993966-1.jsonl` 已完成 `firstVideoFrame -> displayFirstPresent -> steadyMediaObserved`；最终 decode 4376、submit/present 4375/4375，稳态 frame supply P50/P95 30/31fps，submit-to-present P50/P95 1.584/1.969ms，报告丢包 0%，综合质量评分 84/100。
- Validation: schema、sequence、privacy、budget、coverage 均 PASS；Simulator presentation 验收完成。严格 streaming core gate 仅剩 `terminalSelected` 后 3ms 晚到 1 次 `displayPresent`；启动到首显 27.283s、receive-side 恢复字段与 fresh Device xHome/xCloud 继续保留为收口项。
