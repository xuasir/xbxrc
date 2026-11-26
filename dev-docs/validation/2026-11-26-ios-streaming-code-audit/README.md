# iOS 串流体验代码审查

日期：2026-11-26。基准：`main`，HEAD `37e4fb434039d38062d08e0cf2c9a8030d33b8f8`，以本轮工作树中的现行源文件为准。

分析输入为当前 Swift 串流、触控、诊断代码，共用 Rust 监督实现，以及当前测试中的行为断言。审查范围遵循用户要求，历史问题记录与历史运行日志均排除在分析输入之外。本轮产出为审查结论与静态验收证据。

当前 libwebrtc 默认解码器、CVPixelBuffer→Metal 零拷贝、latest-only 入口/渲染队列、GPU 完成后释放帧槽、同一会话内的有界帧反馈已经形成完整基础。后续优化可沿现有 libwebrtc、Metal 和公共 Rust 监督合同推进。

## 优先推进

### 1. 保留触控按键边沿直到网络发送

确定的代码路径：`TouchInputAccumulator` 保存每批采样的 `transitions`，当前 `testShortTapKeepsBothEdgesWithinOneBatch` 断言 A 键状态依次为 `[16, 0]`，最终 `gamepad` 为 neutral。`StreamingTouchLayoutController.bind` 仅传递 `frame.gamepad`；`StreamInputSession.setTouchLayoutState` 保存最终快照；DataChannel 每轮处理后等待 8ms，再读取输出快照。

因此，同批次短按、快速滑过按钮，或者两次轮询之间完成的按下/释放，存在边沿在网络出口合并为 neutral 的路径。采集端保留的边沿应继续进入有界发送队列，数字按键按顺序消费，模拟轴保留最新值；菜单抑制、布局切换、中立释放与背压沿同一输出规则处理。

证据：[采集与发布](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Features/TouchControls/TouchInputAccumulator.swift:184)、[当前短按测试](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRCTests/TouchLayoutTests.swift:103)、[会话绑定](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamingTouchLayoutController.swift:104)、[快照保存](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamInputSession.swift:360)、[轮询发送](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/XboxStreamDataChannels.swift:389)。

### 2. 给显示停滞独立的健康判据与恢复入口

确定的代码路径：真机 `drawable.addPresentedHandler` 已记录系统呈现时刻；监督样本中的 `presented`、`videoPresentAgeMs` 和诊断面板的呈现 FPS 使用 GPU completion 对应的 `presentCount`/`lastPresentAtMilliseconds`。与此同时，解码 `frameSupplyDelta > 0` 会更新 `lastVideoActivityAtMs`，恢复动作受 `inactivityElapsedMs >= stallThresholdMs` 门槛控制。

解码持续推进、显示停滞的场景会保持视频活跃时间新鲜，现有恢复入口因此可能持续等待。应向公共 Rust 监督传递实际 drawable 呈现 age/delta、提交与解码 delta、上屏 P95/P99，单独判定显示健康，再在现有 Metal surface 中执行有界重置与资源恢复。网络、解码和显示分别提供事实，动作仍由公共监督协调。

证据：[系统呈现回调](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamingMetalRenderer.swift:1375)、[GPU 完成计数](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamingMetalRenderer.swift:1421)、[解码刷新活跃时间](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamSessionActor.swift:722)、[监督样本](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamSessionActor.swift:1089)、[恢复门槛](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamSessionActor.swift:1164)、[共享恢复判定](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/src/session/recovery/supervision.rs:281)。

### 3. 降低主线程上每帧和每次输入采样的固定成本

确定的代码行为：解码帧入口经 MainActor 接入，纹理导入、drawable 获取、命令编码与完成回调处理位于这条执行链；输入轮询也运行在 MainActor。显示 telemetry 每个 GPU completion、每个 drawable presentation 各对最多 240 个延迟样本排序。逐帧 accepted/submit/present 构造 trace payload；轻量记录的采样判定位于 writer 内部，payload 在进入判定前已经构造。

实体手柄每次采样还会重建按钮/轴字典并排序稳定的设备 profile。触控绘制每次遍历所有控件并重建 accessibility elements，现有图标位图缓存可继续复用。

建议第一轮缓存设备 profile 与映射结构，按配置变化刷新；延迟样本用环形缓存，分位数按健康快照频率计算；为 trace 提供提前判定与惰性 payload；触控仅更新发生变化的控件，并复用无障碍元素。后续可评估同一 Metal 管线内的串行渲染队列，将 UIKit 生命周期与绘制工作清晰隔离。

这类成本由代码确认，CPU 占比、输入抖动和帧延迟收益需要真机采样。

证据：[MainActor 帧入口](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamingMetalRenderer.swift:890)、[drawable 与编码](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamingMetalRenderer.swift:1239)、[分位数计算](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamingMetalRenderer.swift:538)、[trace 采样位置](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Shared/Diagnostics/IOSRuntimeTraceWriter.swift:145)、[设备 profile 采样](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamInputSession.swift:167)、[触控重绘](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Features/TouchControls/TouchLayoutSurface.swift:185)。

## 后续优化

### 4. 补齐接收缓冲、解码耗时和一致的采样时间

