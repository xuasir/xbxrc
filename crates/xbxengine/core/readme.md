✦ xbxengine/core 模块是该项目的核心媒体引擎内核，主要负责 WebRTC 协议栈的深度集成、视频处理流水线的编排、输入路由以及自动化的健康恢复逻辑。

  以下是该模块工作的详细梳理：


  1. 核心架构与状态协调 (api/ & lib.rs)
   * XbxEngineRuntime
     (状态机中心)：作为整个引擎的控制中枢，它协调信令交换（HostBridge）、事件通知（EventSink）和媒体后端（MediaBackend）。它管理会话的完整生命周期（Idle ->
     Starting -> Running -> Reconnecting -> Stopped）。
   * 后端抽象 (XbxEngineMediaBackend)：定义了媒体层的标准接口。当前主线实现是 XbxNegotiationBackend（基于 webrtc-rs），同时也保留了 Placeholder
     实现用于单元测试和离线开发。
   * 输入集成 (input.rs)：通过 OhMyGamepad 桥接本地手柄。它能将本地驱动捕获的手柄状态通过 WebRTC Data Channel 实时路由到远端会话，并支持模拟控制指令（如 Nexus
     键）。


  2. WebRTC 传输层实现 (transport/webrtc/)
   * 连接管理 (transport.rs & stack.rs)：
       * 实现了标准的 WebRTC 协商流程（CreateOffer / SetRemoteDescription）。
       * 针对云游戏场景优化了 SDP，如注入 b=AS 码率限制、开启音频 stereo、配置 H.264 Profile 约束。
       * Supervisor 模式：在 XbxActiveMediaStack 中运行一个异步监督任务，当 WebRTC 轨道（Track）就绪时，自动构建并装载媒体流水线。
   * 数据通道 (data_channel.rs)：
       * 管理 input (输入)、control (控制)、message (信令) 等多个 Data Channel。
       * 实现了握手协议、定时关键帧请求、以及基于通道拥塞情况的输入帧过滤（丢弃旧包）。
   * 音频采集 (microphone.rs)：
       * 集成 cpal 进行跨平台音频采集。
       * 内置 Opus 编码器和重采样逻辑，将本地麦克风数据推送到远端。


  3. 视频处理流水线 (media/video/)
  引擎建立了一套基于 Actor 模型 的高性能视频流水线：
   * Ingress (入站调度)：VideoIngress 负责根据帧序号和 PTS 过滤过期帧，并在配置变更或丢失参考帧时强制等待关键帧。
   * Decode (硬解加速)：
       * 抽象了硬件解码接口，目前针对 macOS 实现了 VideoToolbox 插件，支持 H.264 硬件解码。
       * 采用独立线程的 DecodeActor，避免解码延迟阻塞主逻辑。
   * Pacer (起搏器)：XbxPacerActor 根据 PTS 逻辑平滑视频发布，处理网络抖动带来的帧堆积（Catch-up 模式）。
   * Render (渲染对接)：XbxRenderState 提供了一个“最新帧槽位”，使得宿主 UI（如 Tauri 窗口）能以只读方式、零拷贝地消费解码后的 CVPixelBuffer 或像素数据。


  4. 智能自愈与恢复系统 (session/ & recovery.rs)
  这是该模块的特色功能，确保在不稳定网络环境下的体验：
   * 多维度信号监控：同时监控传输层（包到达）、媒体层（帧渲染）和解码层（解码器是否卡死）。
   * 阶梯式恢复策略：
       1. L1 - 关键帧请求：检测到轻微卡顿或解码错误时触发。
       2. L2 - 解码器重置：解决硬解状态异常。
       3. L3 - 自动重连：当连接中断或长时间无法恢复画面时，自动触发信令层的重新协商（Session Reconnect）。
   * 环境预设：内置了 CloudConservative (保守云端)、CloudAggressive (激进云端) 和 LanLowLatency (局域网) 三种恢复配置。


  5. 诊断与统计 (diagnostics/)
   * 全链路统计：实时聚合 RTT (往返时延)、丢包率 (Loss Ratio)、抖动 (Jitter)、当前帧率 (FPS) 和码率 (Bitrate)。
   * 统一日志：提供带有时间戳和模块标签的结构化日志（xbx_log_info! 等），方便排查复杂的异步网络问题。


  总结
  xbxengine-core
  模块不仅仅是一个播放器，它是一个具备自愈能力的低延迟流媒体控制器。它完成了从网络包解析、硬件解码、视频起搏到输入反馈的闭环，是整个项目的核心技术门槛所在。
