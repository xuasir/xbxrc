# 云游戏目录接口、可用性与交互增强分析

日期：2026-11-11。状态：方向探索与源码审计完成，候选任务待进入 `task-run`。

证据范围：xbxrc `main` 当前工作区，HEAD `74d3caee1fba2e82f8c84d870f6f9e87eea58abc`；better-xcloud 本地 `typescript` 分支，HEAD `76db611e610c45ea996f6d35a9df5f980e8f0f85`。公开 Game Pass 请求已抽样；带账户认证的目录、MRU、输入配置与排队接口使用源码证据。

公开样本与请求合同：[public-api-probe.json](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-11-cloud-catalog-ux-analysis/public-api-probe.json)。线上样本代表此次查询的 market、language 和商品集合，字段覆盖与栏目数量随服务变化。

## Problem Framing

推荐把云游戏页建设成三个任务的入口：快速再次游玩、发现适合自己的游戏、理解访问状态并完成恢复。目录数据的准确性、浏览连续性和开玩决策信息共同决定体验。

当前页面已有最近游玩、新入库、全部目录、本地搜索、渐进增加卡片、SWR 缓存、显式刷新和几何导航。主要机会集中在数据事实进入交互、服务异常保留可操作内容、游戏详情与可靠开玩入口。

三类用户路径：

- 回访：找到上次玩的游戏，查看当前访问状态，一次执行再次游玩。
- 发现：按类型、关键词和输入方式缩小范围，阅读简介、截图和语言支持，决定开玩或收藏。
- 恢复：确认目录是否来自缓存、账户访问是否需要更新、当前服务器选择与错误原因，完成有针对性的恢复动作。

## Current Constraints

### 架构与历史

- Vue 3 + TypeScript 负责页面；Rust `xbox-webapi` 负责 HTTP 合同；`xbox-cloud-catalog-flow` 负责目录规范化；Tauri data service 负责认证范围、缓存、并发和 RPC。
- 主线保持 Browser 媒体与 Browser GamepadBackend；导航沿用 geometric pathfinding engine、统一 scope、焦点记忆和输入归属。
- 目录元数据、服务准入、输入能力、排队估计、串流创建各自保留事实来源与时效。
- 设置中的云服务器区域使用当前服务返回的有效区域；高级登录兼容沿既有设置入口。目录 market、文案 language、游戏运行语言、服务器区域分别建模。
- 当前商品 market 固定 `US`，中文文案使用 `zh-TW`，英文使用 `en-US`。语言调整需保留缓存范围迁移和回退。
- 账户 overlay 按账户 / region host / language / market 隔离。收藏可先使用按账户隔离的本地持久化。
- 真实账户、Tauri 窗口、物理手柄和串流启动由用户实机验收。

相关历史：

- [共享目录、三身份与 iOS 详情交付](/Users/guo.xu/Documents/code/games/xbxrc/docs/reports/2026-07-16-ios-xcloud-game-library-detail-play.md)：共享 flow 已落地，iOS 已有首批 75 项商品补全、详情和图片候选链。
- [PC 个人与主机游戏详情](/Users/guo.xu/Documents/code/games/xbxrc/docs/reports/2026-11-05-pc-console-library-my-pages.md)：已有详情、成就、时长、相册、来源返回和焦点恢复模式。
- [非串流 HTTP 审计](/Users/guo.xu/Documents/code/games/xbxrc/docs/reports/2026-11-08-non-streaming-http-audit-fixes.md)：认证恢复、有限重试、来源失败、缓存 scope / generation 与总预算已经形成公共约束。
- [云服务器选择与登录兼容](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-11-10-better-xcloud-selected-streaming-improvements.md)：服务有效区域列表与服务器偏好已有接入。

### 接口能力地图

