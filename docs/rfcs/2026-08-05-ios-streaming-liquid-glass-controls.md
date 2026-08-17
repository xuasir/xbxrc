# iOS 串流 Liquid Glass 快捷操作层 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: validation-pending
- Owner: agent
- Last Updated: 2026-08-05
- Execution Gate: 用户已确认执行 P0/P1；手柄相关能力延期

## Background

- iOS 串流页当前由全屏视频、连接状态、失败状态和退出按钮组成，[`StreamingPlayerView.swift`](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamingPlayerView.swift) 尚未提供串流中的快捷控制层。
- 桌面串流页已经具备 Xbox 键、音频、麦克风、发送文字、显示设置、体验指标、内部诊断、超分实验和会话退出等入口，[`Stream.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/Stream.vue) 已将用户操作与诊断入口分层。
- iOS 已采用 SwiftUI、iOS 26 原生 `GlassEffectContainer` / `glassEffect`、libwebrtc 和自定义 Metal 上屏主线，现有技术边界可以承载原生 Liquid Glass 操作层。
- 串流画面是播放页唯一主内容。用户需要在一跳内完成高频动作，工程排障需要保留完整 RTC、媒体、恢复、音频和上屏证据。
- iOS 接收音频生命周期加固正在实施，本 RFC 复用其音频状态与 route 事实，避免建立第二套音频状态源。

## Goal

1. 建立默认隐藏、快速唤出、低遮挡的 iOS 串流操作层。
2. 使用双指从屏幕两侧向中心划入作为主快捷手势，单击画面和手柄入口提供辅助唤出路径。
3. 用户层只展示高频动作、简短状态和明确结果，每个高频动作在唤出后一次点击内完成。
4. 调试层完整展示网络健康、恢复链、media supply、steady supply、submit/present 长尾与帧供给 delta，并支持 trace 导出。
5. 通过 capability gate 接入 iOS 已有消费能力；后续能力具备真实控制合同后自动获得明确接入位置。
6. 保持 SwiftUI UI、libwebrtc 媒体、Metal 上屏和 Rust 控制面现有所有权边界。

## Product Principles

### 用户层

- 播放中默认显示纯画面。
- Dock 最多展示 5 个高频入口。
- 高频动作使用图标和即时状态反馈，用户无需阅读指标或工程术语。
- 弱网、恢复、音频中断和手柄断开使用单行短时提示。
- 退出、关机、显示设置和连接概况进入二级用户面板，降低误触风险。
- 不可用能力由 capability gate 隐藏，用户看到的每个控件都具有真实消费点。

### 调试层

- 调试信息从二级用户面板进入，并受现有诊断配置控制可见性。
- 调试页允许高信息密度、原始枚举、时序值、分位数和状态链。
- 每项调试数据标注事实来源与采样时间；缺少来源的字段显示“未采集”。
- 导出动作直接调用现有 iOS Runtime Trace 文件与系统分享能力。

## Scope

### In Scope

- `StreamingPlayerView` 的操作层拆分、可见性状态机和自动隐藏。
- iOS 26 原生 Liquid Glass Dock、用户面板、调试面板、提示条和确认弹窗。
- 双指向内划、单击画面、触觉反馈、手势冲突管理与无障碍入口。
- 用户体验状态投影与详细调试快照的独立数据模型。
- 远端音频、发送文字、画面适应模式、退出和 xHome 关机退出的 capability/action 合同。
- 麦克风、显示效果、截图和超分的能力门与后续接入位。
- RTCStats、会话状态、音频生命周期、Metal 上屏健康和 Runtime Trace 的调试投影。
- Swift/XCTest、真机手势、横屏布局、VoiceOver、Reduce Motion 和 Reduce Transparency 验收。

### Out Of Scope

- 改造 Rust session flow、signaling、ICE、transport 和恢复策略。
- 建立自定义 PCM、AudioUnit 或 AVAudioEngine 播放管线。
- 在原生消费能力具备前实现麦克风、显示效果和超分的占位控制。
- 改动桌面端串流信息架构。
- 引入第二套 iOS runtime、media pipeline 或 UI 框架。
- Xbox 键、手柄快捷唤出、手柄焦点路由和震动 UI。

## User Information Architecture

### 1. 默认播放态

- 全屏视频占满页面。
- 操作层在进入 `playing` 后自动隐藏。
- 顶部提示只显示当前需要用户理解的事件：`连接波动`、`正在恢复画面`、`音频已中断`、`手柄已断开`。
- 提示恢复后自动消失，常态下不保留状态徽标和性能数字。

### 2. Liquid Glass Dock

Dock 固定最多 5 个入口，按当前能力动态组成：

1. 声音：点击静音/恢复；长按或二次展开进入系统音量与输出 route 控制。
2. 发送文字：xHome 会话中点击直接打开原生输入面板。
3. 显示：点击在适应与填充之间切换。
4. 更多：打开二级用户面板。

布局规则：

- 使用单个 `GlassEffectContainer` 承载整组控件。
- 使用 SF Symbols、稳定 44pt 以上触控区域和短标签辅助 VoiceOver。
- 当前状态通过图标形态、系统 tint 和触觉反馈表达。
- Dock 展开和收起使用原生 glass morphing；Reduce Motion 下使用短淡入淡出。
- 视频画面继续作为背景采样源，页面不增加装饰卡片和永久遮罩。

### 3. 二级用户面板

用户面板只承载可理解、可执行的内容：

- 连接：`良好 / 波动 / 恢复中`。
- 声音：静音状态、当前输出 route、系统音量控件。
- 显示：`适应 / 填充` 分段控制。
- 会话：退出串流；xHome 且具备主机命令上下文时展示关机并退出。
- 调试信息：诊断配置允许时展示，进入独立详细页。

退出和关机并退出使用独立确认弹窗。普通退出保持主动作，关机并退出使用危险动作样式。

### 4. 调试信息页

调试页按事实域分组：

| 分组 | 字段 |
| --- | --- |
| 会话 | target、attempt/generation/peer epoch、会话状态、启动阶段、连接与首帧耗时 |
| 网络 | ICE/route、candidate pair、RTT、jitter、丢包、接收码率、packets、NACK/PLI/FIR |
| 恢复 | 当前恢复阶段、触发原因、关键帧进展、重连代际、可用恢复事实与阻塞项 |
| Media supply | frames received/decoded/dropped、receive/decode FPS、最近媒体增量、音视频 track readiness |
| Steady supply | steady media 状态、连续供给窗口、最近有效帧年龄、供给中断次数 |
| 上屏 | submit/present 计数、present FPS、submit-to-present P50/P95/P99、drawable busy、local drop、帧供给 delta |
| 音频 | lifecycle state、route、sample rate、I/O buffer、interruption/reset、最近恢复结果 |
| 控制 | data channel 状态、输入可用性、发送文字与 Xbox 动作结果 |

调试页提供刷新、复制诊断摘要和导出 Runtime Trace。字段直接读取已有事实源；需要新增采样的字段通过现有 trace/stats 合同扩展。

## Gesture Contract

### 双指向内划

- 两个触点分别从屏幕左右 24% 区域开始。
- 两指开始时间差不超过 120ms。
- 两指水平移动方向相向，各自移动阈值为 `72...120pt`，按屏幕宽度自适应。
- 垂直漂移不超过 40pt，完成时间不超过 450ms。
- 识别成功后提供一次轻触觉反馈并显示 Dock。
- 识别阈值达到前保持视频 surface 的触摸传递；识别成功后消费当前手势。
- `playing` 状态开放识别；sheet、键盘、系统权限弹窗和终止流程期间暂停识别。

### 辅助入口

- 单击画面切换 Dock 可见性。
- VoiceOver 用户通过可访问性动作直接打开“串流控制”。

### Trace

- `streamControlsGestureStarted`
- `streamControlsGestureRecognized`
- `streamControlsGestureRejected`
- `streamControlsVisibilityChanged`
- `streamControlInvoked`
- `streamControlCompleted`

手势 trace 只记录手势类型、结果、耗时、拒绝原因和页面状态，坐标与触点原始轨迹保持在进程内。

## State Model

操作层状态固定为：

- `hidden`：纯画面播放。
- `revealing`：手势命中后的入场动画。
- `visible`：Dock 可操作，3 秒无交互后进入隐藏流程。
- `userSheetPresented`：用户面板展示，自动隐藏计时暂停。
- `debugSheetPresented`：调试页展示，统计刷新按可见性启停。
- `confirmingDestructiveAction`：退出或关机确认。
- `hiding`：收起动画，完成后回到 `hidden`。

滑杆、文本输入、面板滚动和确认弹窗会续期可见性。会话离开 `playing` 后清理 overlay 状态、键盘、计时器和待执行动作。

## Architecture

### UI Boundary

- `StreamingPlayerView` 只负责视频 surface、状态层和操作层组合。
- 新增独立 overlay state model，持有可见性、sheet、确认动作和自动隐藏时钟。
- Liquid Glass Dock、用户面板、调试页和提示条拆成独立 SwiftUI 视图。
- 专用 `UIGestureRecognizer` 负责双指轨迹识别，通过 `UIViewRepresentable` 与视频 surface 协作。

### Action Boundary

- `StreamingFeatureStore` 对 UI 暴露 `StreamingControlCapabilities`、`StreamingUserExperienceState` 和动作方法。
- 用户动作沿 `StreamingFeatureStore -> StreamSessionActor -> StreamingPeerRuntime/StreamingControlSession` 进入现有所有权边界。
- UI 不直接持有 `RTCPeerConnection`、data channel、音频 session 或 Metal renderer。
- 每个动作携带当前 generation/attempt 上下文，晚到结果只能更新对应会话。

### Projection Boundary

- `StreamingUserExperienceState` 只包含简短状态、可恢复提示和当前用户动作结果。
- `StreamingDebugSnapshot` 包含原始 RTC、恢复、媒体、上屏、音频和控制字段。
- 用户投影和调试投影分别测试，防止内部字段回流到用户层。
- 调试页可见时启动高频 stats 投影；隐藏时保持低频 trace 与现有运行时采样预算。

## Desktop Capability Mapping

| 桌面能力 | iOS 方案 | 首次落地阶段 |
| --- | --- | --- |
| 音量/静音 | 远端音轨静音 + 原生系统音量控件 + route 状态 | Phase 1 |
| 麦克风 | 上行音轨、权限和控制协议闭环具备后由 capability gate 展示 | Phase 3 |
| 发送文字 | 原生输入 sheet，经已有文本控制通道发送 | Phase 1 |
| 显示设置 | 首期提供适应/填充；Metal effect 有消费点后加入画面参数 | Phase 1/3 |
| 体验 HUD | 用户层投影为简短连接状态；详细数值进入调试页 | Phase 1 |
| 内部诊断 | iOS 原生详细调试页 | Phase 2 |
| Trace 导出 | 调试页直接调用系统分享 | Phase 2 |
| 全屏 | iOS 播放页现有沉浸式横屏状态 | 已具备 |
| 退出 | 用户面板确认后结束当前 generation | Phase 1 |
| 关机并退出 | xHome capability 可用时调用主机命令并结束会话 | Phase 2 |
| 超分 | Metal effect pipeline 与性能保护具备后展示 | Phase 3 |

## Plan

### Phase 1：可用的快捷操作闭环

1. 提取 overlay 状态机、自动隐藏时钟和纯逻辑手势判定。
2. 落地 Liquid Glass Dock、用户面板、短时提示和退出确认。
3. 接入双指向内划、单击、触觉反馈、VoiceOver 和手势 trace。
4. 接入远端音频静音、系统音量、发送文字和动作结果反馈。
5. 接入普通退出、连接概况和适应/填充本地显示动作。
6. 建立简洁用户体验投影，接入现有会话、音频 lifecycle 和基础网络状态。

### Phase 2：详细调试与会话增强

1. 落地独立调试页和可见性门控的高频 stats 投影。
2. 接入 RTCStats、Metal health、恢复状态、media supply、steady supply 和 submit/present 长尾。
3. 接入诊断摘要复制和 Runtime Trace 导出。
4. 为 xHome 增加 capability-gated 关机并退出。
5. 完成真机多触点误触验证。

### Phase 3：具备真实消费点后的增强

1. 接入麦克风权限、上行音轨、静音、系统中断和会话控制状态。
2. 接入亮度、对比度、锐化和色彩等 Metal effect 参数。
3. 接入截图、超分、质量档位、性能保护与对比导出。

## Validation

- [x] overlay 状态模型覆盖显示、sheet、确认态、返回 Dock 与隐藏迁移；SwiftUI 自动隐藏和 sheet 生命周期已接入
- [x] 双指纯逻辑测试覆盖开始区域、时间差、方向、垂直漂移、时长和自适应移动阈值
- [x] capability gate 由播放态、远端音轨就绪和 xHome target 事实驱动
- [x] 用户投影只返回连接良好、连接波动和正在恢复；RTC、代际与上屏原始字段保留在调试页
- [x] 调试页覆盖网络健康、当前恢复事实、media supply、steady supply、submit/present P50/P95 与帧供给 delta；缺少事实源的恢复链和 P99 显示“未采集”
- [ ] iPhone/iPad 横屏布局在安全区、Home Indicator、Dynamic Type 和长文案下无重叠
- [ ] VoiceOver 可打开操作层、识别所有动作状态并完成退出确认
- [ ] Reduce Motion 与 Reduce Transparency 使用稳定可读的系统降级表现
- [x] 全量 Swift parse
- [x] iPhoneSimulator/iPhoneOS Swift 6 `strict-concurrency=complete`、`warnings-as-errors` 应用源码 typecheck
- [ ] `xcodebuild build-for-testing` 与可用 Simulator XCTest
- [x] `bash iosapp/scripts/check-streaming-session-boundary.sh`
- [x] `plutil -lint` 覆盖 PBX 与 Info.plist
- [x] `git diff --check`
- [ ] fresh Device xCloud/xHome 验证双指误触、Dock 响应、系统手势、音频 route、手柄和退出清理
- [x] 手势 trace 静态检查通过，仅包含类型、结果、耗时、拒绝原因与页面状态，不记录坐标和轨迹

## Acceptance Targets

- 操作层从手势识别成功到可点击的 P95 小于 250ms。
- Dock 高频动作从展开态到执行最多一次点击。
- 播放稳态无永久 UI 覆盖；用户提示最多一行并在状态稳定后自动消失。
- 双指手势在 30 分钟真实游戏操作中误触发次数为 0。
- 操作层隐藏时不启动额外高频 UI stats 刷新。
- 调试页能从同一次会话还原网络、恢复、媒体供给、上屏和音频时间线。

## Risks

- SwiftUI 高层手势与 Metal/UIKit surface 的多触点竞争会影响事件传递，专用 recognizer 需要严格控制识别前后的消费行为。
- iOS 系统边缘手势、Home Indicator 和游戏触控可能形成误触，真机横屏矩阵需要覆盖左右方向与 iPhone/iPad。
- `MPVolumeView`、蓝牙 route 和远端音轨静音表达不同层级状态，用户投影需要明确当前控制对象。
- 调试页高频刷新会增加主线程和 stats 采集负担，可见性门控与采样预算需要进入性能验收。
- 音频生命周期任务正在修改 `LibWebRtcRuntime` 与 `StreamSessionActor`，本任务实施应在其边界稳定后接入或按文件所有权串行执行。
- xCloud 与 xHome 的关机、文本和控制能力存在差异，capability 必须由会话事实驱动。

## Progress

- [x] Step 1: 完成 iOS 播放页、桌面控制层、Liquid Glass 组件和串流诊断能力审计。
- [x] Step 2: 固定“用户层简洁直达、调试层详细完整”的信息架构。
- [x] Step 3: 固定双指向内划合同、操作层状态机、能力映射、阶段计划和验收门。
- [x] Step 4: 用户确认执行 P0/P1，手柄相关能力延期。
- [x] Step 5: 完成 Phase 1/2 的代码实现与当前环境静态/单元门禁。
- [ ] Step 6: 完成 WebRTC artifact Xcode build、Simulator XCTest、fresh Device xCloud/xHome 验收、Report 与任务收口。

## Execution Notes

- Date: 2026-08-05 | Status: planned
- Update: 根据用户反馈压缩用户层信息密度，Dock 限制为 5 个高频入口，详细指标统一迁入独立调试页。
- Decision: 用户层表达动作和结果；调试层表达原始事实、状态链、时序和分位数。
- Decision: 双指左右向中心划入作为主快捷手势，单击画面、手柄和 VoiceOver 作为辅助入口。
- Decision: 每个控件由 capability gate 和真实消费合同共同决定可见性。
- Risk/Blocker: 当前 iOS 音频生命周期加固任务与本任务共享部分串流文件，实施顺序需要保持文件所有权清晰。
- Date: 2026-08-05 | Status: in-progress
- Update: 用户确认实施 P0/P1；当前范围包含操作层、双指手势、声音、文字、显示、退出、用户状态、详细调试和 trace 导出。
- Decision: Xbox 键、手柄唤出、手柄焦点和震动 UI 延期，现有底层手柄输入保持原样。
- Date: 2026-08-05 | Status: validation-pending
- Update: Phase 1/2 代码完成。播放页默认纯画面，双指内划/单击唤出 Liquid Glass Dock；远端音轨静音、系统音量、xHome 文字、适应/填充、退出、关机退出、用户连接状态、详细诊断、摘要复制和 Runtime Trace 导出均已接入真实消费点。
- Update: 双指识别统一使用 `UITouch.timestamp`，增加开始区域/时间差提前拒绝与无坐标 trace；上屏健康只缓存并随 RTC stats 周期发布，避免逐帧拉取 peer debug snapshot。
- Validation: `cargo fmt -p xbox-ios-bridge -- --check`、`cargo check -p xbox-ios-bridge`、Rust streaming 24 tests、iPhoneOS/iPhoneSimulator Swift 6 strict typecheck、全量 Swift parse、streaming session boundary、PBX/Info lint、`git diff --check` 通过。
- Risk/Blocker: 完整 `xcodebuild build-for-testing` 需要解析固定 checksum 的 WebRTC binary artifact。当前沙箱禁止写入 SwiftPM/Clang 用户缓存，沙箱外自动审批服务返回 503；CoreSimulatorService 同时不可用。构建尚未进入 WebRTC 链接与 XCTest 执行。
- Pending: fresh Device 需要验证横屏双指误触、Liquid Glass/Reduce Transparency、系统音量与 route、xCloud/xHome 文本和关机 capability、退出清理及新 trace 敏感字段扫描。
