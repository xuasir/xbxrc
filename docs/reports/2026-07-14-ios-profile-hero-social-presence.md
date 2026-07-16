# iOS 个人账号 Hero 与社交资料 Report

## Summary

- Related RFC: [`docs/rfcs/2026-07-14-ios-profile-hero-social-presence.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-07-14-ios-profile-hero-social-presence.md)
- iOS 个人账号页已完成内容驱动 Hero、在线活动、社交统计与成就概况升级，XUID 仅保留在数据层。

## Delivered

- 最近游玩 Hero 图背景、头像模糊回退、深色渐变遮罩与前景账号身份区。
- Gamerscore、在线设备、当前游戏、Rich Presence、最近游玩、好友、关注与粉丝展示。
- 基于现有游戏库数据的跨游戏成就数量、点数和完成度聚合。
- Social、People、User Presence Rust API 与 UniFFI 可选字段。
- 动态字号、窄屏、Reduce Motion、VoiceOver、下拉刷新和退出入口。

## Changes

- `xbox-webapi` 增加 Social Summary、People Friends 与 User Presence 只读请求和宽容解析测试。
- `xbox-ios-bridge::fetch_profile` 并发请求四类资料；基础 Profile 维持成功边界，三个附加接口独立降级。
- `ProfileView` 消费 `XboxProfile` 与 `XboxDataStore`，根据已有数据自动收缩信息区。
- UniFFI Swift bindings 与 XCTest fixture 同步新增可选字段。

## Validation

- `cargo fmt`
- `cargo test -p xbox-webapi`：15 passed。
- `cargo test -p xbox-ios-bridge`：7 passed。
- `xcodebuild ... -destination 'generic/platform=iOS' ... EXCLUDED_SOURCE_FILE_NAMES=Assets.xcassets build`：`BUILD SUCCEEDED`。
- `xcodebuild ... -destination 'generic/platform=iOS' ... EXCLUDED_SOURCE_FILE_NAMES=Assets.xcassets build-for-testing`：`TEST BUILD SUCCEEDED`。
- `git diff --check`：通过。

## Risks

- Social、People 与 Presence 属于在线 Xbox 服务，真实字段值和隐私隐藏场景需要真机账号验收。
- 当前环境 CoreSimulatorService 缺少可用 runtime，Simulator 构建与 XCTest 实际运行仍依赖正常 Xcode 环境。

## Follow-up

- 在真机登录后核对好友、关注、粉丝、在线设备和 Rich Presence 的实际响应。
- 在安装 iOS Simulator Runtime 后执行 Simulator App 构建与 XCTest 实际运行。
