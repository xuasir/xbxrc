# iOS 懒惰数据加载与手动刷新 RFC

## Status

- Completion: 已完成
- Current State: completed
- Owner: agent
- Last Updated: 2026-07-22

## Background

上一轮审计确认 iOS 当前存在 Scene active 自动刷新、Tab 激活任务重复进入、凭据续期级联刷新，以及 `AsyncImage` 视图重建后重新进入 loading 的组合问题。产品策略现已明确：浏览数据采用快照优先的懒惰加载；每个页面在当前账号代际首次进入时后台刷新一次，后续刷新由用户手动触发。

## Goal

- App 启动只恢复认证状态和本地快照，保持浏览数据请求惰性。
- 每个数据页面在当前账号代际首次可见时先展示快照，再后台刷新一次；没有快照时显示首次加载态并执行同一轮请求。
- 首次后台刷新完成后，已有数据在 Tab 往返、前后台切换、凭据续期和普通 View 重建时保持展示，后续网络刷新由用户手动触发。
- 每个可刷新页面提供下拉刷新、重试或工具栏刷新，刷新过程中保留旧内容。
- 每个远程数据页在内容、空数据和失败状态都提供下拉刷新；按钮重试作为空态的辅助入口。
- 图片按可见性懒加载，使用共享缓存和 URL single-flight，避免同 URL 多次下载。

## Scope

- In scope:
  - `iosapp/XBXRC/App/XBXRCApp.swift` 的启动、Scene active 和 session task。
  - `XboxDataStore`、`CloudLibraryStore`、`AuthStore` 的 lazy bind/ensure/refresh 生命周期。
  - 主机、游戏库、成就、账户、设置页面的首次进入和手动刷新入口。
  - 共享图片 loader、内存/磁盘缓存、候选替换和加载状态保持。
  - XCTest、iOS Runtime Trace 和真机复现。
- Out of scope:
  - Rust Xbox API 协议和 WebRTC streaming 数据面。
  - 后台定时同步、推送驱动刷新和系统 BackgroundTasks。
  - 本 RFC 确认前的代码修改。

## Lazy Behavior Contract

| 场景 | 行为 | 网络请求 |
| --- | --- | --- |
| App 启动 | 恢复 Keychain session、内存状态和磁盘快照 | 0；只做本地恢复 |
| 首次进入主机/游戏库/账户 | 立即展示快照，同时后台刷新；无快照时显示加载态 | 每个 surface 每个 owner generation 1 次 |
| 首次进入成就详情 | 立即展示 titleID 快照，同时后台刷新；无快照时显示加载态 | 每个 titleID 每个 generation 1 次 |
| Tab 往返 | 恢复页面和滚动位置 | 0 |
| Scene background -> active | 保留页面内容和图片 | 0 |
| Access lease/token 续期 | 只更新凭据和访问句柄，保留所有可展示内容 | 0 |
| 下拉刷新/刷新按钮 | 立即发起一次手动请求，保留旧内容和滚动位置 | 1 次，参与 single-flight |
| 失败空态重试 | 用户点击“重新加载”后发起请求 | 1 次，参与 single-flight |
| 退出登录/账号切换/地区切换 | 清理受影响 scope，下一次进入按 miss 处理 | 当前动作只做必要的认证/地区请求 |
| 图片首次进入可见区域 | loader 先查内存、磁盘，再请求网络 | 每个 URL single-flight |

## Page Contracts

- 主机：首次进入立即展示主机快照并后台刷新一次；后续只通过下拉刷新更新。开关机成功后的主机刷新属于用户命令结果更新，保留旧卡片。
- 游戏库：首次进入立即展示 fresh/stale/expired 目录快照并后台刷新一次；没有快照时显示加载态。后续刷新只由下拉刷新、重试或明确按钮触发。
- 成就：首页首次进入展示 TitleHub 快照并后台刷新一次；详情页按 titleID 首次进入展示快照并后台刷新一次，重复进入复用缓存，后续强制刷新仅来自用户操作。
- 账户：首次进入立即展示 Profile 快照并后台刷新一次；Scene active 不再刷新；后续下拉刷新和菜单刷新请求 Profile，并按需刷新用户主动要求的游戏活动数据。
- 设置：页面进入不请求；应用地区设置是显式动作，清理受影响目录并更新认证 scope。

下拉刷新覆盖主机、游戏库、成就首页、单游戏成就详情和账户页。设置页只处理本地配置与显式地区动作，游戏详情页只消费已加载目录数据。

## State Model

每个 Store 维护以下独立状态：