| 接口 | 身份与职责 | 当前接入 | 优化用途 |
| --- | --- | --- | --- |
| GET `{region}/v2/titles` | Cloud Bearer；账户当前云目录；包含 stream title、product、Xbox title、输入类型、entitlement | 目录主来源 | 卡片访问状态与开玩事实；公开栏目与账户目录的交叉核对 |
| GET `{region}/v2/titles/mru?mr=25` | Cloud Bearer；云游戏最近记录 | 合并为布尔标记 | 保留服务顺序，驱动快速再次游玩 |
| POST `{region}/v2/titles`，`alternateIds` + `alternateIdType: productId` | Cloud Bearer；定向按商品查询 | better-xcloud 已使用；xbxrc `XcloudApi` 可扩展 | 详情或启动前按需更新单游戏身份与访问事实，缩短全目录依赖 |
| GET `catalog.gamepass.com/sigls/v2` | 公开 market / language 栏目 | 当前仅通用 newest | 云专用新入库、热门、即将离开、BYOG、输入方式栏目 |
| POST `catalog.gamepass.com/v3/products`，`RemoteLowJade0` | 公开商品轻量资料 | 全目录缺项补全，75 项 / 批、四路并发、30 秒整轮预算 | 卡片名称、分类与封面，优先首屏与最近记录 |
| 同一 products 接口，`RemoteHighSapphire0` | 公开完整商品资料 | WebAPI 已提供 hydration 参数，cloud flow 当前使用轻量默认值 | 详情按需取得简介、主题图、截图、语言、分级、功能与无障碍信息 |
| GET `{region}/v1/waittime/{streamTitleId}` | Cloud Bearer；服务资源分配与准备估计 | `SessionApi` 和会话 scheduler 已接入；目录 RPC 可扩展 | 详情与稳定聚焦项的预估排队；进入真实排队后消费会话当前事实 |
| POST `{region}/v2/titles/inputconfigs`，`titleIds` + `titleIdType: xboxTitleId` | 指定标题身份和目标访问上下文；输入配置 | 当前目录 RPC 使用 home gateway | 修正 Cloud 上下文与 ID 映射，驱动输入可用性说明 |
| TitleHub / achievements / userstats / mediahub | Web / XSTS；个人活动与进度 | “我的”详情已接入 | 云详情复用最近活动、累计时长、成就、个人相册 |

三身份合同必须贯穿页面与启动：`productId` 负责商品 / 缓存 / 收藏；`streamTitleId` 负责云会话和排队；`xboxTitleId` 负责输入配置、个人活动、时长与成就。版本和子标题存在一对多可能，需要实际响应确认映射。

### 公开接口抽样

此次请求采用当前代码的 `Accept`、`Content-Type`、`ms-cv: 1.0`、`calling-app-name` 和 `calling-app-version` 合同。

| 检查 | 结果 | 产品含义 |
| --- | --- | --- |
| 5 商品 `RemoteLowJade0` | 响应体 10,061 bytes；名称、发行商、分类和 Tile / Poster；简介、Hero、语言与分级字段在这批样本中缺省 | 列表保持轻量，简介搜索与详情需要完整资料来源 |
| 同 5 商品 `RemoteHighSapphire0` | 响应体 52,013 bytes；5/5 有简介、开发商、Hero、截图、分级与发行日期 | 完整资料适合按详情商品加载；样本体积约 5.2 倍 |
| `zh-TW` / `zh-CN` 对照 | 5 商品均返回；其中一个名称从 `《Football Manager 27》主機版` 变为 `《足球经理27 主机版》` | 中文目录可以验证简体文案与繁体 / 英文回退，并保留原名别名 |
| 通用 newest `f13cf6b4…` / 云专用 newest `44a55037…` | 商品 35 / 32，交集 31；通用独有 4，云专用独有 1 | 云专用新入库优先，随后与账户目录交叉核对 |
| cloud popular `e7590b22…` | 30 商品，标题为“雲端上的最熱門項目” | 热门发现直接有官方数据源 |
| leaving `393f05bf…` | 14 商品 | 可提示“即将离开”；具体离开日期需要独立字段证据 |
| native MKB `8fa264dd…` | 66 商品 | 游戏官方输入支持集合可作为发现标签；客户端能力独立核对 |

完整商品资料还提供 `Attributes`、`LanguageSupport`、`AccessibilityFeatures`、`AvailablePlatforms`、`PassMetadataByPassProductId`。本批 Pass metadata 观察到 `EntryDateUTC`；language support 使用数字状态，需要确认枚举含义并对缺省值保留未知。游戏商品的 4K / 60fps / 本地合作属性与当前云会话的分辨率 / 帧率 / 多人输入能力各自呈现。

### 当前实现的可用性缺口

