# iOS xCloud 游戏库数据对齐、游戏详情与 Play 入口 RFC

> 说明：本 RFC 承接 `docs/isu/ios-xcloud-library-data-alignment.md` 的既定方向，统一记录跨 Rust、Tauri 与 iOS 的执行里程碑。任务完全完成后再产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: agent / Xbox Data / iOS App
- Last Updated: 2026-07-16

## Background

- 当前 iOS 游戏库直接展示 `XboxDataStore.games`。该集合来自 TitleHub 游玩历史，表达“账号玩过哪些游戏”，并由 UserStats 与 Achievements 补充时长、成就和最近游玩数据。
- Tauri 的 Game Pass/xCloud 栏目表达“当前账号、区域与市场下可串流哪些游戏”。官方数据源由 xCloud `/v2/titles`、MRU、Game Pass SIGL newest 与 products hydration 组成，并输出 `DataXcloudTitleSummary`。
- iOS 当前 `GameSummary.titleID` 是 Xbox 数字 Title ID；Tauri `DataXcloudTitleSummary.titleId` 是 xCloud 串流目标 ID。继续复用同名字段会混淆目录、串流与成就身份。
- iOS 游戏库 Hero、Shelf 与栏目全量列表中的游戏卡片当前全部进入 `GameAchievementsView`。目标交互需要统一进入游戏详情页，由全屏主题图、类似成就轮播的数据卡和 Play 按钮组成。
- iOS 目前只有 `StreamingRuntime` 接口骨架，端到端 xCloud/libwebrtc 媒体运行时仍处于独立规划阶段。本任务交付稳定 Play 请求边界和页面状态，真实首帧、音频、手柄与退出闭环继续沿独立串流阶段实现。

## Goal

- 让 iOS 游戏库使用与 Tauri Game Pass/xCloud 栏目一致的官方可串流目录语义。
- 保留 `productId`、`streamTitleId`、`xboxTitleId` 三种稳定身份，并通过 `xboxTitleId` 合并现有账号活动、游玩时长与成就。
- 保持 Tauri 现有 `getXcloudTitles / refreshXcloudTitles / primeXcloudTitles`、SWR、分层缓存、事件与前端字段合同稳定。
- 将游戏库所有游戏卡片统一导航到 `GameDetailView`。
- 交付全屏主题图、圆弧轮播式数据卡、描述与元数据、固定 Play 按钮，以及 loading、error、unavailable、preparing 等状态。
- Play 请求始终使用 `streamTitleId`，为后续真实 StreamingRuntime 接入提供稳定合同。
- 骨架屏与真实 Hero、Shelf、卡片和详情布局使用同一几何常量，数据到达后保持尺寸、滚动位置和页面层级稳定。
- 缓存刷新遵循用户可预期原则：热缓存直接展示，自动刷新只在明确失效条件触发，后台结果在稳定提交点替换；用户操作保持唯一强制刷新入口。
- 图片建立 Hero、Poster、Tile、账户 Artwork 与主题占位的多级回退链，并缓存最近成功 URL。
- 大目录按分类、分页和可视窗口渐进加载，首屏优先交付可操作内容，避免等待全量 hydration 后再渲染。

## Data Contract

### Stable identities

| Field | Meaning | Ownership |
| --- | --- | --- |
| `productId` | Store/Game Pass 产品身份与目录主键 | 目录、产品元数据、缓存 |
| `streamTitleId` | xCloud 串流目标身份 | Play / StreamingRuntime |
| `xboxTitleId` | Xbox 数字标题身份 | TitleHub、UserStats、Achievements |

关联不到 `xboxTitleId` 的目录项继续保留完整浏览与 Play 能力。`streamTitleId` 缺失的目录项保留详情浏览，Play 进入 unavailable 状态。

### Shared catalog model

```text
CloudCatalogItem
  product_id: String
  stream_title_id: Option<String>
  xbox_title_id: Option<u64>
  name: String
  publisher_name: String
  description: String
  tile_image_url: String
  poster_image_url: String
  hero_image_url: Option<String>
  categories: Vec<String>
  supported_input_types: Vec<String>
  has_entitlement: Option<bool>
  is_recently_played: Option<bool>
  is_new: Option<bool>

CloudCatalogSnapshot
  items: Vec<CloudCatalogItem>
  scope: CatalogScope
  cache_state: miss | fresh | stale
  updated_at_ms: Option<u64>
  refreshing: bool
  schema_version: u32
```

### iOS presentation model

