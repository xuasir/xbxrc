# iOS 零拷贝 Metal 渲染主线替换 `RTCMTLVideoView` RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: planned
- Owner: agent
- Last Updated: 2026-07-24

## Background

- 当前 iOS 串流播放路径在 [StreamingPlayerView.swift](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamingPlayerView.swift) 内将远端 `RTCVideoTrack` 直接挂到 `RTCMTLVideoView`。
- 最新 Simulator home 串流 trace 已证明 session、offer/answer、ICE、control ready、`videoSurfaceRendererReady`、`firstVideoFrame` 与 `framesDecoded` 持续增长都成立，故障面收敛到最终 presentation / visible present，而非协商、会话或 media supply。
- 当前 [StreamSessionActor.swift](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamSessionActor.swift) 的 `displayHealthSnapshot` 仍为 `unsupported`，iOS 端缺少真实 submit/present 证据，无法回答“首帧是否真正显示到屏幕”。
- 产品后续明确需要低延迟上屏、latest-only 本地丢帧避免积压、超分和后处理效果链。`RTCMTLVideoView` 将 decode 后调度、present 时机、纹理导入与 telemetry 封装在 libwebrtc 内部，无法承载这组目标。
- 用户已确认渲染主线直接移除 `RTCMTLVideoView`，切到零拷贝自定义 Metal renderer。

## Goal

- iOS App 主播放路径完全移除 `RTCMTLVideoView`。
- 固定 decode 后渲染主线为 `RTCVideoRenderer -> latest-only mailbox -> render scheduler -> Metal presenter/effect pipeline`。
- 固定零拷贝合同为 `RTCCVPixelBuffer/CVPixelBuffer -> CVMetalTextureCache -> MTLTexture -> CAMetalLayer/MTKView`，实现 0 CPU 像素拷贝。
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

### 2. 零拷贝合同

主线固定遵守以下合同：

1. libwebrtc 继续承担远端视频解码，iOS App 只消费 decode 后 frame。
2. 主线只接受 `RTCCVPixelBuffer`，底层像素载体固定为 `CVPixelBuffer`。
3. 禁止 `toI420()`、`UIImage`、`CGImage`、`CGContext`、CPU colorspace convert、CPU resize、CPU memcpy。
4. mailbox 只保存 frame 引用和元数据，不保存 CPU 像素副本。
5. Metal 侧通过 `CVMetalTextureCacheCreateTextureFromImage` 把 `CVPixelBuffer` 映射成 `MTLTexture`。
6. effect pass、超分 pass、present pass 都在 GPU 上完成。

主线外的异常 buffer 处理固定为：

- `RTCI420Buffer`、未知 buffer 类型、无效 plane 描述进入结构化 drop/diagnostic。
- App 主路径不引入 CPU fallback。
- 若现场需要 debug fallback，单独列为后续实验任务，不进入本 RFC 的交付目标。

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

- [ ] `find iosapp/XBXRC iosapp/XBXRCTests -name '*.swift' -print0 | xargs -0 xcrun swiftc -parse`
- [ ] iPhoneOS App / XCTest Swift 6 strict typecheck 覆盖新 presenter 与 renderer
- [ ] `python3 -m py_compile iosapp/scripts/check-streaming-core-trace.py iosapp/scripts/tests/test_check_streaming_core_trace.py`
- [ ] 新增 iOS renderer/mailbox/scheduler 定向测试
- [ ] `git diff --check`
- [ ] fresh Simulator trace：出现 `displayFirstPresent`，`displayHealthSnapshot.presentCount > 0`
- [ ] fresh Device xHome trace：`peerConnected -> controlReady -> firstVideoFrame -> displayFirstPresent -> steadyMediaObserved`
- [ ] fresh Device xCloud trace：`peerConnected -> controlReady -> firstVideoFrame -> displayFirstPresent -> steadyMediaObserved`

## Risks

- libwebrtc 在个别环境下可能交付非 `RTCCVPixelBuffer` buffer；本 RFC 选择维持零拷贝纯度，异常类型走结构化 drop 与显式诊断。
- `CVMetalTextureCache` 导入、plane format、colorspace/range 映射如果处理不完整，可能出现黑屏、偏色、拉伸或 UV 错位。
- 去掉 `RTCMTLVideoView` 后，现有 `videoSurfaceRendererReady` 只证明尺寸有效；新的 first present 合同需要同步替换 gate 与 trace analyzer。
- `CAMetalLayer`/`MTKView` 的 drawable 生命周期、display link 节拍与 App 前后台切换需要单独验证，防止 retained old drawable 把问题转移到 presenter。

## Progress

- [ ] Step 1: 固定零拷贝合同与 display telemetry 字段
- [ ] Step 2: 新增自定义 `RTCVideoRenderer` 与 mailbox
- [ ] Step 3: 新增 Metal presenter 与 scheduler
- [ ] Step 4: 删除 `RTCMTLVideoView` 主路径
- [ ] Step 5: 补 trace gate 与验收样本

## Execution Notes

- Date: 2026-07-24 | Status: planned
- Update: 用户确认 iOS 串流渲染主线直接移除 `RTCMTLVideoView`，并要求渲染链按 0 拷贝实现。
- Decision: iOS 渲染主线固定为 `RTCCVPixelBuffer/CVPixelBuffer -> CVMetalTextureCache -> Metal presenter`，主路径禁止 CPU fallback。
- Decision: decode 后执行模型对齐仓库既有 latest-only mailbox 与 low-latency display scheduling，不引入 FIFO backlog。
- Decision: `.playing` 升级为真实 present 合同，`displayHealthSnapshot` 从 `unsupported` 升级为事实投影。
- Risk/Blocker: 当前现场黑屏已收敛到 presentation；自定义 presenter 落地前，iOS 端仍缺真实 present 证据与 first present gate。