| 优先级 | 源码事实 | 用户影响 | 建议 |
| --- | --- | --- | --- |
| P0 | MRU 和 newest 转为 `HashSet`，最终 titles 按名称排序，页面仅 `filter` | 最近游玩的首项随名称决定，新入库的服务顺序也丢失 | 保留独立有序 product ID 列表或 rank；全部列表单独按本地化名称排序 |
| P0 | `spawn_xcloud_refresh` 完成 lease；更新事件位于成功路径。stale RPC 可先返回 `refreshing: true` | 后台失败期间页面可能持续旋转；强制刷新 guard 在 refreshing 时直接返回 | 发布 scoped refresh 完成状态，成功 / 失败 / 取消均结束 busy，保留缓存与可重试操作 |
| P0 | 商品刷新只补数据库缺项，`_force_refresh` 当前未参与资料刷新；旧 base 每次保存获得新时间 | 封面、名称等资料可持续沿用旧内容，7 天资料窗口被保存时间不断延长 | 按实际获取时间维护资料；过期项重新取得；手动刷新有明确范围 |
| P0 | `save_xcloud_catalog_overlay` 对合法空 entries 直接返回 | 服务返回空目录时，已有旧目录可继续回流 | 合法空结果作为新的账户快照提交；协议错误保留旧内容并标记失败 |
| P0 | `hasEntitlement` 缺省为 `true`；页面仅按空 `titleId` 禁用，点击直接进入串流 | 卡片可启动性与账户资格信息脱节 | 权益采用 allowed / denied / unknown；详情始终可访问，Play 根据当前事实给出动作 |
| P0 | 云 `target_id` 直接交给按 `xboxTitleId` 查询的方法，查询端使用 home gateway | 输入查询可能选错身份与访问上下文，影响输入提示和开玩判断 | 建立三身份映射和 Cloud scoped 查询；输入配置只解释合同指定字段 |
| P1 | MRU / newest 失败回退空集合，source availability 仅记录在内部；仍可能输出 fresh | 用户看到最近和新入库消失，页面缺少来源状态 | 同 scope 保留最近成功栏目；每来源携带状态、更新时间和安全错误 |
| P1 | 冷启动先 titles，再等待 MRU / SIGL，再等全部缺项 hydration | 可选资料延长首屏等待，metadata 整轮预算可占 30 秒 | 尽早返回有 scope 的首屏快照；可选栏目和剩余商品后台渐进合并 |
| P1 | 刷新及列数变化将 rendered count 重置为四行；node ID 含数组 index | 深处浏览项可能被移除，搜索退出、刷新和窗口调整影响焦点与滚动 | stable ID 使用 section + product；保留已浏览窗口与商品锚点 |
| P1 | `appLevel >= 2` 决定整页展示；后端先确保 Cloud 访问，再读缓存 | 凭据过期遇到网络故障时，已有缓存也受准入链阻挡 | 公开浏览数据与账户访问状态分别加载；缓存浏览保留，启动由有效身份确认 |
| P1 | 先选择非空 Tile URL；Tile HTTP 失败后 `RemoteImage` 显示占位 | 已有 Poster 可用时仍可能显示占位 | 图片候选按失败递进：Tile → Poster → 主题占位；尺寸规范化另做样本验证 |
| P1 | 目录时间和错误摘要主要写入 shell status，而当前 shell status 为视觉隐藏 | 常规用户看到卡片和刷新图标，目录时效信息缺少可见位置 | 刷新旁显示简短“缓存 · 更新时间 / 更新失败”与针对性重试 |

关键证据：

- [MRU 解析与权益缺省](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-cloud-catalog-flow/src/lib.rs:371)、[最终名称排序](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/cache_repository.rs:327)。
- [缺项补全和重写 base](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/services/xcloud_service.rs:119)、[空 overlay 保存](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/cache_repository.rs:211)。
- [后台刷新完成路径](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/service.rs:292)、[页面刷新 guard](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/XCloud.vue:308)。
- [云输入查询调用](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/streaming/context.rs:211)、[home 查询上下文](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/services/streaming_query_service.rs:197)、[inputconfigs 身份合同](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/session_api.rs:242)。
- [启动入口](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/XCloud.vue:255)、[渲染窗口重置](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/XCloud.vue:156)、[共享图片组件](/Users/guo.xu/Documents/code/games/xbxrc/src/components/common/RemoteImage.vue:27)。

## Options

### Option A：页面层快速增强

- 添加分类筛选、排序、图片回退、稳定商品节点与详情展示；直接使用已有 RPC 字段。
- 交付范围小，详情基础资料与浏览恢复收益明显。
- 服务排序、权益未知、来源失败与刷新完成合同仍需独立修复。

### Option B：修正数据事实，再形成浏览与开玩闭环（推荐）