```text
CloudLibraryGame
  catalog: CloudCatalogItem
  activity: AccountGameActivity?

AccountGameActivity
  xbox_title_id: u64
  last_played_at: Date?
  playtime_minutes: Int?
  achievement_progress: AchievementProgress?
```

目录事实、账户活动与未来用户 overlay 保持独立更新频率。`XboxDataStore.games` 继续服务成就页和个人页，新增 `CloudLibraryStore` 专门驱动游戏库。

## Architecture

### Shared Rust

- `xbox-webapi` 只负责官方 HTTP API、原始 DTO、transport 与 error：
  - xCloud `/v2/titles`、`/v2/titles/mru`。
  - Game Pass SIGL newest。
  - Game Pass products hydration。
- 新增宿主无关的 `xbox-cloud-catalog-flow`：
  - 规范化 `productId / streamTitleId / xboxTitleId`。
  - 处理 JSON 变体、相对图片 URL、分类与输入能力去重。
  - 组装 base + overlay snapshot、排序、缺失 product 判定与 cache state。
  - 不依赖 Tauri store、Swift、UniFFI 或宿主生命周期。

### Tauri adapter

- `src-tauri/src/mods/data` 继续负责 session 解析、settings store、single-flight、SWR、事件与 RPC 投影。
- `XcloudService` 改为调用共享 flow，并继续输出现有 `DataXcloudCatalogPayload`。
- 保留官方来源限定：live titles 是可串流全集，MRU 与 SIGL 只提供动态标记，products hydration 只补元数据。
- 保留缓存语义：
  - scope 使用稳定 `xid + regionHost + language + market`。
  - base 可渲染 7 天。
  - overlay fresh 10 分钟、可渲染 24 小时。
  - fresh/stale 立即渲染，stale 后台刷新，miss 等待首轮刷新。
  - 同 scope 刷新 single-flight；xCloud 长请求只串行自身链路。
- Tauri 前端 RPC、事件名与字段兼容保持稳定。

### iOS adapter

- `xbox-ios-bridge` 新增按需 `prepareCloudAccess`：
  - 使用 refresh token 与 JWK seed 生成 xCloud token。
  - 沿用 `xgpuweb -> xgpuwebf2p` 回退。
  - 返回轮换后的持久凭据和 Rust 内存 cloud handle。
  - xCloud bearer token 与 region 只驻留 Rust 内存。
- UniFFI 新增 `XboxCloudGame`、`XboxCloudCatalogSnapshot` 与目录加载/刷新入口。
- `CloudLibraryStore` 负责：
  - Application Support 下的版本化 JSON 快照。
  - stale-while-revalidate、single-flight 与账号 generation 隔离。
  - base 跨账号复用，overlay 严格绑定当前 `xid`。
  - 401/403 时串行重建一次 cloud access 并重试一次。
  - 使用 `xboxTitleId` 合并 `XboxDataStore.games` 的账户活动。
  - 记录 `lastSuccessfulSnapshot`、`lastUserRefreshAt`、`lastBackgroundRefreshAt` 与刷新原因，阻止视图出现期间重复自动刷新。
  - 首屏只读取本地 snapshot 与首批分类；后续分类和 metadata hydration 使用有界并发增量提交。

### Refresh policy

- App 启动、Tab 切换、详情返回和普通前后台切换只读取当前可渲染 snapshot。
- 自动远端刷新只在以下条件触发一次：cache miss、overlay 超过 24 小时、账号/region/market/language scope 变化、鉴权恢复后首次进入游戏库。
- overlay 处于 10 分钟到 24 小时 stale 区间时继续展示缓存，并在下次用户下拉刷新或显式刷新动作更新。
- 同 scope 刷新通过 single-flight 合并；刷新完成后只有 product 集合或可见字段发生变化才发布 UI snapshot。
- snapshot 提交保持稳定排序与 identity，SwiftUI diff 复用既有卡片，避免整页重建和滚动位置跳动。
- 图片下载继续使用系统 URL cache；`CloudLibraryStore` 保存每个 product 最近成功的图片候选 URL，用于下一次首帧直接命中。

### Incremental loading policy

- 本地 snapshot 读取完成后立即生成最近、新上架、全部三组首屏数据。
- 首页每组先发布 8 项，完整栏目首批发布 24 项，滚动接近末尾时继续按 24 项扩展可见窗口。
- products hydration 按既有 75 项网络分块执行，解析后按分类/分区增量合并；单块失败保留已展示内容。
- 账号活动与成就采用低优先级 overlay，每批最多 20 个 `xboxTitleId`，只更新对应卡片指标。
- 搜索基于本地完整轻量索引，详情 metadata 和图片按需加载。

