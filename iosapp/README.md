# XBXRC for iOS

原生 SwiftUI iOS 应用，目标系统为 iOS 26。Xbox Auth 与 API 协议复用 Rust，系统登录、Keychain、生命周期和界面由 Swift 管理。

## 环境准备

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
```

工程使用 Xcode 26.1 和 Swift 6。Xcode 构建阶段会自动编译当前平台的 `xbox-ios-bridge` 静态库。

Rust 导出 record 或函数变化后重新生成 UniFFI 文件：

```bash
./iosapp/scripts/generate-rust-bindings.sh
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
- `Features/Library`：沉浸式游戏库首页，提供通顶最近游玩轮播、最近游玩/新入库/全部云游戏栏目与全量列表，并为串流入口保留稳定游戏身份。
- `Features/Achievements`：前五游戏 Liquid Glass 轮播、游戏时长与成就进度卡片、完整游戏列表和成就详情。
- `Features/Profile`：个人资料、刷新和退出登录界面。
- `Platform/RustBridge`：UniFFI 生成代码和 Swift 业务适配。
- `Platform/Streaming`：未来 libwebrtc、VideoToolbox、Metal 和音频会话适配。
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
