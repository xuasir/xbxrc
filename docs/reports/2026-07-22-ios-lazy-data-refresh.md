# iOS 懒惰数据加载与手动刷新 Report

## Summary

- Related RFC: [`docs/rfcs/2026-07-22-ios-lazy-data-refresh.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-07-22-ios-lazy-data-refresh.md)
- iOS 浏览数据已收敛为快照优先、当前账号代际首次进入后台刷新一次、后续由用户手动刷新的生命周期。

## Delivered

- App 恢复认证凭据与云目录快照，Scene active、普通 session token 更新和 Tab 往返不再触发浏览数据刷新。
- 主机、云游戏库、成就首页、单游戏成就详情、账户页在加载、内容、空数据和失败状态均支持下拉刷新。
- 共享图片 loader 提供 64 MiB 内存缓存、URLCache、规范化 URL single-flight、候选回退和旧图保持。

## Changes

- `AuthStore`、`XboxDataStore`、`CloudLibraryStore` 增加 owner/scope generation、一次性 activation、完整刷新流程 single-flight 和旧请求结果丢弃。
- `AppRootView` 将当前 Tab 显式传给远程数据页，页面只在真正切入时激活；Store 门闩保证后续往返零请求。
- Profile、成就和游戏图片迁移到 `SharedRemoteImage`，缓存命中、网络开始/合并/完成和视图 phase 均写入 iOS Runtime Trace。

## Validation

- `xcrun swiftc -frontend -parse`：本任务实现文件与 `XBXRCTests.swift` 通过。
- `plutil -lint iosapp/XBXRC.xcodeproj/project.pbxproj`：通过。
- `git diff --check`：通过。
- Xcode 报告的四处 `Escaping closure captures non-escaping parameter 'content'` 已通过为 refreshable 空态构建器补充 `@escaping` 修复；定向 Swift parse 与 diff check 通过。
- focused XCTest 已覆盖认证恢复零 Profile 请求、Profile 首次激活、DataStore 懒绑定、token 续期零请求、首次 activation 和手动刷新 single-flight、Cloud 快照优先。
- `xcodebuild build-for-testing` 已尝试；CoreSimulatorService 不可用，SwiftPM/Clang 用户缓存受沙箱限制，提权审批服务返回 503。

## Risks

- 本轮未获得完整 Xcode 类型检查和 XCTest 执行结果，Swift 6 严格并发诊断仍需在可用 Xcode 环境确认。
- 真机五 Tab 往返和同 URL 图片网络合并需要 Runtime Trace 采样验证。

## Follow-up

- Xcode 环境恢复后执行 Simulator/Device `build-for-testing` 与 focused XCTest。
- 真机完成五轮 Tab 往返、两次前后台切换、一次 token 续期和图片同 URL 并发采样，使用 iOS Runtime Trace gate 验证零自动刷新与 single-flight。
