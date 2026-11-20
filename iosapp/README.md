# XBXRC for iOS

原生 SwiftUI iOS 应用，目标系统为 iOS 26。Xbox Auth 与 API 协议复用 Rust，系统登录、Keychain、生命周期和界面由 Swift 管理。

页面层级、胶囊工具区、透明表面、导航、系统字号与无障碍验收统一遵循 [iOS / iPad 设计规范](DESIGN.md)。

## 环境准备

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
```

工程使用 Xcode 26.1 和 Swift 6。Xcode 构建阶段会自动编译当前平台的 `xbox-ios-bridge` 静态库。

串流通过仓库内 `iosapp/Packages/WebRTC` Swift Package 固定消费预编译 `WebRTC` XCFramework。开发、Release 和 CI 使用同一个 `xuasir/xbxrc` GitHub Release 资产，SwiftPM 对完整 ZIP 执行 checksum 校验；镜像 manifest 同时固定上游 package revision、libwebrtc commit、架构、二进制 SHA-256、Privacy Manifest 与许可证来源。

串流固定使用公共 Rust 会话/信令 + libwebrtc 传输与解码 + 自定义 Metal 渲染。WebRTC 与 Metal 为必需构建依赖，播放器直接消费 `RTCVideoTrack -> RTCVideoRenderer -> latest-only mailbox -> Metal presenter`。真机使用 `CVPixelBuffer -> CVMetalTextureCache` 零拷贝导入，模拟器的 I420 三平面上传共用同一渲染链；播放状态以真实首帧 present 为门槛。

首次构建前解析依赖并校验 XCFramework 架构、module、Privacy Manifest 与二进制指纹：

```bash
./iosapp/scripts/resolve-libwebrtc.sh
```

开发环境默认使用 Xcode GUI 的 DerivedData/SourcePackages 缓存，脚本解析完成后可以直接在 Xcode 中构建。CI 可通过 `SOURCE_PACKAGES_DIR` 与 `DERIVED_DATA_PATH` 指定版本隔离缓存；依赖版本、Release URL 与 checksum 由本地 `Package.swift` 和 `artifact-manifest.json` 固定。

CI 版本隔离示例：

```bash
SOURCE_PACKAGES_DIR=/tmp/xbxrc-ios-source-packages/137.7151.04 \
DERIVED_DATA_PATH=/tmp/xbxrc-ios-package-derived-data/137.7151.04 \
./iosapp/scripts/resolve-libwebrtc.sh
```

Rust 导出 record 或函数变化后重新生成 UniFFI 文件：

```bash
./iosapp/scripts/generate-rust-bindings.sh
```

检查 cloud/home 共用 Rust 会话层、Swift 仅保留 libwebrtc 与远端 ICE 应用职责：

```bash
./iosapp/scripts/check-streaming-session-boundary.sh
```

## 打开工程

```bash
open iosapp/XBXRC.xcodeproj
```

## 命令行验证

Device SDK 无签名构建：

```bash
xcodebuild \
  -project iosapp/XBXRC.xcodeproj \
  -scheme XBXRC \
  -destination 'generic/platform=iOS' \
  -derivedDataPath /tmp/xbxrc-ios-derived-data \
  CODE_SIGNING_ALLOWED=NO \
  build
```

Simulator 构建与测试需要在 Xcode 的 `Settings > Components` 中安装 iOS 26.1 Simulator Runtime：

```bash
xcodebuild \
  -project iosapp/XBXRC.xcodeproj \
  -scheme XBXRC \
  -sdk iphonesimulator \
  -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath /tmp/xbxrc-ios-derived-data \
  CODE_SIGNING_ALLOWED=NO \
  build
```

```bash
xcodebuild \
  -project iosapp/XBXRC.xcodeproj \
  -scheme XBXRC \
  -sdk iphonesimulator \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro' \
  -derivedDataPath /tmp/xbxrc-ios-test-data \
  CODE_SIGNING_ALLOWED=NO \
  test
```

## 模块

- `App`：应用入口和根导航。
- `Features/Authentication`：登录状态机与 `ASWebAuthenticationSession`。
- `Features/Library`：云游戏栏目、全量目录、搜索/收藏及详情；方形封面与封面下方文字遵循 PC 视觉密度，串流入口保留稳定游戏身份。
- `Features/Achievements`：前五游戏轮播、PC 透明卡片、游戏时长与成就进度、完整游戏列表和成就详情。
- `Features/Profile`：个人资料、刷新和退出登录界面。
- `Platform/RustBridge`：UniFFI 生成代码和 Swift 业务适配。
- `Platform/Streaming`：公共 Rust 会话接线、libwebrtc、latest-only 自定义 Metal 视频输出、音频生命周期与触屏/手柄输入。
- `Shared/Security`：只在本机可用的 Keychain 会话存储。

## 登录数据边界

- OAuth pending/state 只存在于当前登录流程内存中。
- refresh token、Xbox Web Token 和私有 JWK seed 存入 `kSecAttrAccessibleWhenUnlockedThisDeviceOnly` Keychain。
- iOS 登录只生成 Web Token；xHome/xCloud 串流 Token 在启动串流时按需生成。

## Xbox 数据加载

- TitleHub 游戏历史先返回并显示，UserStats 在第二阶段按单标题契约以最多 4 路并发补齐最近 100 个标题的 `MinutesPlayed`。
- 成就详情按 titleId 延迟加载并保存在当前登录会话的内存缓存中。
- Xbox 未提供 `MinutesPlayed` 时保留未知状态，不推导或伪造时长。
- 成就首页取当前列表前五条进入固定 `260×390pt` 扇形轮播，支持手势滚动、透视过渡、内容背景联动和 Reduce Motion。

## 移动设置

设置提供通用、串流体验、连接与主机、高级诊断和关于子页。选择、滑杆、常用码率和步进调整覆盖无键盘操作；自定义 TURN 可导入 JSON 文件或通过系统粘贴按钮添加，确认后凭据存入本机 Keychain，并经现有 Rust plan 传递给 libwebrtc。

画面比例、PC 同值的颜色预设和锐化沿现有 Metal renderer，真机放大可选 MetalFX 超分；Simulator 继续沿标准缩放。诊断支持限时录制、停止并分享、问题快照及最近记录。设置能力、平台适配及验证见 [迁移 RFC](../docs/rfcs/2026-11-16-ios-settings-parity.md)。

串流菜单与诊断菜单支持最多 4 键的触屏自定义组合、清空和恢复默认。手柄震动提供 PC 同值的真实/增强/全强及四路独立预设；旧偏好恢复线性曲线。防分辨率下降按用户选择保留标准自适应，SDK 逐帧时间能力差异记录在迁移报告。