- 先补 P0 数据与状态合同，再交付轻量目录、完整详情、快速再次游玩与官方发现栏目。
- 功能直接服务选游戏、理解状态和开玩；复用 Rust flow、个人资料模块与现有导航。
- 涉及 DTO、缓存迁移、事件和来源状态，按 complex 任务进入 RFC 分阶段实施。

### Option C：完整个人发现与个性化系统

- 收藏、隐藏、按游戏设置、语言与功能筛选、历史使用画像形成个性化游戏库。
- 长期价值高，可在基础闭环稳定后演进。
- 跨设备同步、推荐算法、完整输入扩展分别形成后续专题；首期使用本地收藏和官方栏目即可提供明确价值。

## Recommended Direction

### better-xcloud 借鉴对照

better-xcloud 在官方 Xbox 页面上增强接口消费、栏目、卡片和设置。其目录页面基础由官方站点承载；本轮把增强脚本实现、官方目录数据和 xbxrc 新增设计分别标注。

| 借鉴项 | better-xcloud 证据 | xbxrc 建议 | 优先级与成本 |
| --- | --- | --- | --- |
| 聚焦后排队预估 | `GameTile.requestWaitTime` 延迟 500ms；普通卡片聚焦后查询，MRU 轨道可一次查询多项 | 稳定聚焦或详情打开后按需查询；scope 含账户 / region / offering / stream title，30–60 秒 TTL、single-flight、离开取消、最多两路并发 | P1，中；排队 HTTP 已有实现，目录需 typed RPC |
| 按商品定向查询 | `XcloudApi.getTitleInfo` POST `/v2/titles`，body 为 `alternateIds` / `alternateIdType` | 详情/启动前刷新一个商品的 title 和访问事实；复用目录中已有映射减少额外查询 | P1，中；带账户实际合同需要实机确认 |
| 官方发现栏目 | `GamePassCloudGallery` 定义云新入库、热门、即将离开、BYOG、MKB、touch 等 SIGL | 采用云专用新入库，加热门和即将离开；保持服务顺序，并与当前账户目录匹配 | P1，低至中；公开端点此次已抽样 |
| 栏目可见性与控制器界面 | `UI_HIDE_SECTIONS`、default / normal / TV 布局、controller-friendly 偏好 | 默认减少重复轨道，已有密度负责窗口适配；后续允许记忆常用栏目与首页偏好 | P1，低至中；当前几何导航与密度继续承载交互 |
| 输入与合作图标 | 官方输入图标增强；community local co-op product list；native / generic / custom touch 处理 | 卡片只放决策必要标签；官方支持、社区布局和本地模拟分别记录；当前可用输入由 runtime + title + 偏好解释 | P1，中；合作与触控运行能力按独立任务验证 |
| 按游戏覆盖设置 | `GameSettingsStorage` 使用 Xbox title ID，读取单游戏值后回退全局值；相同值可归并 | 详情增加“此游戏设置”；首批覆盖画面适配、锐化、音量与输入映射，Rust 负责持久化与有效配置 | P2，高；涉及配置优先级、版本映射、迁移和跨端合同 |
| 设备适配触控提示 | `modifyTitleInfo` 根据设备、手柄与用户偏好添加/移除有效输入 | 原始服务能力与有效输入分别投影；连接手柄后更新页面提示，用户选择保持稳定 | P2，中至高；依赖真实 runtime 能力 |

实现借鉴采用明确边界：

- HTTP 和认证接入现有 Rust provider；Vue 通过 typed RPC 消费。
- focus scope、方向寻路和弹层恢复接入当前几何导航；better-xcloud 的 React 属性读取、bundle 字符串补丁和独立 50ms gamepad polling 属于其宿主适配实现。
- better-xcloud 的标题/排队缓存只按 ID 保存，当前源码采用进程内持续缓存；失败可存空对象，DOM 也提前置 `hasWaitTime`。xbxrc 应保留 source status、有效期、重试与 scope 保护。
- 卡片排队代码读取 `estimatedAllocationTimeInSeconds`；服务同时提供 total / provisioning。total 字段可用时显示“预计等待”，仅有 allocation 时显示“预计排队”，并保留字段来源；实际启动进度消费会话监控。

better-xcloud 关键源码：

