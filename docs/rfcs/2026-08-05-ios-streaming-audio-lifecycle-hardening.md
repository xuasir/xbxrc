# iOS 串流接收音频生命周期加固 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: in-progress
- Owner: agent
- Last Updated: 2026-08-05
- Execution Gate: 用户已确认一期范围并授权执行

## Background

- iOS xCloud/xHome 已通过 libwebrtc 接收 Opus 音频，并由 `RTCAudioSession` 激活系统 playout。
- 当前实现只在建连时同步激活、关闭时同步释放；系统 interruption、route change、media services lost/reset 尚未进入明确状态机。
- 现有 trace 记录激活时的 route、sample rate 与 I/O buffer，缺少音频中断、恢复边沿、路由切换结果和幂等释放证据。
- 桌面端已有可观测音频输出与延迟控制；iOS 一期优先补齐 Apple 平台生命周期和真实设备稳定性。

## Goal

1. 建立单一、可测试、代际安全的 iOS 串流音频生命周期 owner。
2. 在 interruption、route change、media services lost/reset 后按当前活跃会话事实恢复远端音频 playout。
3. 保证 start、peer rebuild、stop、后台退出和快速重启期间激活/释放幂等，旧回调无法恢复新会话或已结束会话。
4. 用结构化 trace 和确定性测试证明音频 route、状态迁移、恢复尝试及清理结果。

## Scope

### In Scope

- `iosapp/XBXRC/Platform/Streaming` 内的接收音频生命周期协调器、纯状态/策略类型及 WebRTC runtime 适配。
- `RTCAudioSessionDelegate` 与 `AVAudioSession` notification 的统一事件入口。
- 处理 interruption began/ended、route change、media services lost/reset。
- 音频 session 的激活、重新配置、恢复、停用、observer/delegate 注销与 generation gate。
- route、category/mode、sample rate、I/O buffer、secondary audio、恢复结果和失败分类的脱敏 trace。
- XCTest 覆盖状态迁移、重复通知、旧 generation、stop 后晚到回调、恢复失败与快速重启。
- fresh Device xCloud/xHome 的扬声器、有线/蓝牙 route、中断恢复和退出清理验收。

### Out Of Scope

- 麦克风采集、party chat、上行 audio track 与回声消除 UX；权限边界仅保留为未来上行能力的显式申请入口。
- 自定义 Opus 解码、PCM tap、AudioUnit/AVAudioEngine 平行播放管线。
- 用户音量/静音控制、锁屏媒体控制和 AirPlay 产品能力扩展。
- 音画同步控制、underrun 策略和长期 playout latency 调优。

## Design

### 1. Ownership

- `LibWebRtcRuntime` 继续持有 `RTCPeerConnection` 和 libwebrtc audio playout。
- 新增单一音频生命周期协调器，持有通知订阅、音频 generation 和当前激活状态。
- 协调器只通过窄 adapter 配置 `RTCAudioSession`，不拥有 PeerConnection、session flow 或媒体数据。
- `makeOffer` 首次建 peer 时启动音频生命周期；同 peer ICE restart 保持当前 owner；peer rebuild 复用当前会话 generation；terminal cleanup 先终止恢复资格，再释放音频资源。

### 2. State Model

目标状态固定为：

- `idle`：无活跃串流音频 owner。
- `activating`：正在配置并激活系统音频 session。
- `active`：远端 playout 可用。
- `interrupted`：系统中断期间暂停恢复动作。
- `mediaServicesLost`：系统媒体服务丢失，等待 reset。
- `recovering`：当前 generation 正在重新配置和激活。
- `failed`：本次恢复失败，保留可诊断原因；后续有效系统事件允许受控重试。
- `stopping`：终态清理中，拒绝所有晚到恢复事件。

状态迁移由纯策略函数决定，系统 API 调用作为 effect 执行，便于确定性测试。

### 3. System Events

- interruption began：记录 route 与原因，进入 `interrupted`。
- interruption ended：仅在系统允许恢复且当前 session/generation 仍活跃时重新激活。
- route change：记录前后 route 和 reason；活跃输出消失时执行一次受控重配置。
- media services lost：使当前激活事实失效，停止即时恢复。
- media services reset：重建 `RTCAudioSession` 配置并恢复当前活跃会话。
- stop/background/auth terminal：先撤销 generation，再移除 observer/delegate 并停用 session。

### 4. Trace Contract

关键事件：

- `audioLifecycleStarted`
- `audioSessionActivationStarted/Succeeded/Failed`
- `audioInterruptionBegan/Ended`
- `audioRouteChanged`
- `audioMediaServicesLost/Reset`
- `audioRecoveryStarted/Succeeded/Failed/Skipped`
- `audioSessionDeactivationStarted/Succeeded/Failed`
- `audioLifecycleStopped`

所有事件携带 attemptId、generation、audioGeneration、route 类型、原因枚举与脱敏错误分类。生产 trace 保持无设备名称、账号、URL 和 token。

## Plan

1. 提取可测试的音频状态、系统事件、恢复决策和 trace payload 合同。
2. 新增音频生命周期协调器及 `RTCAudioSession/AVAudioSession` adapter。
3. 将协调器接入 `LibWebRtcRuntime` 的 makeOffer、peer rebuild 与 closeTransport。
4. 接入 interruption、route change、media services lost/reset 和 `RTCAudioSessionDelegate`。
5. 增加 XCTest、Swift 6 并发门禁、trace gate 和资源释放回归。
6. 完成 Simulator 静态验证与 fresh Device xCloud/xHome 音频矩阵验收。

## Validation

