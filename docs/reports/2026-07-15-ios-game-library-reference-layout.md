# iOS 游戏库参考布局 Report

## Summary

- Related RFC: [`docs/rfcs/2026-07-15-ios-game-library-reference-layout.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-07-15-ios-game-library-reference-layout.md)
- 已完成 iOS 游戏库沉浸式首页、通顶最近游玩 Hero 轮播、四维度内容栏目与图二式全量列表页。

## Delivered

- 首页移除导航页头，Hero 延伸到状态栏后方，并提供分页圆点、6 秒自动切换、手势分页、触觉反馈与 Reduce Motion 适配。
- 首页按“最近游玩、玩得最多、成就进度、全部游戏”组织数据；最近使用横向 Hero 卡，其余使用 2:3 Poster 卡。
- 栏目标题整行进入全量列表页；目的页提供透明导航、大号居中标题、隐藏 TabBar、Liquid Glass 宽卡和成就进度条。
- 登录、骨架加载、全量失败、空数据、局部错误、下拉刷新、图片占位和会话隔离状态均已接入。
- Dynamic Type Accessibility Size 使用图上文下列表布局，VoiceOver 提供栏目、游戏与维度摘要。

## Changes

- `GameLibraryView.swift` 重建首页状态树、通顶滚动容器、刷新与栏目导航。
- 新增 `LibraryPresentation.swift`，集中处理四类栏目、稳定排序、Hero 5 项与首页 8 项截断。
- 新增 `LibraryComponents.swift`，承载 Hero、Shelf、Poster、目的列表与宽卡组件。
- `XBXRCTests.swift` 新增 6 项展示逻辑测试，覆盖排序、缺失值、并列回退、截断、完整集合与元数据。
- Xcode 工程登记新增 Swift Sources；README 同步更新游戏库模块说明。

## Validation

- `xcrun swiftc -parse ...`：业务 Swift 全部通过。
- Device SDK 业务代码与 Rust Bridge 完整编译链接：`EXCLUDED_SOURCE_FILE_NAMES=Assets.xcassets` 下 `BUILD SUCCEEDED`；完整 Asset Catalog 受沙箱内 CoreSimulatorService 限制。
- 完整 Simulator SDK 构建：Asset Catalog、Swift、Rust Bridge 与链接全部完成，`BUILD SUCCEEDED`。
- iPhone 17 Pro / iOS 26.1 XCTest：17 项全部通过，包含新增 6 项游戏库展示测试，`TEST SUCCEEDED`。
- `plutil -lint iosapp/XBXRC.xcodeproj/project.pbxproj`、源码门禁与 `git diff --check` 通过。
- 真实账号内容截图与参考图的最终视觉微调由用户在模拟器完成。

## Risks

- Xbox Hero/Artwork 的比例与清晰度随服务数据变化，页面使用固定裁切和稳定占位吸收差异。
- 当前栏目基于 TitleHub、UserStats 与 Achievements；后续 xCloud 目录接入后可通过展示层映射继续复用 UI。
- 自动轮播在用户手势后按新的 selection 重新计时；Reduce Motion 环境保持静态分页。

## Follow-up

- 用户使用真实账号在模拟器检查 Hero 裁切、首屏栏目露出量与列表页密度，并回传需要微调的截图。
- xCloud 共享目录与 StreamingRuntime 落地后，将 `streamTitleId` 接入现有游戏详情入口。
