# iOS 数据请求、缓存与重载策略审计 RFC

## Status

- Completion: 已完成
- Current State: completed
- Owner: agent
- Last Updated: 2026-07-22

## Background

iOS 各 Tab 共享 `AuthStore`、`XboxDataStore` 与 `CloudLibraryStore`，界面图片主要使用 SwiftUI `AsyncImage`。当前用户观察到图片显示后再次进入加载态、应用自动重复请求、Tab 往返重复加载等现象。本轮先完成代码级审计，明确请求触发源、缓存边界与组件生命周期，再进入实现。

## Goal

- 建立每个 iOS 页面的数据请求、缓存、刷新和生命周期矩阵。
- 定位图片二次加载、前台自动刷新、Tab 往返触发请求的根因。
- 设计可验证的整改方案：请求 single-flight、会话代际隔离、页面激活策略、图片字节缓存与加载状态保持。
- 使用现有 iOS Runtime Trace 证明每次网络请求的触发原因、缓存命中和视图重建。

## Scope

- In scope:
  - `iosapp/XBXRC/App/XBXRCApp.swift`、`AppRootView.swift` 的应用/Scene/Tab 生命周期。
  - `Features/*` 页面中的 `.task`、`.onAppear`、`.refreshable` 和 `AsyncImage` 使用。
  - `Shared/State/XboxDataStore.swift`、`CloudLibraryStore.swift`、`CloudCatalogSnapshotRepository.swift`。
  - `Shared/Components/CircularCardCarousel.swift` 及库、成就、账户图片加载组件。
  - 对应 XCTest、Runtime Trace 字段和真机复现验证。
- Out of scope:
  - Xbox Rust API 协议本身的改造。
  - WebRTC/streaming 数据面。
  - 与本问题无关的视觉、导航和资源供应链改动。
  - 本轮不修改 iOS 源代码；整改方案作为后续执行范围保留。

## Current Findings

1. `XBXRCApp` 在每次 Scene 进入 active 时调用 `authStore.refreshProfile()`，没有最小刷新间隔或 active 边沿去重；前后台切换会产生 Profile 网络请求。
2. `GameLibraryView` 使用 `.task(id: authStore.isSignedIn)` 激活云目录。Tab 视图任务被取消后，`CloudLibraryStore.refresh` 内部创建的非结构化 `Task` 可能继续完成，但外层因取消而丢弃结果；目录仍处于 miss/旧状态，下一次进入会再次刷新。
3. `CloudLibraryStore` 的目录快照只缓存目录元数据和成功 URL，`CloudGameRemoteImage`/`AsyncImage` 没有应用级图片字节缓存或跨视图请求合并。系统 URLCache 是否命中由 HTTP 缓存头和系统实现决定，页面重建仍会重新进入 `.empty`。
4. `GameDetailView` 同时渲染背景和 Hero 两个 `CloudGameRemoteImage`，`ProfileView` 同时渲染头像和头像背景两个 `AsyncImage`；同一 URL 存在多个加载消费者。
5. `CloudGameRemoteImage` 使用 `.id(currentURL)`，并在 `candidates` 变化时重置候选状态。目录首批快照、metadata hydration、成功回退 URL 写入和活动合并会改变候选数组或宿主视图，已显示图片可能重新进入加载态。
6. `AuthStore.prepareCloudAccess()` 和 `prepareHomeAccess()` 会更新 `session`。这会触发 `XBXRCApp` 的 `.task(id: authStore.session?.webTokenJSON)`，`XboxDataStore.sync` 清空并重新加载主机/游戏库；云目录首次激活因此可能串联一次额外的主机和 TitleHub 请求。
7. `XboxDataStore.refreshLibrary()` 没有与 `refreshHosts()` 同等级别的 in-flight single-flight 保护；前台刷新、手动刷新、会话同步可重叠。成就按 titleID 有 loaded 阶段缓存，主机有请求中跳过，云目录有刷新合并，三者策略不一致。
8. Tab 切换本身没有直接调用主机、成就或账户刷新；重复加载主要来自页面任务激活、Scene active 刷新、会话 token 变化，以及图片视图重建。

