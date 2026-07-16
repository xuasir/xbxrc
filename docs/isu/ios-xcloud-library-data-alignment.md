# iOS 接入桌面端 xCloud 游戏库数据能力

## Problem Framing

- Tauri 的“游戏库”代表当前账号与区域下可串流的 xCloud 目录。数据来自 xCloud `/v2/titles`、最近游玩接口与 Game Pass Catalog，桌面端将结果合并成 `DataXcloudTitleSummary` 并写入分片缓存。
- iOS 当前的 `GameSummary` 代表 Xbox TitleHub 游玩历史。它叠加 UserStats 游玩时长与 Achievements，主要服务成就页和个人页；游戏库 Tab 当前保留为串流入口空状态。
- 本次目标定义为：让 iOS 游戏库获得与 Tauri 一致的 xCloud 可串流目录能力，同时复用 iOS 已有账户活动数据，形成可浏览、可搜索、可离线回退、可继续接入串流的统一游戏库。
- 第一阶段同步服务器派生数据，各设备按同一账号和 scope 独立拉取。后续收藏、置顶、隐藏和用户排序进入跨设备用户数据同步阶段。

## Current Constraints

### Architecture

- Xbox HTTP、鉴权、响应变体解析和目录合并规则继续位于 Rust。
- SwiftUI 管理 iOS 生命周期、页面状态、本地持久化和系统交互，并通过稳定 UniFFI record 消费 Rust 数据。
- Tauri 数据服务当前依赖 `src-tauri` 的 `DataCacheRepository` 与 settings store；可复用逻辑需要从宿主适配层抽离。
- iOS 已有 `xbox-ios-bridge`、`XboxDataClient`、`XboxDataStore` 和会话隔离机制，可作为新链路的接入基础。

### Existing data semantics

Tauri 目录记录包含：

- 身份：`productId`、串流 `titleId`、可选 `xboxTitleId`。
- 展示：名称、发行商、描述、Tile/Poster 图片、分类。
- 能力：支持输入类型、entitlement。
- 动态覆盖：最近游玩、新上架。

iOS 账户活动包含：

- `titleID`、名称、Artwork/Hero、最近游玩时间。
- 游玩时长、成就计数、Gamerscore 和进度。

统一游戏库需要保留三类稳定身份：

| 字段 | 语义 | 主要用途 |
| --- | --- | --- |
| `productId` | Store/Game Pass 产品身份 | 目录主键、元数据缓存键 |
| `streamTitleId` | xCloud 串流目标身份 | 后续启动云串流 |
| `xboxTitleId` | Xbox 标题数字身份 | 关联 TitleHub、UserStats、Achievements |

合并规则以 `xboxTitleId` 关联账户活动，`productId` 继续作为目录主键，`streamTitleId` 继续作为启动参数。关联失败的目录项仍可完整浏览和启动。

### Authentication and security

- iOS 当前登录流程只生成 Web Token；xCloud 目录需要按需取得 `xCloudToken`。
- refresh token、Web Token、JWK seed 保持 Keychain `WhenUnlockedThisDeviceOnly` 边界。
- xCloud token 由 Rust 按需生成并驻留当前内存会话，退出、账号切换和鉴权失败时立即释放。
- 目录缓存只保存展示 DTO、scope、时间戳和 schema version；日志只记录状态、数量、scope 摘要和错误类别。

### Cache behavior to preserve

- scope 使用稳定 `xid`，并携带 region host、language、market。
- 基础产品元数据可渲染期为 7 天。
- 动态 overlay 新鲜期为 10 分钟，可渲染期为 24 小时。
- stale-while-revalidate：先返回可渲染缓存，再在后台刷新。
- 桌面 v2 兼容层当前可读取同 region/language/market 的最近 overlay。iOS v3 只复用跨账号 base 元数据，账户 overlay 严格绑定当前 `xid`。
- 最新成功的 overlay 整体替换旧 overlay；单个补充请求失败时保留已加载主列表。

### Historical constraints

- 2026-06-23 的桌面缓存修复确认 `xid` 是账号缓存稳定键，并保留跨账号兼容分片回退。
- 2026-07-13 的 iOS 架构确定原生 SwiftUI 宿主与 Rust 协议边界。
- 2026-07-14 的 iOS Xbox 数据链确定分阶段加载、旧会话请求隔离和退出清理语义。
- 真实账号 `/v2/titles`、region 差异、xgpuweb/xgpuwebf2p 权限仍需要真机在线验收。

## Options

### Option A：迁移桌面缓存文件

- 核心：导出 Tauri 的 `data.xcloudCatalog.v2.*` 快照，通过文件、局域网或 iCloud 导入 iOS。
- 收益：可快速展示一次桌面已有目录，离线演示成本低。
- 代价：缓存与设备、账号、区域、语言和时效绑定；桌面在线状态成为 iOS 更新依赖；Token 与数据迁移增加安全面。
- 适用范围：调试夹具、离线迁移工具和故障取证。

### Option B：共享 Rust 目录能力，各端独立拉取

