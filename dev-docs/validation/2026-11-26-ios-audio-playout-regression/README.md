# iOS 扬声器音频回归修复

日期：2026-11-26。用户反馈：扬声器、麦克风关闭时，串流声音出现撕裂、慢放与延迟。本轮修正前一轮音频调参和播放期重配置，恢复固定 SDK 的默认播放合同。

## 代码依据与修复

固定 WebRTC M137 commit `cec4daea7ed5da94fc38d790bd12694c86865447` 的默认 ADM 消费 16-bit 单声道 PCM，默认首选采样率 48kHz、I/O 缓冲 20ms。`AudioDeviceIOS` 按设备实际采样率和 I/O 时长初始化缓冲，`VoiceProcessingAudioUnit` 的 PCM 声道和每帧字节数保持 mono / 2 bytes。原生立体声能力声明为 false。源码摘要、行号和哈希见 [upstream-contract.json](upstream-contract.json)。

前轮将首选硬件输出声道设为 2、I/O 缩至 5ms，并在 SDP/路由事件后重新设置会话。这增加了回调压力和播放期设备重配置风险；初始化及数值测试覆盖不足。硬件输出声道描述设备路由，本地 PCM 声道遵循默认 ADM。当前仓库的新实机 trace 输入为空，具体撕裂/慢放时序以修复后的实机复测为依据。

本轮恢复 SDK 默认配置，保留 playAndRecord / voiceChat、默认扬声器及 SDK 输入兼容条件；协商与健康路由变化更新诊断。初始激活、显式麦克风方向切换和系统中断/路由故障恢复继续沿已有生命周期。新增 `audioConfigurationOwner`、`nativePlayoutChannels`，继续记录远端声道及实际设备参数。接收统计、色彩和其他任务的串流修改保留。

## 最小验证

一轮 iPhone 18 Pro / iOS 27.2 Simulator 的 6 项 XCTest 全部通过，失败与 XCTest 运行时警告均为 0：SDK 默认 PCM/缓冲、立体声协商下的配置约束、Opus 发送声明、真实会话的协商/健康路由只读观测、默认/反馈两种原生工厂 offer，以及原有独立音频配置。

新增回归直接检查全局 `RTCAudioSessionConfiguration` 对象在声道协商和健康路由后保持同一实例、单声道、20ms 首选缓冲及相同激活次数；原生会话保持音频启用。完整 Simulator 工程编译、4 个 Swift 文件按改动范围格式化、会话边界及修改范围 diff 检查通过。

结果见 [audio-test-summary.json](audio-test-summary.json)、[audio-tests.json](audio-tests.json)、[audio-run.json](audio-run.json)、[checks.json](checks.json) 与 [implementation.diff](implementation.diff)。本轮验证为 SDK 合同和原生会话行为；真实连续 RTP 听感、网络与恢复健康、媒体/稳态供给、提交/呈现长尾和同窗帧 delta 接续实机短测。

## 清理与实机接续

本轮临时构建、依赖副本、结果包、上游源码与脚本按 [执行清单](execution-manifest.json) 清理，模拟器恢复初始关闭状态；必要摘要、源码证据和检查记录保留，释放量及路径核验见 [cleanup.json](cleanup.json)。共享依赖、Rust 缓存和其他任务产物保留。

重新构建并重启 App，再以扬声器、麦克风关闭开始一次串流，确认声音连续性、播放速度及延迟。