## Current Request Matrix

| Surface | Automatic trigger | Manual trigger | Cache and retention | Concurrency behavior |
| --- | --- | --- | --- | --- |
| 主机 | `webTokenJSON` 变化后由 `XboxDataStore.sync` 请求 | 下拉刷新；开关机成功后刷新 | 仅进程内数组；刷新时保留已有主机 | 主机刷新 single-flight；电源命令 single-flight |
| 游戏库目录 | 游戏库 Tab 的 `.task(id: isSignedIn)` 调用 `activate`；cache miss/expired 发起请求 | 下拉刷新、失败重试、地区切换 | base 最长可渲染 7 天；overlay 10 分钟 fresh、24 小时可渲染；刷新时保留已有目录 | `CloudLibraryStore.refresh` 合并并发请求；页面任务取消存在结果丢弃窗口 |
| TitleHub/时长 | `webTokenJSON` 变化后由 `XboxDataStore.sync` 请求；目录活动覆盖跟随两阶段发布 | 成就首页、账户页下拉刷新 | 仅进程内数组；正常手动刷新保留旧内容；token 变化先清空 | 缺少 library single-flight；TitleHub 和 playtime 分两阶段发布 |
| 单游戏成就 | 成就详情 `.task(id: titleID)` 首次请求 | 下拉刷新、失败重试使用 `force=true` | 按 titleID 进程内缓存到退出或 token 变化 | loaded 状态跳过；loading 状态仍允许第二个请求 |
| 账户资料 | App 每次 Scene 进入 active 时请求 | 账户页下拉刷新和菜单刷新 | 仅进程内 `profile`；刷新期间保留旧资料 | 缺少 single-flight 和最小刷新间隔 |
| 设置 | 无页面进入请求 | 应用地区设置时清云目录、续期会话并重新激活目录 | 地区切换主动失效 account overlay | 操作按钮通过 `isApplyingRegion` 串行化 |
| 远程图片 | `AsyncImage` 创建、URL/identity 变化、Lazy View 重建 | 无 | 云目录仅保存成功 URL；图片字节依赖系统 URLCache 和服务端缓存头 | 缺少跨视图 in-flight 合并；同 URL 可有多个消费者 |

## Target Behavior Contract

> 2026-07-22 决策更新：后续实现以 [`2026-07-22-ios-lazy-data-refresh.md`](./2026-07-22-ios-lazy-data-refresh.md) 为准；页面首次进入采用“快照优先 + 后台刷新一次”，后续采用手动刷新，替代本节原有的 stale-while-revalidate 建议。

1. Tab 往返只恢复视图和滚动状态。已有可渲染数据时，五个 Tab 往返一轮产生 0 次 Profile、Hosts、TitleHub、Achievements 和 Catalog 网络请求。
2. 首次登录按 owner generation 各触发一次 Hosts、TitleHub 和 Catalog 请求。Cloud/Home access lease 更新只替换凭据和访问句柄，屏幕内容保持可见。
3. Profile 使用 15 分钟 freshness window。Scene 从 background 回到 active 且缓存过期时后台刷新；inactive -> active 的短暂系统过渡沿用现有资料。
4. Hosts、TitleHub、单游戏成就、Catalog 各自使用 single-flight。第二个调用等待同一结果；`force` 表示绕过 freshness，仍参与请求合并。
5. Catalog 使用 stale-while-revalidate：fresh 直接展示；stale 直接展示并在 Store 生命周期内最多启动一次后台刷新；expired/miss 在没有内容时显示首次加载态。
6. 页面离开只解除 UI observer。Store 拥有的数据刷新继续完成并提交到相同 owner generation；退出登录、账号切换、地区 scope 切换负责取消和丢弃旧代际结果。
7. 远程图片统一使用共享 loader。内存命中同步展示，磁盘命中直接解码，网络层按 URL single-flight；背景、Hero、列表卡和头像共享同一字节结果。
8. 已显示图片在数据字段无关更新、Tab 往返和 Lazy View 重建后保持展示。候选 URL 真正变化时继续显示上一张成功图片，直到新图片成功替换。
9. 图片缓存使用 64 MiB 解码内存预算、256 MiB URLCache 磁盘预算；成功响应遵循服务端 freshness 并设置最长 24 小时应用复用窗口，失败结果保留 5 分钟退避。
10. 手动下拉刷新始终可发起一次新请求，旧内容持续显示；明确的加载指示器表达刷新状态，内容区保持稳定。