- 核心：抽取 Tauri 的 xCloud 目录请求、规范化、合并和缓存判定为宿主无关 Rust 能力；Tauri 与 iOS 分别提供会话和持久化适配。
- 收益：两端共享协议与字段语义；iOS 可独立刷新；桌面主线继续复用现有能力；后续串流直接使用 `streamTitleId`。
- 代价：需要一次共享边界重构、UniFFI 扩展和双端回归。
- 适用范围：第一阶段生产实现。

### Option C：服务端账户库同步

- 核心：建设自有后端或 CloudKit，保存目录快照和用户操作，通过账号或容器完成多端同步。
- 收益：收藏、置顶、隐藏、排序和设备间偏好可实时同步，也可集中控制版本与增量。
- 代价：需要用户身份映射、隐私策略、服务运维、数据保留和冲突协议。
- 适用范围：用户可编辑 overlay 进入产品范围后的第二阶段。

## Recommended Direction

采用 Option B 交付首期能力，并为 Option C 预留 `UserLibraryOverlay` 合同。桌面缓存文件继续承担桌面离线回退，iOS 从同一 Xbox 账户独立构建自己的目录快照。

### Target data model

建议共享 Rust 领域模型：

```text
CloudCatalogItem
  product_id: String
  stream_title_id: String
  xbox_title_id: Option<u64>
  name: String
  publisher_name: String
  description: String
  tile_image_url: String
  poster_image_url: String
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

iOS 展示模型再合并 `AccountGameActivity`：

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

`CloudCatalogItem` 表达可串流目录事实，`AccountGameActivity` 表达账号活动事实，`UserLibraryOverlay` 表达未来用户编辑事实。三层拥有独立更新频率和冲突策略。三个动态布尔字段使用可空值保留“服务确认”与“补充请求缺失”的差异；Tauri 适配层继续投影现有布尔 RPC 合同。

### Shared Rust boundary

1. 在共享 Rust 层建立 typed xCloud catalog 能力：
   - xCloud titles、MRU、Game Pass newest 与 products hydration 请求。
   - JSON 变体解析、ID 规范化、图片 URL 规范化、分类和输入能力去重。
   - base + overlay 合并、排序、cache state 与 missing product 判定。
2. Tauri `XcloudService` 收敛为宿主适配：
   - 解析已有桌面 session。
   - 读写 settings store。
   - 投影现有 `DataXcloudCatalogPayload`，保持前端 RPC 合同稳定。
3. `xbox-ios-bridge` 增加 UniFFI 云目录入口：
   - 按需建立内存 `XboxCloudSession`。
   - 输出 `XboxCloudGame`、`XboxCloudCatalogSnapshot`。
   - 接受 locale、market、force refresh 和缓存摘要参数。
4. Swift 新增 `CloudLibraryStore`：
   - 管理目录加载、后台刷新、账号切换和错误状态。
   - 从 `XboxDataStore.games` 构建 `xboxTitleId` 活动索引。
   - 生成 `CloudLibraryGame` 供页面展示。

共享 crate 的最终命名与 Rust 模块组织在执行 RFC 中确定。推荐职责边界为：`xbox-webapi` 负责 HTTP 端点，新的游戏库领域模块负责 typed normalization 与 snapshot assembly，宿主负责会话和持久化。

### iOS authentication flow

1. App 恢复现有 Keychain 会话并加载个人资料与 TitleHub 活动。
2. 用户进入游戏库时，`AuthStore` 串行发起 `prepareCloudAccess`，Rust 使用 refresh token 与 JWK seed 生成 xCloud token，沿用 `xgpuweb -> xgpuwebf2p` 回退。
3. `prepareCloudAccess` 返回轮换后的持久凭据与仅驻留内存的 cloud access handle；`AuthStore` 原子更新 Keychain，继续作为唯一凭据写入者。
4. Rust 选择 token 中默认 region，目录请求复用内存 cloud access handle。
5. 云目录请求遇到 401/403 时由 `AuthStore` 串行重建一次 cloud access并重试一次。
6. App 进入后台时保留短期内存 access；退出登录和账号切换时释放 access并清理账户 overlay。

### iOS cache plan

- 存储位置：Application Support 下版本化 JSON 快照，启用系统 Data Protection。
- base key：`schemaVersion + market + language`。
- overlay key：`schemaVersion + xid + regionHost + market + language`。
- 启动顺序：读取可渲染 snapshot → 立即更新 UI → 判断新鲜度 → 后台刷新 → 原子替换 snapshot。
- 清理规则：退出清理当前账号 overlay；版本迁移清理旧 schema；base 元数据按最近使用时间回收。
- 图片继续使用系统 URL cache；快照保存远端 URL。
- 同一 scope 的刷新通过 single-flight 合并，前台刷新与自动刷新共享同一结果。

这一 v3 scope 将产品元数据与账号动态层拆分。Tauri 现有 v2 缓存通过一次兼容读取迁移到新结构，迁移完成后继续使用共享规则。

### Merge and conflict rules

- 目录 base：同 scope 采用最新成功 `fetched_at`。
- 目录 overlay：一次成功刷新形成完整新版本，按 snapshot 原子替换。
- 账户活动：`lastPlayedAt` 采用最新时间；时长与成就采用本次成功返回值；缺失补充值沿用当前可用值直到 snapshot 过期。
- 多会话并发：请求携带 account generation，只有当前 generation 可以提交状态。
- 未来用户 overlay：每个字段携带 `updatedAt + deviceId`，删除使用 tombstone，服务端按字段级 last-write-wins 合并。

### UI delivery scope

首期恢复 Tauri 的三组浏览语义：

- 最近游玩：优先使用 xCloud MRU；已关联 TitleHub 时展示最近时间、时长和成就摘要。
- 新上架：使用 Game Pass newest overlay。
- 全部游戏：按本地化名称排序，支持搜索和输入类型筛选。

卡片点击先进入游戏详情，详情保留 `streamTitleId` 供后续串流任务使用。首期重点交付数据正确性、缓存体验和状态完整性。

### Delivery phases

#### Phase 0：契约与在线证据

- 使用同一真实账号记录 Tauri 的 scope、目录数量、ID 样本和 fallback 行为。
- 验证 iOS 真机可取得 xgpuweb 或 xgpuwebf2p token。
- 固化匿名 JSON fixtures，覆盖 `/v2/titles`、MRU、newest 和 products hydration 变体。

#### Phase 1：共享 Rust 目录核心

- 建立 typed DTO、解析、规范化和 snapshot assembly。
- 让 Tauri 通过共享核心输出原有 RPC payload。
- 对同一 fixtures 做新旧输出差异测试，字段与排序保持一致。

#### Phase 2：iOS bridge 与缓存

- 增加按需 cloud session 和 UniFFI records。
- 建立 iOS versioned snapshot repository。
- 覆盖首载、stale refresh、离线、401 重建、账号切换和取消请求。

#### Phase 3：iOS 游戏库 UI

- 接入最近、新上架、全部、搜索、刷新、空态、离线态和错误态。
- 合并 TitleHub 活动信息。
- 完成 VoiceOver、动态字体、Reduce Motion 与图片降级。

#### Phase 4：串流入口衔接

- 将卡片 `streamTitleId` 交给原生 streaming runtime。
- 串流会话复用当前 cloud session 的 token 与 region 上下文。
- 使用单独 RFC 明确 libwebrtc、VideoToolbox、Metal、音频与生命周期边界。

### Acceptance gates

- 同一账号、region、language、market 下，Tauri 与 iOS 的目录 product 集合一致。
- 随机抽样至少 20 个游戏，名称、图片、分类、输入能力、entitlement、recent/new 与桌面一致。
- `productId -> streamTitleId -> xboxTitleId` 映射覆盖率可观测，关联失败项保留在目录中。
- 热缓存首屏在本地读取完成后可立即展示；stale 数据在后台刷新完成后原子更新。
- 飞行模式可展示 24 小时内 overlay 与 7 天内 base。
- 账号切换后的状态提交只接受当前 account generation。
- refresh token、JWK、Web Token、xCloud token 全程留在安全存储或 Rust 内存；缓存与日志只保存业务 DTO 和诊断摘要。
- `cargo test` 覆盖共享核心与 bridge；`cargo check -p xbxrc` 保持桌面兼容；iOS Device/Simulator build 与 XCTest 通过；Rust 执行 `cargo fmt`，Swift 执行项目约定验证。

## Open Questions

- 真机在线请求需要确认 xgpuweb 与 xgpuwebf2p 在目标账号、网络和地区下的可用率。
- market 优先读取 xCloud token 的服务端值，系统 locale 只决定 language；在线样本需要确认两者组合的产品元数据覆盖率。
- `xboxTitleId` 缺失或复数产品映射同一标题时，需要用 fixture 统计覆盖率，再决定是否增加别名表。
- 首期详情页可展示的信息范围需要与串流 Phase 4 的交互设计一起确认。
- 用户收藏、置顶、隐藏和排序进入产品范围时，需要选择 CloudKit 或自有后端并补充隐私与迁移策略。

## Candidate Follow-On Tasks

1. **复杂任务：iOS/Tauri 共享 xCloud 游戏库核心 RFC**
   - 明确 crate/module 边界、typed DTO、v2→v3 缓存迁移、UniFFI 合同和双端兼容策略。
2. **复杂任务：共享 Rust 目录核心与 Tauri 回接**
   - 抽取解析和合并逻辑，建立 fixture 差异测试，保持桌面 RPC 输出稳定。
3. **复杂任务：iOS xCloud 鉴权、bridge 与缓存链路**
   - 实现内存 cloud session、token 回退、snapshot repository 和会话隔离。
4. **复杂任务：iOS 游戏库页面与账户活动合并**
   - 交付最近、新上架、全部、搜索、刷新、离线状态和详情入口。
5. **复杂任务：iOS 云串流入口 RFC**
   - 从 `streamTitleId` 接入原生 streaming runtime，并定义完整媒体与生命周期验收门禁。
6. **候选复杂任务：用户游戏库 overlay 跨设备同步**
   - 在收藏、置顶、隐藏或排序进入范围后，设计 CloudKit/后端数据模型、冲突和 tombstone 规则。
