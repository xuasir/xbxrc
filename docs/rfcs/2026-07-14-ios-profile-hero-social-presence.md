# iOS 个人账号 Hero 与社交资料 RFC

## Status

- Completion: 已完成
- Current State: completed
- Owner: agent
- Last Updated: 2026-07-14

## Background

iOS 个人账号页当前以居中头像和资料列表为主，信息密度与 Xbox 账号场景较弱。目标视觉参考 Xbox 移动端个人资料页，需要内容驱动背景、清晰账号层级、在线活动和社交统计。

## Goal

- 使用最近游玩游戏 Hero 图构建沉浸式顶部背景，头像图作为回退素材。
- 展示显示名称、Gamertag、Gamerscore、在线状态、当前活动、最近游玩、好友、关注、粉丝和成就概况。
- XUID 仅用于接口身份与请求链路。
- 附加接口失败时继续展示基础 Profile 与已有本地数据。

## Scope

- In scope:
  - `xbox-webapi` 增加 Social、People、User Presence 只读接口。
  - `xbox-ios-bridge` 扩展 `XboxProfile` 可选社交与在线字段。
  - `ProfileView` 接入 `XboxDataStore` 的最近游玩、Hero 图和成就聚合。
  - SwiftUI 顶部视觉、窄屏、VoiceOver 与 Reduce Motion 适配。
- Out of scope:
  - 编辑 Xbox 账号资料、隐身状态或社交关系。
  - 展示 XUID。
  - 新增独立封面上传或本地封面配置。

## Plan

1. 建立 Profile 附加数据 DTO，接入 Social、People、Presence 并采用 best-effort 聚合。
2. 重新生成 UniFFI Swift bindings，保持 Rust 解析权威。
3. 重构 ProfileView 顶部背景、账号信息、状态与统计布局。
4. 执行 Rust 测试、iOS Device/Simulator 源码构建与差异检查。

## Validation

- [x] `cargo fmt`
- [x] `cargo test -p xbox-webapi`
- [x] `cargo test -p xbox-ios-bridge`
- [x] iOS Device SDK App 与 XCTest target 编译链接
- [ ] iOS Simulator SDK App 编译链接
- [x] `git diff --check`

## Risks

- Xbox 私有 Social、People、Presence 响应结构可能随服务端调整，解析采用可选字段和宽容结构。
- 用户隐私设置可能隐藏在线活动或社交计数，UI 对缺失字段自动收缩。
- Hero 图依赖游戏库加载时序，首屏使用头像背景并在数据到达后平滑更新。

## Progress

- [x] Step 1: 字段范围、背景来源与 XUID 展示边界已确认。
- [x] Step 2: Rust API、bridge DTO 与解析测试完成。
- [x] Step 3: SwiftUI 个人账号页重构完成。
- [x] Step 4: bindings、跨层整合与 Device/XCTest 构建验证完成。

## Execution Notes

- Date: 2026-07-14 | Status: in-progress
- Update: 用户确认采用建议字段集合，并明确隐藏 XUID；Rust 数据链与 SwiftUI 页面已拆分并行执行。
- Decision: 基础 Profile 为主请求；Social、People、Presence 作为可选附加请求；最近游玩与成就概况复用现有 `XboxDataStore`。
- Risk/Blocker: 真实账号在线字段值需要后续真机登录验收。
- Date: 2026-07-14 | Status: completed
- Update: Social、People、Presence API、UniFFI DTO、生成 bindings 与 SwiftUI Hero 页面完成；Device App 和 XCTest target 编译链接成功。
- Decision: Hero 优先使用最近游玩游戏图片，加载失败时使用头像放大模糊背景；缺失的在线或社交字段以自适应占位展示。
- Risk/Blocker: 当前环境缺少可用 Simulator Runtime；完整 Simulator 构建、XCTest 运行和真实账号在线接口验收保留到正常 Xcode 会话。