## Missing Test Evidence

- 缺少 `XboxDataStore.refreshLibrary` 并发合并测试。
- 缺少 `loadAchievements` loading 状态并发合并测试。
- 缺少 `AuthStore.refreshProfile` single-flight、15 分钟 freshness 和 Scene active 边沿测试。
- 缺少 Cloud Library 外层 task 取消后共享刷新继续提交的测试。
- 缺少 token 更新保留 Hosts、TitleHub 和 Achievements 内容的 generation 测试。
- 缺少图片内存命中、磁盘命中、同 URL 并发、失败退避和候选替换测试。
- 现有 Trace 的 `imageCandidateStarted` 代表 View phase，缺少 `memoryHit/diskHit/networkStarted/networkCoalesced/networkCompleted` 传输事实。

## Plan

1. 审计 `XBXRCApp`、五个 Tab、详情页、Store、快照仓库和共享图片组件。
2. 按触发事件、缓存层、旧内容保留和并发保护建立请求矩阵。
3. 根据代码证据整理目标行为、验证门禁和后续整改顺序。

## Validation

- [x] Swift 源码、Store 和已有 XCTest 代码级审计。
- [x] 请求矩阵、目标行为合同和缺失测试门禁整理。
- [x] `git diff --check -- docs/project-task.md docs/rfcs/2026-07-22-ios-data-cache-reload-audit.md`。
- [ ] 真机 Runtime Trace 和整改后的 XCTest，归入后续实现任务。

## Risks

- iOS `AsyncImage` 的内部缓存行为不属于稳定业务合同，不能仅通过 UI phase 推断网络是否真正发出；需要 URLSession/自有 loader 事实和 trace 双证据。
- 图片磁盘缓存需要处理容量、过期、响应 MIME、账号/区域 scope 和敏感 URL 脱敏，避免缓存污染或无限增长。
- 前台刷新节流可能延迟资料更新，需要保留手动刷新和明确的过期边界。
- 云目录缓存当前以 24 小时 overlay 可渲染窗口为产品策略，缩短或延长前需基于真实 trace 决策。

## Progress

- [x] Step 1: 代码级请求、缓存和生命周期审计，形成七类请求矩阵。
- [x] Step 2: 找到图片二次加载、Scene active、Tab task 取消和 session 级联触发链，固定十项目标行为合同。
- [x] Step 3: 形成后续请求/图片加载整改建议和验证门禁。
- [x] Step 4: 产出审计 Report；真机 trace 与代码整改保留为后续任务。

## Execution Notes

- Date: 2026-07-22 | Status: completed
- Update: 完成 `XBXRCApp`、五个 Tab、Xbox/Cloud store、快照仓库、图片组件和已有 XCTest 的代码审计；补齐七类请求矩阵、十项目标行为合同和缺失测试门禁。
- Decision: 先保留当前 Rust/SwiftUI 技术边界，整改集中在 Swift 生命周期、Store single-flight 和共享图片 loader。
- Risk/Blocker: 当前工作区没有可用于本轮的 iOS Runtime Trace 导出文件；真机复现和代码整改归入后续任务。