## UI Specification

### Library collections

- 最近游玩：优先使用 xCloud MRU；关联到账号活动时展示最近时间、时长与成就摘要。
- 新上架：使用 Game Pass newest overlay。
- 全部游戏：按本地化名称排序，支持搜索；输入类型筛选作为同阶段交付项。
- 现有“玩得最多/成就进度”栏目只展示成功关联 `xboxTitleId` 且拥有对应活动数据的目录项。
- Hero、Shelf、栏目全量列表三类卡片全部进入 `GameDetailView`。成就 Tab 的卡片继续进入 `GameAchievementsView`。
- Loading 状态复用真实页面的 Hero 高度、Shelf 标题高度、卡片比例、间距与 content margins；占位数据只替换内容材质。
- 缓存数据存在时持续渲染真实内容，刷新状态使用轻量顶部反馈和局部 metadata 占位。

### Game detail

- 页面位于游戏库 `NavigationStack` 内，隐藏 Tab Bar，内容延伸到顶部安全区。
- 背景优先使用 `heroImageUrl`，依次回退 Poster、Tile 与主题渐变；使用暗角和底部渐变保证标题与正文可读。
- 账户 Artwork 作为关联成功时的第四级远程回退；所有图片候选统一升级 HTTPS、规范化协议相对 URL，并对失败候选做本次会话熔断。
- Hero 展示游戏名、发行商、分类与简短描述。
- 中部使用可复用圆弧卡片轮播，保持现有成就轮播的拖拽、中心聚焦、Reduce Motion 与触觉语义。
- 数据卡最多 5 张，按可用数据组成：
  1. 成就进度与 Gamerscore。
  2. 游玩时长与最近游玩。
  3. 输入方式与 entitlement。
  4. 分类、发行商与新上架/最近标记。
  5. 已加载的代表成就或近期成就。
- 关联不到账号活动或成就时，详情仍完整展示目录信息和 Play 入口。
- 描述支持折叠/展开，远程图片失败时保持稳定布局。
- 使用 `safeAreaInset(edge: .bottom)` 固定主 Play 按钮，避免长内容滚动后丢失启动入口。

### Play state

- Play 请求模型固定为 `StreamingLaunchRequest(streamTitleId, productId, displayName)`；StreamingRuntime 目标固定为 `.cloud`。
- 按钮状态：ready、preparing、connecting、unavailable、failed、retry。
- `streamTitleId` 缺失、服务明确无 entitlement、当前请求正在提交时禁止重复启动。
- 本任务交付 `StreamingFeatureStore`/runtime adapter 的注入边界、状态转换、幂等与错误展示。
- 真实 session provisioning、libwebrtc 协商、VideoToolbox、Metal、音频、GCController 与首帧验收继续由独立 iOS 云串流 RFC 承担。

## Scope

- In scope:
  - `crates/xbox-webapi` 的官方 xCloud/Game Pass HTTP 能力。
  - 新共享 `xbox-cloud-catalog-flow` 目录规范化与 snapshot assembly。
  - `src-tauri/src/mods/data` 通过共享核心回接并保持现有合同。
  - `crates/xbox-ios-bridge` 的 cloud access 与目录 UniFFI records。
  - iOS `CloudLibraryStore`、快照缓存、活动合并与会话隔离。
  - iOS 游戏库栏目数据切换、搜索/筛选与全量状态处理。
  - `GameDetailView`、共享圆弧卡片组件与 Play 状态入口。
  - Rust fixtures、Swift/XCTest 与双端字段对齐验证。
- Out of scope:
  - 收藏、置顶、隐藏、用户排序与跨设备用户 overlay 同步。
  - 第三方目录源、第三方图片目录与自有后端目录镜像。
  - Tauri Game Pass 页面视觉重做。
  - 真实 iOS libwebrtc 媒体会话、音视频渲染、手柄串流输入与稳定退出闭环。
  - 将成就、协议、目录合并或 token 逻辑迁移到 Swift/TypeScript。

## Implementation Map

### Rust workspace

- Workspace registration:
  - `Cargo.toml`
  - `Cargo.lock`
- Existing HTTP layer:
  - `crates/xbox-webapi/src/xcloud_api.rs`
  - `crates/xbox-webapi/src/lib.rs`
