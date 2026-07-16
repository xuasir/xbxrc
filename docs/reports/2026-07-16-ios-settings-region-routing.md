# iOS 设置 Tab、地区路由与诊断入口迁移 Report

## Summary

- Related RFC: [`docs/rfcs/2026-07-16-ios-settings-region-routing.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-07-16-ios-settings-region-routing.md)
- 已完成 iOS 独立设置 Tab、xCloud 地区路由配置、认证与 Cloud Access 参数透传、地区切换后的目录 scope 重建，以及 Runtime Trace 配置入口迁移。

## Delivered

- 底部 Tab 新增“设置”，形成游戏库、成就、账户、设置四个入口。
- 地区路由提供 Default、Australia、Brazil、Europe、Japan、Korea、United States、South India、Central India，与桌面端值保持一致。
- 设置使用 UserDefaults 保存 preset ID；登录完成、session renew、Cloud Access prepare 读取同一设置并通过 UniFFI 传给 Rust。
- 应用地区设置会释放当前 Cloud Access、清理旧 overlay/scope、续期认证并重新激活游戏库。
- Runtime Trace profile、导出当前/全部、清理 Trace 已从账户菜单迁移到设置页，退出登录状态仍可访问。
- trace 只记录 preset ID 与 `forceRegionApplied`，不记录地区 IP、token 或账号身份。

## Changes

- 新增 `Features/Settings/AppSettingsStore.swift` 与 `SettingsView.swift`。
- 更新 `AppRootView`、`XBXRCApp`、`AuthStore`、`XboxAuthClient`、`XboxCloudDataClient`、ProfileView 与 Xcode project。
- 更新 `xbox-ios-bridge` 的 `complete_login`、`refresh_login`、`prepare_cloud_access` 参数合同，并重新生成 Swift/FFI bindings。
- 新增地区持久化和认证续期参数回归测试；Rust 新增 force region 空白/非空规范化测试。

## Validation

- `cargo fmt --all`。
- `cargo test -p xbox-ios-bridge`：14 项通过。
- `cargo check -p xbox-ios-bridge`。
- `xcrun swiftc -parse`：设置、认证、Tab、Profile、bridge adapter 与 XCTest 文件通过。
- `plutil -lint iosapp/XBXRC.xcodeproj/project.pbxproj` 通过。
- arm64 Simulator `xcodebuild ... build-for-testing` 通过，App 与 XCTest target 完成编译链接。
- `git diff --check` 通过。

## Risks

- Xbox 服务端可能调整地区路由策略，个别固定预设可能失效；现有 offering/status/retriable trace 可继续定位。
- 当前验证覆盖构建与合同；具体地区能否返回 `appLevel=2` 需要用户在模拟器应用设置后生成真实 trace 验收。

## Follow-up

- 在模拟器设置页依次验证 Japan、Korea、United States 等预设，选择能获得 `appLevel=2` 的地区。
- 用最新 iOS Runtime Trace 确认 `forceRegionApplied=true`、`cloudAccessBoundarySucceeded` 与 `catalogRefreshCommitted`。
