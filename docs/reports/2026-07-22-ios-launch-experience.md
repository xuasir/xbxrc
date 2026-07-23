# iOS 品牌启动页与启动过渡 Report

## Outcome

已完成 iOS 白色启动体验：系统首帧使用白色背景与用户提供的 `icon-source.svg` 中心图标；SwiftUI 接管后持续显示同一构图，认证恢复结束时图标以中心为锚点放大外溢并淡出，Reduce Motion 下改为纯淡出。

## Changes

- `iosapp/XBXRC/Resources/Info.plist` 配置 `LaunchBackground` 与 `LaunchIcon`。
- `iosapp/XBXRC/Resources/Assets.xcassets/LaunchIcon.imageset` 保存用户 SVG 的矢量版本，保留原始玻璃渐变与透明留白。
- `iosapp/XBXRC/Resources/Assets.xcassets/LaunchBackground.colorset` 固定纯白背景。
- `iosapp/XBXRC/App/LaunchExperienceView.swift` 新增启动层，绑定 `AuthPhase.restoring`，实现 420ms 放大外溢、220ms Reduce Motion 淡出和 VoiceOver 语义。
- `iosapp/XBXRC/App/AppRootView.swift` 接入启动层，保证认证恢复成功、失败和无会话路径都能退出。
- `docs/designs/2026-07-22-ios-launch-screen-white.svg` 与 PNG 预览记录最终构图。

## Validation

- `xcrun swiftc -parse`：通过。
- `plutil -lint`：Info.plist 与 Xcode 工程通过。
- `jq empty`、`xmllint --noout`：启动资源 JSON/XML 通过。
- `git diff --check`：通过。
- `rsvg-convert` + `ffmpeg`：白底中心图标预览通过。
- 完整 `xcodebuild`：当前环境无法访问 SwiftPM/Clang 缓存，且 CoreSimulatorService 连接被系统拒绝；构建命令未能进入业务编译阶段。

## Follow-up

在具备正常 Xcode 缓存和 CoreSimulator 服务的机器上执行一次 Debug Device/Simulator build，并用启动恢复成功、无会话、恢复失败三条路径截图确认放大外溢时序。