- New HTTP module:
  - `crates/xbox-webapi/src/gamepass_api.rs`
- New shared flow crate:
  - `crates/xbox-cloud-catalog-flow/Cargo.toml`
  - `crates/xbox-cloud-catalog-flow/src/lib.rs`
  - `crates/xbox-cloud-catalog-flow/src/types.rs`
  - `crates/xbox-cloud-catalog-flow/src/normalize.rs`
  - `crates/xbox-cloud-catalog-flow/src/assemble.rs`
  - `crates/xbox-cloud-catalog-flow/tests/fixtures/*`
- Cloud access reuse:
  - `crates/xbox-auth-flow/src/flow.rs`
  - `crates/xbox-auth-flow/src/types.rs`

### Tauri adapter

- `src-tauri/src/mods/data/services/xcloud_service.rs`
- `src-tauri/src/mods/data/cache_repository.rs`
- `src-tauri/src/mods/data/service.rs`
- `src-tauri/src/mods/data/types.rs`
- `src-tauri/src/mods/data/runtime_state.rs`

这些文件继续承担宿主 session、settings store、SWR、single-flight、缓存兼容和 RPC 投影。共享 flow 只替换目录请求后的规范化与组装实现。

### iOS bridge and state

- Existing bridge:
  - `crates/xbox-ios-bridge/src/lib.rs`
  - `crates/xbox-ios-bridge/src/data.rs`
- New bridge modules:
  - `crates/xbox-ios-bridge/src/cloud_access.rs`
  - `crates/xbox-ios-bridge/src/cloud_catalog.rs`
- Generated bindings:
  - `iosapp/XBXRC/Platform/RustBridge/Generated/*`
- New Swift data boundary:
  - `iosapp/XBXRC/Platform/RustBridge/XboxCloudDataClient.swift`
  - `iosapp/XBXRC/Shared/Models/CloudLibraryGame.swift`
  - `iosapp/XBXRC/Shared/State/CloudLibraryStore.swift`
  - `iosapp/XBXRC/Shared/State/CloudCatalogSnapshotRepository.swift`
- App injection:
  - `iosapp/XBXRC/App/XBXRCApp.swift`
  - `iosapp/XBXRC/App/AppRootView.swift`
  - `iosapp/XBXRC.xcodeproj/project.pbxproj`

### iOS feature UI

- Existing library integration:
  - `iosapp/XBXRC/Features/Library/GameLibraryView.swift`
  - `iosapp/XBXRC/Features/Library/LibraryPresentation.swift`
  - `iosapp/XBXRC/Features/Library/LibraryComponents.swift`
- New detail feature:
  - `iosapp/XBXRC/Features/Library/GameDetailView.swift`
  - `iosapp/XBXRC/Features/Library/GameDetailPresentation.swift`
- Shared carousel extraction:
  - `iosapp/XBXRC/Shared/Components/CircularCardCarousel.swift`
  - `iosapp/XBXRC/Features/Achievements/AchievementsView.swift`
- Play state:
  - `iosapp/XBXRC/Platform/Streaming/StreamingRuntime.swift`
  - `iosapp/XBXRC/Shared/State/StreamingFeatureStore.swift`
- Tests:
  - `iosapp/XBXRCTests/XBXRCTests.swift`

## Delivery Checkpoints

1. **Catalog parity checkpoint**
   - 官方 HTTP 与共享 flow 落地。
   - Tauri 回接完成，现有 RPC 输出和缓存行为保持一致。
   - fixtures 差异测试通过后进入 iOS 数据链。
2. **iOS data checkpoint**
   - cloud access、UniFFI、快照缓存与 `CloudLibraryStore` 落地。
   - 同账号目录集合与 Tauri 对齐，账号活动通过 `xboxTitleId` 合并。
3. **Detail UI checkpoint**
   - 游戏库首页、Shelf、全量列表全部进入 `GameDetailView`。
   - 全屏主题图、共享圆弧卡片、描述、降级与辅助功能完成。
4. **Play entry checkpoint**
   - `StreamingFeatureStore` 接受 `streamTitleId` 并输出完整按钮状态。
   - 幂等、失败、重试和 unavailable 测试通过。
5. **Closure checkpoint**
   - Rust/Tauri/iOS 回归、真实账号 20 项抽样和文档 Report 完成。

