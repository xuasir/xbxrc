# iOS Home / Cloud 共用音频输出修复

日期：2026-11-25；任务：`2026-11-25-ios-stream-audio`；状态：代码与自动冒烟完成，真机声音验收待执行。

固定 WebRTC M137 `137.7151.04` 的默认音频设备使用 VoiceProcessingIO，同时启用输入与输出；`RTCAudioSession.configureWebRTCSession` 在播放初始化时检查 `inputAvailable`。原接收会话的 `playback/default` 与这项要求冲突。[固定版本源码摘录](upstream-compatibility.json)保留 commit、来源、行号与哈希。

接收与上行使用独立的 `playAndRecord/voiceChat` 配置，默认路由为扬声器，外接输出交由系统路由。`recvonly`、上行音轨及显式麦克风授权沿现有开关控制。协调器启用 `useManualAudio`，激活与释放通过 `isAudioEnabled` 控制原生音频设备。

M137 在配置和初始化 AudioUnit 前触发 `audioSessionDidStartPlayOrRecord`，对应 trace 改为 `audioPlayoutRequested`；结束事件为 `audioPlayoutStopped`。音频事件新增输入可用性、输入/输出通道数、输出音量和手动控制状态，便于区分接收、请求与系统输出事实。

单轮 iPhone 18 Pro / iOS 27 Simulator 的 15 项 XCTest 全部通过：12 项生命周期状态机、1 项方向配置、1 项权限边界、1 项真实默认/反馈工厂 offer 冒烟。原生冒烟直接检查系统输入路径可用、输入通道为正、手动控制已启用、退出后音频关闭。[结果摘要](xctest-summary.json)、[测试明细](smoke-test-cases.log)与[验证记录](validation.json)已归档；修改范围的 Swift 格式、会话边界与 diff 检查通过。

真机验收：使用本次源码构建的 App，分别进入 Home 和 Cloud 播放有声音的游戏，确认扬声器输出；各退出并重连一次，确认声音随会话恢复。外接输出和麦克风开关按日常设备补验。模拟器 offer 冒烟覆盖初始化前置条件；听感、真实媒体到达及原生 AudioUnit 实际播放启动由真机会话验证。完整运行 trace 的网络、恢复链、media supply、steady supply、submit/present 长尾和帧供给 delta 随真实 Home/Cloud 会话补齐。

临时构建、结果包、复制的依赖和前一个聊天遗留的同任务源码下载按[执行清单](execution-manifest.json)清理；[清理记录](cleanup.json)保存进程完成依据、释放量和路径核验。共享 WebRTC/Rust 缓存与现有模拟器数据保留。