当前 RTC stats 投影覆盖字节、包/帧计数、RTT、jitter、freeze 和 NACK/PLI/FIR。可扩展 `jitterBufferDelay`、`jitterBufferEmittedCount`、`totalDecodeTime`、重传/修复计数与音频 concealment，以增量计算缓冲耗时和解码耗时。共享监督窗口已经声明 jitter-buffer 字段，当前 accumulator 填充接收/解码/呈现速率、loss、RTT、jitter，其余字段采用默认值。

监督 tick 在异步 `debugSnapshot` 前取得 `now`，随后再读取可经 actor 重入更新的 stats，存在 stats 观测时刻晚于 tick 的 `now`、从而进入 fresh=false 分支的时序窗口。应在异步依赖完成后捕获判定时间与一致的事实快照，保护 3s/15s 连续观察窗口。

这些指标能判断延迟来自接收缓冲、解码还是显示。缓冲参数调整应以当前 SDK 的可用能力和真机对比为依据，保持标准 WebRTC 接收与恢复主线。

证据：[stats 投影](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/LibWebRtcRuntime.swift:1396)、[采样时间与异步边界](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamSessionActor.swift:1062)、[fresh 判定](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamSessionActor.swift:1082)、[窗口填充](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/src/session/recovery/observation.rs:116)。

### 5. 按帧元数据选择 YUV 色彩转换

当前 `makeFragmentUniforms` 根据 full/video range 选择矩阵，两组系数均对应 BT.601。对于带 BT.709 元数据的高清源，这条路径会产生色彩偏差。应读取 CVPixelBuffer 的 YCbCr matrix、色彩原色和 transfer function 附件，选择对应的 SDR 转换矩阵和输出色彩空间，并缓存配置变化。

收益是颜色、肤色和画面层次的准确性，代码依据明确。

证据：[像素范围判定](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamingMetalRenderer.swift:1118)、[固定矩阵](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamingMetalRenderer.swift:1853)。

### 6. 校准实际音频声道、路由与延迟

SDP 支持 Opus stereo；音频 session 配置目前对收听与语音共用 `playAndRecord + voiceChat`，`makeConfiguration` 消费的方向参数被忽略，输出声道、采样率和 I/O buffer 保留 SDK 配置值。实际声道、output latency 和 I/O buffer 已有诊断投影。

应将协商声道与实际输出建立闭环，分别测量扬声器、有线耳机和蓝牙路由；显式设置适合设备的首选声道、采样率和 buffer，并核对最终系统值。当前代码注明 M137 VoiceProcessingIO 在接收播放初始化时同样需要有效输入通道，优化需保留这项 SDK 兼容条件。

音质与延迟收益按路由确认；当前源码支持的是配置与测量改进的判断。

证据：[Opus stereo](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/LibWebRtcRuntime.swift:357)、[session 配置](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamingAudioLifecycleCoordinator.swift:191)、[实际参数](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamingAudioLifecycleCoordinator.swift:256)。

### 7. 按 GPU 预算与温度调整可选画质效果

Metal drawable 自动跟随画布像素尺寸；超分开启且满足放大条件时，在同一 command buffer 内增加源图转换、MetalFX scaling 和输出绘制。超分 scaler 与中间纹理已经按尺寸缓存。当前 iOS 源码的温度、低电量状态可作为后续新增的输入事实。

建议依据 GPU 时长、真实上屏 P95/P99 和温度，优先动态调整超分、锐化与本地渲染尺寸，并设置恢复滞回。该优化主要面向开启画质效果的长时游玩，收益需要真机测量。

证据：[drawable 配置](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamingMetalRenderer.swift:983)、[超分预算与复用](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/Streaming/StreamingMetalRenderer.swift:1692)。

## 验收依据

本轮仅执行一次现有 `iosapp/scripts/check-streaming-session-boundary.sh` 冒烟，并核对上述源码事实；结果、代码摘要及文件 SHA-256 记录在 `checks.json`。当前测试的行为断言用于审查输入，测试执行状态以 `checks.json` 明确记录。本轮运行时性能测量为空，优化收益为定性判断。

后续实施按单项做最小对比，保持以下六个维度同时可评估：

| 维度 | 对比事实 |
| --- | --- |
| 网络健康 | RTT、窗口丢包、jitter、接收缓冲、NACK/PLI/FIR、候选路径 |
| 恢复链健康 | libwebrtc 解码结果、关键帧请求、恢复后连续解码/呈现与动作冷却；参考链内部事实按 SDK 能力提供 |
| media supply | 接收字节/包、received/decoded 的同窗 delta |
| steady supply | 连续 3s/15s 接收、解码与实际呈现覆盖 |
| submit/present 长尾 | drawable 获取、GPU 完成、实际 drawable 呈现的 P95/P99，入口到提交等待 |
| 帧供给 delta | received、decoded、submitted、GPU completed、drawable presented、local drop 的同窗增量 |

第一轮建议保留触控边沿、降低热路径固定成本，并补齐实际上屏健康判据。涉及监督合同与执行边界的实现应先完成 RFC，再按用户确认进入开发。

本轮最终保留审查文档与静态验收摘要；临时产物清单为空。
