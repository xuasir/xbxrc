# iOS Xbox 游戏库、成就与游玩时长 Report

## Summary

- Related RFC: [`docs/rfcs/2026-07-14-ios-xbox-library-achievements-playtime.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-07-14-ios-xbox-library-achievements-playtime.md)
- 已完成从 Xbox API、Rust 归一化、UniFFI bridge 到 Swift 状态与 SwiftUI 页面的一体化数据链。

## Delivered

- 游戏库展示标题历史、封面、最近游玩时间、成就汇总与 `MinutesPlayed`。
- 成就模块提供按游戏聚合、标题内搜索、延迟加载、缓存、刷新、列表和详情。
- 登录 Token 变化自动刷新数据，退出登录清空会话缓存，旧会话请求结果无法写回新会话。

## Changes

- `xbox-webapi` 新增 TitleHub、Achievements、UserStats 客户端，包含安全 URL 构造、成就分页与只读统计 POST 重试策略；UserStats 遵循单标题请求契约。
- `xbox-ios-bridge` 新增游戏、时长、成就 records 和三个异步导出函数，集中处理身份 claims、分页、去重及 Xbox JSON 变体。
- Swift 新增 `XboxDataClient`、`XboxDataStore` 和领域模型，游戏列表采用分阶段加载，成就按 `titleId` 缓存。
- SwiftUI 完成登录、加载、内容、空态、错误态、搜索、下拉刷新、动态字体和 VoiceOver 信息。

## Validation

- `cargo test -p xbox-webapi`：12 项通过。
- `cargo test -p xbox-ios-bridge`：7 项通过。
- `cargo test -p xbox-auth-flow`、`cargo check -p xbxrc` 通过。
- `cargo build -p xbox-ios-bridge --target aarch64-apple-ios` 通过。
- `cargo build -p xbox-ios-bridge --target aarch64-apple-ios-sim` 通过。
- iOS Device App、arm64 Simulator App、`XBXRCTests` build-for-testing 均完成 Swift 6、UniFFI 与 Rust 静态库编译链接。
- `cargo fmt --check`、Xcode project/plist lint、`git diff --check` 通过。

## Risks

- `MinutesPlayed` 的覆盖率与统计口径由各游戏上报行为决定，缺失标题在 UI 中保持未知状态。
- Xbox 服务权限和历史响应变体仍需要真实账号在线验证，解析层已覆盖当前仓库历史实现中的主要结构。
- UserStats 最多使用 4 路并发查询最近 100 个标题，单标题 400 保持未知时长并继续加载其余标题。
- 当前沙箱中的 CoreSimulatorService 限制了完整 Asset Catalog 构建与 XCTest 实际运行；测试 target 已完成编译链接。

## Follow-up

- 在正常 Xcode 会话中运行完整 Asset Catalog 构建与 `XBXRCTests`。
- 使用真实 Xbox 账号核对 TitleHub 排序、UserStats 覆盖率、成就分页与秘密成就展示。
- 下一阶段将串流入口接入游戏详情，并保持原生 libwebrtc、VideoToolbox 与 Metal 的运行时边界。

## Product Adjustment

- 2026-07-14：游戏库页面调整为固定空状态，作为后续串流入口。
- 成就首页卡片承接游戏时长、成就点数、已获成就数量和全成就进度展示。
