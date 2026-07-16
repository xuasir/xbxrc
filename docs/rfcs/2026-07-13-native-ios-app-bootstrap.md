# RFC: 原生 iOS 应用骨架

**状态：** 已批准并实施
**日期：** 2026-07-13

## 目标

在仓库根目录新增 `iosapp` 原生应用，为 Xbox 成就、游玩时长、游戏列表和后续 WebRTC Direct 串流提供独立 iOS 宿主。

## 决策

- UI 使用 SwiftUI，系统导航、Tab 与材质遵循 iOS 26 原生行为。
- iOS 工程保持独立 Xcode target，不改变现有 Tauri 桌面应用构建链。
- Xbox API、认证与 signaling 后续优先复用 Rust crate，通过 XCFramework 与 UniFFI/C ABI 接入。
- RTC 数据面后续接入固定版本的 libwebrtc；VideoToolbox、Metal、AVAudioSession 和 GCController 归 iOS 平台层。
- 当前阶段只建立可编译工程、模块边界和原生应用壳，不引入 RTC 二进制依赖。

## 目录边界

```text
iosapp/XBXRC/
  App/                 应用入口与根导航
  Features/            游戏库、成就、账户等产品功能
  Platform/Streaming/  iOS RTC 与媒体运行时边界
  Shared/Models/       稳定领域模型
  Resources/           Asset Catalog 等资源
```

## 验收

- Xcode 26 可以直接打开 `iosapp/XBXRC.xcodeproj`。
- iOS 26 Device SDK 无签名 Debug 构建通过。
- 单元测试 target 可以编译和链接。
- Simulator Runtime 安装后执行模拟器构建与测试运行。
- 桌面端现有代码与构建配置保持原状。