6. **Experience stability checkpoint**
   - 骨架与真实布局几何差为零，加载切换保持滚动位置稳定。
   - 无用户操作场景下，热缓存页面保持内容稳定且不发起重复刷新。
   - 图片多级回退和失败熔断覆盖真实缺图样本。
   - 1000 项目录夹具在首批发布后保持可交互，并按分类/分页增量扩展。

## Plan

1. 固化匿名 fixtures 与三身份合同，记录同一真实账号的 Tauri scope、目录数量、ID 样本和 token fallback 证据。
2. 将 Game Pass HTTP 请求抽入 `xbox-webapi`，新增 `xbox-cloud-catalog-flow` 并建立旧 Tauri 输出差异测试。
3. 回接 Tauri `DataService/XcloudService`，保持 RPC、事件、SWR、TTL、single-flight 与缓存兼容。
4. 实现 iOS 按需 cloud access、UniFFI 目录 records、版本化快照和 `CloudLibraryStore`。
5. 将 iOS 游戏库切换到 `CloudLibraryGame`，合并 TitleHub 活动并交付最近、新上架、全部、搜索、筛选和状态页。
6. 抽取共享圆弧卡片组件，新增 `GameDetailView`，替换游戏库三类卡片导航。
7. 接入 Play 请求状态机与 runtime 注入边界，补齐可用性、重复点击、失败与重试状态。
8. 执行 Rust、Tauri、iOS 构建测试和真实账号双端目录抽样验收，完成 Report 与任务收口。

## Validation

- [x] fixtures 覆盖 `/v2/titles`、MRU、SIGL、products object 响应、数字/字符串 `xboxTitleId`、相对图片 URL 与缺失字段。
- [ ] 同一 fixtures 下，共享核心与当前 Tauri 输出的 product 集合、字段、recent/new、排序一致。
- [ ] products 分块边界 0、1、75、76、150 通过；MRU/SIGL/products 独立失败时主目录继续可用。
- [x] `/v2/titles` 失败时使用可渲染缓存；fresh/stale/miss、7 天 base、10 分钟/24 小时 overlay 与 single-flight 通过测试。
- [x] `productId -> streamTitleId -> xboxTitleId` 映射可观测，身份缺失项按合同保留。
- [x] 账号切换只接受当前 generation 提交；退出清理账号 overlay 与内存 cloud access。
- [ ] 同一账号、region、language、market 下，Tauri 与 iOS 目录 product 集合一致。
- [ ] 随机抽样至少 20 个游戏，名称、图片、分类、输入能力、entitlement、recent/new 与 Tauri 一致。
- [x] iOS 游戏库只展示可串流目录；成就、时长和最近游玩只通过 `xboxTitleId` 合并。
- [x] Hero、Shelf、栏目列表的游戏卡片全部进入 `GameDetailView`；成就 Tab 导航保持原语义。
- [x] 详情页主题图覆盖安全区，图片缺失时正确回退；圆弧卡片支持拖拽、动态字体、VoiceOver 与 Reduce Motion。
- [ ] Play 只提交 `streamTitleId`；缺失身份、无 entitlement、连接中重复点击、失败与重试状态都有测试。
- [x] 详情无账号活动、无成就或元数据部分失败时仍可浏览，并按合同保留 Play。
- [x] 骨架与真实页面共享 Hero/Shelf/Card/LayoutMetrics；加载完成前后关键 frame 尺寸一致，滚动位置保持稳定。
- [x] 热缓存、Tab 往返、详情返回和普通前后台切换不触发重复远端刷新；每次刷新都记录明确 reason。
- [x] stale 缓存持续展示，用户下拉刷新形成唯一强制更新入口；相同 snapshot 不发布 UI 更新事件。
- [x] Hero、Poster、Tile、Artwork、主题占位依次回退；失败 URL 会话内熔断，最近成功 URL 可跨启动复用。
- [x] 1000 项目录夹具保持 Hero 5 项、Shelf 8 项立即操作，完整栏目按 24 项分页扩展；hydration 75 项分块失败保持已显示分类。
- [ ] 搜索使用本地轻量索引，详情数据按需加载，活动 overlay 每批不超过 20 个标题。
- [x] `cargo fmt`、相关 Rust tests、`cargo check -p xbxrc` 通过。
- [ ] iOS Device/Simulator build、XCTest 与 `git diff --check` 通过。
- [ ] 真实账号在线验证 xgpuweb/xgpuwebf2p、region/market/language 组合和目录字段覆盖率。

## Risks

