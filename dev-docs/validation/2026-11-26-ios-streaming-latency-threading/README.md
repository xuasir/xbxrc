# iOS 三项高优先级优化规划证据

日期：2026-11-26，Asia/Shanghai。

交付：[线程划分、资源复用与 M0–M3 RFC](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-11-26-ios-streaming-latency-threading.md)。工程状态为 planned；规划和用户追加的资源复用内容各完成一轮最小文档/API 冒烟，后续记录保留前轮结果。

## 核心决策

- InputQueue 和 RenderQueue 各自串行调度，分别持有发送状态与 Metal 渲染资源。
- MainActor 承接 UIKit/SwiftUI、现有 TAK 采集/状态计算、触控视觉、图层配置与生命周期。
- 公共 Rust 监督消费可选的实际呈现事实；GPU completion 和 drawable presented 分别记账。
- 数字边沿 FIFO 与最新模拟轴、跨域作用域租约、neutral barrier、有界恢复构成统一执行合同。
- M2 纳入跨 surface shader/pipeline 能力、当帧纹理引用、MetalFX/Simulator 中间纹理、descriptor/uniform 和有界 timing/feedback 容器复用；资源 key、失效、GPU 寿命与分配计数分别验收。

## 当前复用与新增计划

当前应用层可确认：decoder 包装持续转发 base decoder，surface 内 command queue/pipeline 复用，CVMetalTextureCache 映射，MetalFX 尺寸缓存，以及当前工作树的视频色彩 uniform 缓存。VideoToolbox 内部解码 buffer 池由预编译 libwebrtc 管理。

新增计划进入 RFC 的 M2：跨 surface 的能力 cache 与工程 shader、同帧导入资源、复用 pass descriptor 和参数计算、simulator I420 纹理组，以及减少 timing/feedback Dictionary/Array 热路径分配。command buffer/encoder 使用每帧单次生命周期，新帧按自己的 pixel buffer 正确映射。

## API 依据

本机 SDK 为 `/Applications/Xcode.app/Contents/Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS27.2.sdk`，现行工程最低 iOS 版本为 26.0。

- `GCController.input` / `GCDevicePhysicalInput.queue`：iOS 17 可用；支持在指定 Queue 交付输入回调。
- `inputStateAvailableHandler` / `nextInputState` / `inputStateQueueDepth`：缓冲输入按序读取，默认深度 1；SDK 提供深度 20 示例，并说明覆盖造成输入变化缺失。
- `CAMetalLayer.nextDrawable` / `allowsNextDrawableTimeout`：获取可能等待约 1 秒；规划将等待归入 RenderQueue。
- `MTLDrawable.addPresentedHandler` / `presentedTime`：实际呈现与 GPU completion 分别观测。
- `CVMetalTextureCacheCreateTextureFromImage` / `CVMetalTextureCacheFlush`：当帧平面映射与 cache housekeeping；SDK 文档要求为 GPU 使用保持 CVMetalTexture 引用。

源码与 SDK 指纹、文档链接、任务状态和校验结果记录在 `checks.json`。本轮构建、运行时测试和性能测量清单为空。

临时产物创建清单与残留清单均为空；最终保留 RFC、任务行、本证据说明与检查摘要。
