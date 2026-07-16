# iOS 设置 Tab、地区路由与诊断入口迁移 RFC

## Status

- Completion: 已完成
- Current State: completed
- Owner: agent / iOS App / Xbox Auth Bridge
- Last Updated: 2026-07-16

## Background

- 最新 iOS trace 显示重新登录后 `xHomeToken` 成功、会话为 `appLevel=1`，`xgpuweb` 与 `xgpuwebf2p` 均返回 HTTP 403。
- iOS 当前始终向认证流传入空 `force_region_ip`，请求使用当前公网地区；桌面端已经提供 Default、Australia、Brazil、Europe、Japan、Korea、United States、South India、Central India 路由预设。
- iOS Runtime Trace 的 profile、导出与清理入口当前位于账户页右上角菜单。诊断属于应用级设置，需要在退出登录状态下保持稳定可达。
- 当前底部 Tab 只有游戏库、成就、账户，缺少独立设置入口。

## Goal

- 底部 Tab 新增“设置”，使用系统齿轮图标并保持现有主题与导航语义。
- 设置页提供“云游戏地区路由”单选项，与桌面端预设集合和值保持一致。
- 地区路由使用 Swift 本地设置持久化，并通过现有 Rust bridge 参数传给登录完成、会话续期与 Cloud Access 补取。
- 设置变化后清理当前 Cloud Access/目录 scope，并触发一次会话续期；成功时更新 Keychain 会话和游戏库。
- Runtime Trace profile、导出当前/全部、清理功能迁移到设置页；账户页只保留资料刷新和退出登录。
- trace 只记录路由预设标识与 `forceRegionApplied`，避免记录 IP 原值。

## Scope

- In scope:
  - `AppRootView` 新增 Settings Tab。
  - 新增 Swift 设置模型、UserDefaults 持久化和 `SettingsView`。
  - 桌面端现有地区路由预设在 iOS 侧建立等价定义。
  - `XboxAuthClient`、`AuthStore`、`XboxCloudDataClient` 与 `xbox-ios-bridge` 增加 `forceRegionIp` 参数透传。
  - 设置变化后的认证续期、Cloud Library scope 重建和安全 trace。
  - 账户页诊断菜单迁移与 XCTest/Rust 回归测试。
- Out of scope:
  - 自定义任意 IP 输入。
  - 自动探测最佳地区、测速或代理/VPN 管理。
  - 修改桌面端设置合同。
  - 在 Swift 中实现 Xbox token 或目录协议。

## Data and lifecycle contract

- 设置键：`ios.cloud.forceRegionPreset`，默认 `default`。
- 预设值与桌面端 `FORCE_REGION_IP_OPTIONS` 对齐；Swift UI 只展示地区名称，Rust bridge 接收对应 IP 字符串。
- 启动恢复、OAuth 登录完成、普通 session renew、Cloud Access prepare 均读取同一设置快照。
- 用户切换地区后：保存设置 -> trace 记录 preset ID -> 释放活动 Cloud Access -> 续期认证 -> 更新 Keychain -> 重新激活游戏库。
- 地区切换失败时保留所选设置和现有普通 Xbox 会话，设置页显示脱敏错误；用户可继续选择其他地区。
- `appLevel=2` 表示 xCloud token 成功；`appLevel=1` 继续展示 offering 失败诊断。

## UI specification

- Tab 顺序：游戏库、成就、账户、设置。
- 设置页采用 `NavigationStack + List/Form`，分为：
  - 云游戏：地区路由 Picker、当前认证状态、应用设置按钮/进度和失败提示。
  - 诊断：Trace profile Picker、导出当前、导出全部、清理 Trace。
  - 关于：应用名称与构建版本。
- 地区说明明确提示：路由预设只影响 Xbox streaming token 与区域选择；修改后会刷新云游戏访问。
- 诊断操作在登录和退出状态均可用。

## Plan

1. 新增设置模型与持久化容器，注册到 SwiftUI environment。
2. 新增 Settings Tab 和设置页，将账户页诊断状态与操作迁移过去。
3. 扩展 Rust UniFFI 登录、续期、Cloud Access 参数并更新 Swift adapters。
4. 在 AuthStore/CloudLibraryStore 建立地区切换后的 session renew 与 scope reset 闭环。
5. 补齐 Swift/Rust 测试、生成 bridge bindings、完成 arm64 Simulator test build。

## Validation

- [x] 地区预设默认值及持久化 XCTest 已编写。
- [x] Settings Tab、诊断迁移和退出登录可达性构建验证。
- [x] Rust bridge 参数透传与空值/非空值测试。
- [x] 地区切换后 session renew、Cloud Access scope 重建和 single-flight 接线。
- [x] Trace 源码门禁确认只记录 preset ID 与 `forceRegionApplied`。
- [x] `cargo fmt --all`、Rust tests/check、arm64 Simulator `build-for-testing`、`git diff --check`。

## Risks

- 预设 IP 属于地区路由提示值，服务端策略变化可能让个别预设失效；错误诊断继续保留 offering/status/retriable 字段。
- 地区变化会让缓存 scope 失效；切换期间保持旧内容或同构骨架，避免长时间白屏和重复自动刷新。
- OAuth 回调与后台 session renew 必须读取同一设置源，避免登录与目录补取使用不同地区。

## Progress

- [x] Step 1: 已定位 Tab、账户诊断入口、桌面端路由预设与 iOS bridge 固定空值。
- [x] Step 2: 已完成设置模型、Settings Tab、诊断迁移、认证/Cloud Access 参数透传与 bindings 生成。
- [x] Step 3: Rust、Swift parse、PBX lint 与 arm64 Simulator `build-for-testing` 全部通过。

## Execution Notes

- Date: 2026-07-16 | Status: completed
- Update: 实现与验证完成；Report：[`reports/2026-07-16-ios-settings-region-routing.md`](../reports/2026-07-16-ios-settings-region-routing.md)。
- Decision: 诊断完整迁移到独立设置 Tab；地区设置只保存 preset ID，trace 只记录 preset ID 与是否应用路由。
- Risk/Blocker: 无实现阻塞；真实地区可用性由用户模拟器 trace 验收。