- [标题与排队查询](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/xcloud-api.ts:17)。
- [卡片聚焦与 500ms 延迟](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/ui/game-tile.ts:45)。
- [官方栏目 ID](/Users/guo.xu/Documents/code/games/better-xcloud/src/enums/game-pass-gallery.ts:1)。
- [栏目与布局偏好](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/settings-storages/global-settings-storage.ts:442)。
- [按游戏覆盖全局设置](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/settings-storages/stream-settings-storage.ts:447)。
- [有效输入调整](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/bx-exposed.ts:108)、[社区合作列表](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/local-co-op-manager.ts:13)。

### 页面与操作结构

推荐保留“云游戏”一级入口，页内围绕“最近 / 发现 / 全部”组织内容，收藏通过可见入口访问。短期可沿现有页面布局交付；完整页内视图与状态合同在 RFC 中确认。

- 固定工具栏：搜索、筛选、刷新；简短显示账户访问状态与服务器选择。数量放在结果区，目录时效放在刷新旁。
- 最近：按 MRU 排序，首页展示最近 6–8 项并提供完整列表入口；保留快速再次游玩与可见详情入口。应用本地记录在真实进入播放后更新，作为等待 MRU 更新期间的乐观信息。
- 发现：新入库 / 热门 / 即将离开在同一发现区域切换，保持一个主要栏目区域；具体离开日期按服务字段确认后展示。
- 全部：保留网格，本地名称搜索、分类、访问状态、排序；输入筛选分别表达游戏支持和当前设备可用性。列表首期可使用已有分类，完整语言/多人筛选需建立批量轻量索引。
- 收藏：账户内本地存储 product ID、加入时间与置顶顺序；目录暂缺的收藏仍保留条目和访问状态。服务器同步属于后续工作。
- 详情：即时显示已缓存的名称/封面/访问动作，后台获取完整简介、截图、语言、分类和分级。Play 固定在可见操作区；成就/时长/个人相册复用已有查询。

在全部与发现区，确认进入快速详情，主动作明确为“开始游玩 / 更新访问 / 查看原因”。最近区提供直接再次游玩，另设可见详情操作；每处操作文字和辅助技术名称说明实际行为。新增手柄快捷键需要接入公共 intent 与 scope，并具有可见入口和动态提示。

搜索建议：NFKC、大小写与标点归一化；中文繁简与中英文名称保留别名；多关键词分词 AND；排序按名称精确匹配、名称前缀、名称包含、发行商/分类匹配。结果数量、清除搜索和筛选摘要在页面可见。输入法组合期间延后重算，搜索事件预处理索引后执行；描述全文搜索待完整资料覆盖率达标后开放。

### 浏览连续性与性能

- 详情、串流退出、清空搜索、栏目切换、刷新和窗口缩放均以商品锚点恢复焦点与滚动；恢复时先保证锚点已进入渲染窗口。
- refresh 更新同内容时跳过 DOM 大范围重建；已浏览范围保持，删除当前商品时定位同一栏目邻近项。
- 当前渐进追加属于延后挂载，全部滚动后仍会累积大量 DOM。先修节点稳定、预处理搜索和加载窗口；基于 2,000 项样本实测后决定是否引入行级窗口，并保持几何导航可到达性。
- IntersectionObserver 使用真正的 `.xcloud-page__content` 作为 root；加载触发同时覆盖指针滚动和最后一行手柄导航。首屏图片优先，后续图片继续 lazy。
- 首次 RPC 尽早建立 scope 后再接受增量事件；现有 `CatalogRequestState.acceptEvent` 需要已有 scope，首次快照握手是渐进发布的依赖。
- 每来源独立刷新、时间和失败状态；局部失败仅影响对应栏目。目录 refresh ID / revision 与已有 generation 共同保证迟到响应和结束事件归属。

### 可用性与开玩决策

| 用户可见状态 | 页面行为 | 主要操作 |
| --- | --- | --- |
| 当前目录与访问有效 | 展示内容与启动入口 | 开始游玩 |
| 缓存目录，正在更新 | 内容保持，时效可见 | 浏览；按需核对所选游戏 |
| 更新失败，缓存可用 | 内容保持，显示来源与安全提示，busy 结束 | 重试对应来源 |
| 账户访问待更新 | 公开/已绑定范围的缓存内容可浏览，Play 显示访问状态 | 更新访问 |
| 明确地区/资格拒绝 | 展示服务事实与修复入口 | 查看访问状态；查看设置 |
| 资格未知 | 商品可浏览，资格字段保留未知 | 按商品重新核对 |
| 搜索/筛选有效空结果 | 保留搜索与筛选摘要 | 清除条件 |
| 服务成功返回空目录 | 提交空快照，展示有效空状态 | 刷新 / 查看访问状态 |