- [x] 全量 Swift parse
- [x] iPhoneSimulator / iPhoneOS Swift 6 `strict-concurrency=complete`、`warnings-as-errors` 定向 typecheck
- [x] XCTest 状态机源码与测试 target 定向 typecheck：通知乱序、重复事件、stale generation、恢复失败、stop 后晚到事件、快速重启
- [ ] `xcodebuild build-for-testing` 与可用 Simulator XCTest
- [x] `bash iosapp/scripts/check-streaming-session-boundary.sh`
- [x] `python3 -B -m unittest iosapp/scripts/tests/test_check_streaming_core_trace.py`（14 passed）
- [x] `plutil -lint` 覆盖 PBX 与 Info.plist
- [x] `git diff --check`
- [ ] fresh Device xCloud：扬声器、有线/蓝牙切换、系统中断、恢复、主动退出
- [ ] fresh Device xHome：扬声器、有线/蓝牙切换、系统中断、恢复、主动退出
- [ ] trace 证明每个 interruption/reset 最多产生一次当前 generation 恢复，terminal 后恢复次数为零
- [ ] 敏感字段扫描为零，音频 observer/delegate 与 session 激活无残留

## Risks

- WebRTC M137 的 `RTCAudioSessionDelegate` 回调与 AVAudioSession notification 可能表达同一系统事件，需要事件归一化和幂等门。
- 蓝牙 HFP/A2DP route 会影响采样率、I/O buffer 和输出质量，首期保持系统事实优先并记录实际 route。
- interruption ended 的 option 与实际 route 可用性存在时序差异，恢复动作需要串行化和受控重试。
- media services reset 后 libwebrtc 内部 audio device 状态可能需要重新启用，必须以真实设备音频恢复和 trace 共同验证。
- 当前工作区包含 iOS Swift 架构加固改动，实施需保留其状态 owner 与 detach-first cleanup 边界。

## Progress

- [x] Step 1: 完成现有音频实现、历史约束和桌面对标审计。
- [x] Step 2: 固定一期范围、owner、状态模型、系统事件和验证矩阵。
- [x] Step 3: 用户已确认一期范围，开始实现。
- [ ] Step 4: 自动验证完成；fresh Device xCloud/xHome 音频矩阵和真实 trace 验收待完成。
- [ ] Step 5: 生成 Report 并收口任务状态。

## Execution Notes

- Date: 2026-08-05 | Status: planned
- Update: 根据 iOS 串流完成度评估，将第一阶段聚焦到接收音频系统生命周期和恢复闭环。
- Decision: 继续使用 libwebrtc/RTCAudioSession 作为唯一音频 playout 主线；Swift 平台层承担 AVAudioSession 生命周期适配。
- Decision: 麦克风、party chat、自定义 PCM 管线、音量 UI 与 AV-sync 调优分别进入后续阶段。
- Decision: 一期 WebRTC 音频方向固定为 `recvonly`，音频会话使用 `playback/default` 且不显式附加仅适用于 `playAndRecord` 的路由 options；每次激活创建方向完整的独立 WebRTC 配置，避免全局共享对象污染。只有未来 `sendrecv` 能力通过统一协调器申请麦克风，拒绝权限时返回业务错误并释放会话。
- Decision: 配置首选属性、激活 session、应用 active-only 通道参数分阶段执行；协调器只记录自己成功取得的 activation lease，后续阶段失败时立即回滚。
- Decision: xHome 直连声明 `NSLocalNetworkUsageDescription`，系统仅在首次访问局域网主机时触发授权；xCloud 和未访问局域网的流程不主动申请。
- Risk/Blocker: 完整真机音频矩阵需要用户可用的 iPhone、蓝牙/有线输出设备和真实 xCloud/xHome 会话。
- Date: 2026-08-05 | Status: in-progress
- Update: 用户确认按 RFC 执行；实现拆分为纯状态机/测试与 libwebrtc/AVAudioSession 平台适配两个边界。
- Date: 2026-08-05 | Status: validation-pending
- Update: 音频状态机、`RTCAudioSessionDelegate` 协调器、runtime 接入、代际绑定 trace 与 12 个状态机测试已实现；Simulator/Device Swift 6 严格 typecheck、全量 Swift parse、PBX/Info lint、streaming boundary、trace gate 14 项和 diff check 已通过。
- Validation: `xcodebuild build-for-testing` 在当前沙箱内受 CoreSimulator、SwiftPM 与 Clang cache 权限限制；沙箱外审批服务返回 503，本轮保留完整 Xcode build 与 Simulator XCTest 为外部门禁。
- Update: 复核固定 WebRTC M137 `RTCAudioSession+Configuration` 源码与 Apple category option 合同后，移除 `playback + allowAirPlay` 非法组合、共享配置原地修改和无效的 input channel 清零；配置/激活拆分并覆盖失败 lease 回滚，补齐 xHome 本地网络权限声明及方向配置回归测试。
- Validation: 固定 WebRTC artifact 下载、checksum 和 Device/Simulator slice 校验通过；真实 WebRTC Device API 与全 App Swift 6 strict typecheck、全量 Swift parse、Info/PBX lint、streaming boundary、Rust 50 项、trace gate 14 项、trace analyzer 3 项和 diff check 通过。完整 Xcode build 受本机 iOS 26.5 destination 判定异常阻断；XCTest 手工 typecheck命中既有 actor/显式 return 诊断。
- External Gate: fresh Device xCloud/xHome 需覆盖扬声器、有线/蓝牙 route、interruption 恢复、可触发时的 media services reset、主动退出，并验证 terminal 后恢复次数为零及敏感字段扫描为零。
