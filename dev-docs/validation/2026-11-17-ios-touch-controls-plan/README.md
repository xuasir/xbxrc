# iOS 触屏控制方案证据

核对日期：2026-11-17。方案基于 `main / b7f09ec8` 及当前工作区源码，沿 deep-brainstorm 流程完成历史、协议和 iOS 接入点评估。

正文：[iOS 触屏控制实现方案](../../../docs/isu/2026-11-17-ios-touch-controls-implementation-options.md)。

## 本轮核对

- 引用聊天已通过 read_thread 读取，覆盖布局协议、TAK/Babylon、官方兼容可行性和原生触点协议四轮内容。
- 现有 iOS 通用触控、输入聚合/归属、8ms 发送循环、消息接收、metadata 和视频 scale 与当前源码逐项核对。
- 官方协议证据引用 2026-11-12/13 保存的源码摘录及隔离回放结果：布局/引擎 41 项，原生触点 29 项。该数字表示历史验证记录。
- 本轮验证文档链接、源码锚点、源文件哈希与新增文件的 diff 格式；结果见 [document-checks.json](document-checks.json)。

## 当前关键锚点

| 文件 | 核对对象 |
| --- | --- |
| TouchGamepadOverlay.swift | SwiftUI/UIKit 控件、单控件 activeTouch 与取消释放 |
| StreamInputSession.swift | setTouchButton、outputState、setSuppressed、releaseTouches |
| XboxStreamDataChannels.swift | inputPollInterval、startInputLoopIfReady、handleMessagePayload |
| IOSStreamInputProtocol.swift | 手柄编码、send gate、neutral drain |
| stream_data_channel.rs | touchInputEnabled 与 clientdevicecapabilities |
| xbox-ios-bridge/streaming.rs | ClientMetadata 固定 64 与已有 UniFFI |
| StreamingMetalRenderer.swift | normalizedRenderScale、fit/fill、旋转 |

## 产物与验证范围

本轮产物为方案和核对记录，客户端行为依据静态源码及历史回放。布局运行、真实触点、服务端响应、功耗与输入时延进入候选任务的实施验收。

本轮直接在仓库证据目录保存结果，临时目录产物创建数量为 0，收尾保留上述文档和核对 JSON。