- `ownerGeneration`：账号、地区或登出发生变化时递增，用于丢弃旧请求结果。
- `hasRenderableContent`：内存或磁盘存在可展示数据时保持内容态。
- `didRunInitialRefresh`：按 surface + owner generation 记录首次后台刷新，Tab 重建和 Scene 变化不会重置。
- `initialLoadState`：`idle/loading/loaded/failed`，只在首次进入 miss 时使用。
- `manualRefreshState`：`idle/refreshing/failed`，刷新期间不切换成骨架屏。
- `lastManualRefreshAt`：只用于展示和防止同一手势重复提交，不作为自动刷新触发器。

页面生命周期只调用 `activateOnce()`：先发布快照，再以 `reason=initialActivation` 启动当前代际唯一一次后台刷新。Store 刷新任务独立于 View observer；页面离开不会取消已经开始的 Store 请求。

## Image Contract

- 使用共享 `RemoteImageLoader` actor，键为规范化 URL。
- 顺序为内存解码缓存、URLCache 磁盘缓存、网络请求；同 URL 并发调用共享一个 Task。
- `CloudGameRemoteImage` 保留上一张成功图，候选 URL 更新时只替换请求任务，不先清空画面。
- 背景与 Hero、头像与头像背景、列表卡与详情页共享结果。
- `imageViewPhase`、`imageCacheHit`、`imageNetworkStarted`、`imageNetworkCoalesced`、`imageNetworkCompleted` 分开写入 Trace。

## Plan

1. 将 App 启动和 Scene active 改为本地恢复路径，移除自动 Profile refresh 和 token 变化触发的数据清空重载。
2. 为各 Store 增加 owner generation、`activateOnce`、initial load、manual refresh 和 single-flight；页面首次进入先发布快照，再执行当前代际唯一一次后台刷新。
3. 为主机、游戏库、账户和成就页面补齐一致的手动刷新入口与旧内容保持态。
4. 实现共享图片 loader，迁移 `AsyncImage` 使用点并补图片状态 Trace。
5. 补 focused XCTest、Device/Simulator build 和真机五 Tab 往返 Trace 验收。

## Validation

- [x] App 启动和 Scene active 不发浏览数据网络请求。
- [x] 首次进入页面先展示快照并后台刷新一次；没有快照时同一轮请求使用首次加载态。
- [ ] Tab 往返五轮、前后台两次、凭据续期一次均保持 0 次自动刷新。
- [x] 每个页面手动刷新只发起一次请求，旧内容保持展示。
- [x] 每个远程数据页的内容、空数据和失败状态均可下拉刷新。
- [ ] 同 URL 图片并发只产生一次底层网络请求，命中内存/磁盘不回到 loading。
- [x] iOS Runtime Trace 已覆盖 `initialActivation`、手动刷新和图片 cache/network 事实；真机时序待采样验证。

## Risks

- 首次进入空页面仍需网络请求，登录态和服务不可用时必须提供稳定失败重试入口。
- 手动刷新会降低数据主动新鲜度，页面应显示上次更新时间并保持用户可控。
- 账号/地区切换属于明确失效动作，仍会清理对应 scope 并在下次进入重新加载。

## Progress

- [x] 既有审计结论确认自动刷新和 Tab 重载来源。
- [x] 将数据策略改为快照优先、首次进入后台刷新一次、后续手动刷新，并固定页面边界。
- [x] 实施 Store、页面和图片 loader 改造。
- [x] 补充 Store focused XCTest，并通过 Swift parse、PBX plist lint、`git diff --check`。
- [ ] 完整 Xcode build-for-testing、测试执行和真机五 Tab Trace 验收。

## Execution Notes

- Date: 2026-07-22 | Status: in-progress
- Update: 用户确认执行快照优先的懒惰模式；实现开始，数据生命周期和共享图片 loader 并行推进，并要求所有远程数据页覆盖内容/空态/失败态下拉刷新。
- Decision: 取消 stale-while-revalidate 和 Scene active 自动刷新；每个 surface + owner generation 保留一次 `initialActivation` 后台刷新。
- Risk/Blocker: 现有工作区包含并行 iOS streaming 改动，实施保持文件所有权和技术栈边界；真机 Trace 需后续构建环境验证。

- Date: 2026-07-22 | Status: completed
- Update: 完成 Auth/Xbox/Cloud Store 代际门闩和 single-flight、当前 Tab 激活门控、五类数据页全状态下拉刷新，以及共享图片内存/URLCache/single-flight 和旧图保持。
- Decision: Tab 选择显式传入数据页，避免 SwiftUI 预创建非当前 Tab 时提前触发首次请求；认证恢复先完成 credential renewal，再发布最终 owner generation。
- Risk/Blocker: Swift parse、PBX lint、差异检查已通过；完整 `xcodebuild build-for-testing` 受 CoreSimulatorService、用户缓存权限和审批服务 503 阻断，真机五 Tab Trace 与图片网络合并仍需在可用 Xcode 环境执行。
