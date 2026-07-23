# iOS“我的”Tab 合并 Report

## Summary

- Related RFC: [`docs/rfcs/2026-07-22-ios-my-tab-merge.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-07-22-ios-my-tab-merge.md)
- iOS 底部导航已收敛为“游戏库 / 主机 / 成就 / 我的”，“我的”以账户内容为主页面，并承载四类设置入口与末尾退出登录。
- 现有认证、Cloud Access、地区路由、Runtime Trace、XboxData 与 Rust bridge 合同保持稳定。

## Delivered

- 根 `TabView` 删除独立设置项，账户项更名为“我的”，默认页继续为游戏库。
- 已登录、资料加载、资料失败和未登录状态共享可达的设置入口；已登录内容保留 Profile Hero、活动、社交和成就概况。
- “云游戏 / 登录偏好 / 诊断 / 关于”分别进入职责单一的二级页面，复用“我的”根 `NavigationStack`。
- 资料刷新统一为系统下拉刷新，同时刷新 Profile 与 Xbox 活动库；退出登录固定为已登录内容流最后一项并保留确认。
- XboxData 根同步身份同时绑定 session 与 `ownerGeneration`，地区或账户代际变化会清理旧门闩并重新开放首次 activation。

## Changes

- `AppRootView.swift`：根导航由五项收敛为四项，`AppSection.profile/settings` 合并为 `.my`。
- `ProfileView.swift`：移除右上角菜单与自定义刷新浮层，增加设置分组、末尾退出行、双数据源 activation/refresh 和地区应用门闩。
- `SettingsView.swift`：拆为四个二级设置页面，保留地区应用、临时登录、Trace 导出/清理/分享和版本信息；新增 `MySettingsPresentation` 摘要模型。
- `XBXRCApp.swift`、`XBXRCTests.swift`：增加 session + owner generation 复合同步身份、设置摘要映射和旧代际请求丢弃回归。

## Validation

- `rg --files iosapp/XBXRC iosapp/XBXRCTests -g '*.swift' | xargs xcrun swiftc -parse`：通过。
- `plutil -lint iosapp/XBXRC.xcodeproj/project.pbxproj`：通过。
- `git diff --check` 与 `git diff --cached --check`：通过。
- 定向源码门禁：四个根 Tab、独立 Settings Tab 移除、Profile 菜单移除、四个二级设置页、Profile/library 双 activation 与双 manual pull、复合代际 task identity 均通过。
- `xcodebuild build-for-testing` 已尝试；执行在 Swift 编译前被 SwiftPM 用户缓存权限和 CoreSimulator 环境阻断，沙箱外重试遇到审批服务 503，因此 XCTest 未实际运行。

## Risks

- 模拟器与真机尚未完成已登录、未登录、二级设置、退出确认的截图和交互验收。
- 最大 Dynamic Type、VoiceOver、深浅色以及真实账号的地区切换与下拉刷新仍缺运行时证据。
- XCTest 新用例已通过 Swift parse，完整类型检查与执行结果依赖可用的 Xcode/SwiftPM/CoreSimulator 环境。

## Follow-up

- Xcode 环境恢复后执行 Simulator/Device `build-for-testing` 与 focused XCTest。
- 使用真实账号完成“我的”首屏、四个二级页、下拉刷新、地区应用、退出登录及无障碍矩阵验收。