401 使用既有认证恢复；网络故障提供重试；明确 service code 提供对应说明；通用 403 展示服务拒绝与账户/服务支持地区的检查入口。正文采用“账户云游戏访问资格”措辞，具体订阅结论由服务事实支撑。

运行输入计划当前将 `native_mkb` 和 `touch_surface` 投影为 false。发现栏目可以表达“游戏支持原生键鼠”，当前客户端的开玩建议应结合真实 runtime；键盘映射到虚拟手柄独立表达。见 [runtime 能力](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/streaming/context.rs:279)。

### 验收指标

- 断网 + stale cache、MRU/SIGL/products 局部失败：现有内容保持、busy 正常结束、对应重试可操作。
- 核对 MRU 前 10 项的服务顺序；合法空目录替换旧快照；过期资料刷新实际发出商品请求。
- 浏览第 500 项 → 详情 → 返回；搜索 → 清空；刷新；四档窗口密度切换：商品焦点与浏览锚点恢复。
- 2,000 商品样本测首批内容可见耗时、搜索输入到结果更新时间、DOM 数量、图片请求数和焦点导航耗时；建议首屏本地缓存展示预算 P95 300ms、搜索本地处理预算 P95 50ms，作为待验证工程目标。
- 原生键鼠/虚拟手柄/物理手柄和标题身份对照；游戏支持与实际输入方式一致。
- 快速扫过 20 卡片与重复进入同一详情：排队请求受去重、延迟与并发限制，切换账户/区域后旧响应隔离。
- 真实云启动结果检查：所选 product / stream title / Xbox title 一致，资格错误可解释，失败返回恢复来源卡片。

## Open Questions

1. 当前服务的 `hasEntitlement`、offering 和 BYOG 返回如何覆盖各订阅/购买情形？需要有效账户的脱敏样本，先按 allowed / denied / unknown 设计。
2. POST 定向 title 查询在当前 offering / region 的实际响应、空结果和 401 / 403 如何表现？源码使用已确认，实机合同待补。
3. `LanguageSupport` 数字枚举和实际云运行版本的差异如何解释？商品资料优先作为详情说明，筛选需明确覆盖率。
4. 同 product 的多个 stream title / ChildXboxTitleIds 是否需要版本选择？当前 flow 使用 product → 单条 live title 映射，需要样本确认。
5. 首期采用快速详情弹层还是独立详情路由？推荐独立详情路由复用现有来源返回模式，缩短页首资料并固定 Play；快速详情可作为后续桌面增强。
6. 简体目录采用 `zh-CN → zh-TW → en-US` 哪种按字段回退策略？此次样本确认 `zh-CN` 可返回，完整覆盖率与缓存迁移待验证。

## Candidate Follow-On Tasks

| 顺序 | 候选任务 | 类型 | 收口边界 |
| --- | --- | --- | --- |
| 1 | 云目录数据与可用性合同修正 | complex | MRU/newest 顺序、刷新结束、来源状态、合法空快照、真实资料时间、三身份和权益未知；typed DTO / 缓存迁移 / 回归 |
| 2 | 云目录浏览稳定与搜索增强 | simple（限页面现有合同） | 稳定商品节点、已浏览窗口、滚动锚点、Tile→Poster 失败回退、可见缓存提示、名称归一化与分类/排序 |
| 3 | 云游戏详情与快速再次游玩 | complex | 完整商品按需查询、来源返回、资格动作、个人数据复用；首屏即时资料与局部错误 |
| 4 | 官方发现栏目与按需排队预估 | complex | 云专用 newest / popular / leaving、ordered collections、Cloud scoped waittime RPC、短 TTL / 去重 / 取消 |
| 5 | 公开/缓存浏览与账户访问状态分层 | complex | 浏览与启动准入的状态合同、公开集合来源、已绑定账号范围和退出清理；独立 RFC |
| 6 | 本地收藏与常用栏目记忆 | complex | 按账户 product overlay、置顶排序、目录暂缺条目、版本迁移；数据范围保持本地 |
| 7 | 按游戏设置 | complex | 继承全局→单游戏覆盖、清除覆盖、三身份映射和适用字段；配置有效值与串流消费闭环 |

推荐先完成任务 1 和 2，再交付任务 3 和 4；任务 5–7 根据真实账号证据与产品范围进入后续 RFC。当前阶段产出分析和公开接口证据，实施通过 `task-run` 完成转入与授权闭环。
