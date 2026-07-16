# iOS Xbox 游戏库、成就与游玩时长 RFC

## Status

- Completion: 已完成
- Current State: completed
- Owner: agent
- Last Updated: 2026-07-14

## Background

iOS 已完成 Xbox 登录、Token 恢复与个人资料。游戏库和成就页仍为占位界面，Rust `xbox-webapi` 也缺少 TitleHub、Achievements 与 UserStats 客户端。

## Goal

- 展示当前 Xbox 账户的游戏历史、封面、最近游玩时间和成就汇总。
- 展示每个游戏的成就列表、解锁状态、Gamerscore、进度和解锁时间。
- 展示 Xbox UserStats 可提供的 `MinutesPlayed`，缺失时保持未知状态。
- 保持 Xbox HTTP 协议、鉴权与响应解析集中在 Rust，Swift 只消费稳定 UniFFI record。

## Scope

- In scope:
  - `xbox-webapi` TitleHub、Achievements、UserStats API。
  - `xbox-ios-bridge` 游戏、游玩时长、成就 record 与导出函数。
  - Swift `XboxDataClient`、共享数据 store、游戏库和成就页面。
  - 登录/退出后的数据加载与清理，搜索、下拉刷新、错误与空状态。
- Out of scope:
  - Store/Game Pass 全量目录与云游戏可用性。
  - 本地游戏安装状态、启动与串流入口。
  - 成就社交比较、排行榜和后台长期同步。
  - 真实账号在线服务可用性保证。

## Data Flow

```text
TitleHub history  -> XboxGameSummary[] -> Swift 首屏
UserStats /batch  -> XboxPlaytime[]    -> 分阶段合并 MinutesPlayed
Achievements      -> XboxAchievement[] -> 按 titleId 缓存与详情展示
```

游戏列表先返回 TitleHub 的稳定信息；游玩时长按照 UserStats 单标题契约以最多 4 路并发独立补齐。单个标题缺少 `MinutesPlayed` 或返回 400 时保持 `nil`，UI 显示未知状态；其他 UserStats 请求错误会保留已加载的游戏列表，并通过简洁的内联错误呈现。

## Bridge Contract

- `fetch_game_library(web_token_json) -> Vec<XboxGame>`
- `fetch_playtimes(web_token_json, title_ids) -> Vec<XboxPlaytime>`
- `fetch_achievements(web_token_json, title_id) -> Vec<XboxAchievement>`

所有函数从 Web Token 中解析 XUID/UHS/XSTS Token。Bridge record 使用字符串、整数、布尔和可选字段，Swift 不解析 Xbox 原始 JSON。

## Plan

1. 新增 Rust Xbox API 模块与纯解析测试。
2. 扩展 UniFFI bridge 并重新生成 Swift/FFI bindings。
3. 新增 Swift 数据 client/store 与领域模型。
4. 实现游戏库、成就汇总和成就详情 UI。
5. 完成 Cargo、Device/Simulator SDK、XCTest target 与差异验证。

## Validation

- [x] Rust API URL/body/header 与解析单元测试。
- [x] Bridge 游戏、时长、成就映射测试。
- [x] `cargo test -p xbox-webapi`、`cargo test -p xbox-ios-bridge`。
- [x] `cargo check -p xbxrc`，确认桌面主线兼容。
- [x] Device/Simulator Rust target 构建。
- [x] iOS Device/Simulator SDK Swift/UniFFI/Rust 编译链接。
- [x] `XBXRCTests` target 编译并覆盖 store 合并/清理状态。
- [x] `cargo fmt --check`、plist lint、`git diff --check`。

## Risks

- `MinutesPlayed` 由游戏自行上报，覆盖率和数值口径存在差异。
- Title history 可能包含 App 与系统项，需要按 title type 过滤并容忍字段缺失。
- UserStats 单标题请求会增加延迟和限流风险，当前限制每次刷新最多 100 个标题并使用最多 4 路并发。
- Xbox 接口返回结构存在历史变体，解析应接受顶层 `data` 包装和数值/字符串混合字段。

## Progress

- [x] Step 1: Rust API 与解析。
- [x] Step 2: UniFFI contract 与 bindings。
- [x] Step 3: Swift 数据状态。
- [x] Step 4: SwiftUI 页面。
- [x] Step 5: 验证与报告。

## Execution Notes

- Date: 2026-07-14 | Status: in-progress
- Update: 已确认现有 Rust 只具备 Profile API，开始补齐下一阶段数据主链。
- Decision: 游戏列表与游玩时长分阶段加载，成就按 titleId 延迟加载并缓存。
- Risk/Blocker: 真实账号响应结构与接口权限需要后续在线验收。
- Date: 2026-07-14 | Status: completed
- Update: 已交付 Rust API、UniFFI 数据契约、Swift 共享状态、游戏库、成就聚合与详情页面，并完成 Device/Simulator/XCTest target 代码构建。
- Validation Boundary: 完整 Asset Catalog 构建、XCTest 实际运行和真实 Xbox 账号在线响应验收留给可用 CoreSimulatorService 与登录态的 Xcode 会话。