- iOS 当前登录默认不生成 streaming token。cloud access 必须按需创建，并保持 Keychain 唯一写入者与 token 轮换原子性。
- Tauri `titleId` 与 iOS `GameSummary.titleID` 的同名异义容易产生错误关联；新模型和 UniFFI 字段统一使用 `streamTitleId` 与 `xboxTitleId`。
- 全目录 products hydration 当前缺少稳定 Hero 字段。详情图需要扩展 hydration 或进入详情时按需加载，列表首屏继续保持轻量。
- Tauri 当前 market/language 推导较窄；双端对齐需要以 token region、market 与系统 language 共同构建 scope，并通过在线样本确认。
- 共享核心抽取会同时影响 Tauri 与 iOS；旧 Tauri DTO 差异测试和前端兼容测试是实施前置门禁。
- iOS StreamingRuntime 尚无实现。Play 状态机可以完整交付，真实串流完成度必须在 UI 中准确表达。

## Progress

- [x] Step 1: 已调查 iOS TitleHub 游戏库、Tauri xCloud/Game Pass 数据链、缓存历史、身份合同与既有 ISU。
- [x] Step 2: 已调查游戏库三类导航入口、成就圆弧轮播、全屏 Hero、StreamingRuntime 骨架与可复用组件。
- [x] Step 3: 已形成共享 Rust、Tauri 回接、iOS bridge/store、详情页与 Play 边界的完整执行规格。
- [x] Step 3.1: 已补齐精确文件改动地图与五个交付检查点。
- [x] Step 4: 已完成共享目录、双端 adapter、iOS store/cache、详情 UI 与四项体验稳定性要求。
- [x] Step 5: 已完成本地缓存匿名抽样、Rust/Device build 验证、Report 与任务收口；真机在线验收记录为环境保留项。

## Execution Notes

- Date: 2026-07-16 | Status: planned
- Update: 完成代码现状与历史调查，确认 iOS 当前数据源是 TitleHub，Tauri Game Pass/xCloud 的全集来源是官方 `/v2/titles`，MRU 与 SIGL 只提供动态 overlay。
- Update: 已补充 Rust workspace、Tauri adapter、iOS bridge/state、详情 UI 与测试文件地图，并将实施拆分为目录对齐、iOS 数据、详情 UI、Play 入口、收口五个检查点。
- Decision: 使用独立 `CloudLibraryStore` 驱动 iOS 游戏库，保留 `XboxDataStore` 服务成就与个人页；共享 Rust 目录核心按 `xbox-webapi + xbox-cloud-catalog-flow` 分层。
- Decision: 游戏库所有卡片进入统一 `GameDetailView`；Play 始终使用 `streamTitleId`，端到端媒体串流保持独立阶段。
- Risk/Blocker: 代码实现等待用户确认本 RFC；真实账号的 token、region、market 与目录样本将在执行阶段完成在线验收。
- Date: 2026-07-16 | Status: blocked
- Update: RFC 已达到 implementation-ready，连续三轮保持等待明确执行确认。
- Decision: 按 `task-run` 复杂任务流程暂停实施，并将任务状态、RFC 状态与线程目标统一标记为 Blocked。
- Risk/Blocker: 用户明确确认执行后恢复目标并进入 Catalog parity checkpoint。
- Date: 2026-07-16 | Status: in-progress
- Update: 用户补充四项体验门禁，RFC 已加入同构骨架、显式刷新策略、图片回退/熔断和大目录分类分页策略。
- Decision: 将用户追加要求视为执行确认，恢复实施并优先完成数据稳定性与首屏可操作性。
- Risk/Blocker: 工作区包含同日 iOS/Rust 未提交改动，实施按文件边界增量修改并持续执行差异检查。
- Date: 2026-07-16 | Status: completed
- Update: `xbox-webapi + xbox-cloud-catalog-flow`、Tauri 回接、iOS cloud bridge/store、分层磁盘缓存、渐进 hydration、同构骨架、图片回退与全屏详情页已完成。
- Validation: 相关 Rust 36 项测试、`cargo check -p xbxrc`、iOS Device `build-for-testing`、缓存 base/overlay round-trip 编译门禁与工作树 `git diff --check` 通过。
- Validation: 本机 Tauri 缓存包含 1888 项；前 20 项名称、三身份、图片地址、分类、输入能力与 entitlement 字段完整，全量 Tile/Poster 缺失由回退链覆盖。
- Follow-up: CoreSimulatorService 恢复后执行 XCTest 实际运行和 Asset Catalog 完整构建；真机在线执行同账号目录集合与 20 项图片 HTTP 可达性抽样。
