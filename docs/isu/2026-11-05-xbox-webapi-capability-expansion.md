# Xbox WebAPI 能力扩展分析

- 日期：2026-11-05
- 状态：探索分析
- 参考源码：[`archived/xbox-webapi`](../../archived/xbox-webapi/README.md)，本地版本 `2.0.0-beta4`。
- 当前实现：`crates/xbox-webapi`、`crates/xbox-ios-bridge`、iOS 成就 / 个人 / 主机 / 云游戏页面，以及 Tauri data RPC。
- 证据等级：本地源码、类型定义与历史交付文档核对；在线服务权限、响应覆盖率与实时可用性保留到真实账号验收。

## 问题与机会

移动端已经形成 Xbox 登录、游戏历史、游玩时长、成就与串流的基本闭环。下一步可以把账户数据变成好友互动、游戏选择和远程主机管理入口。

本地参考库提供 16 组业务 provider，另有通用 REST provider。产品机会由三类证据组成：当前 Rust 已有接口和数据、参考库已经定义的新增 HTTP 方法、需要进一步研究协议和服务覆盖率的能力。

推荐首个完整增量为「好友列表 → 好友详情 → 同一游戏成就对比」。成就筛选适合作为独立的小步增强；主机已安装游戏适合作为下一阶段的串流功能扩展。

## 当前实现与约束

| 当前能力 | 代码事实 | 继续扩展时的关键点 |
| --- | --- | --- |
| 个人资料与本人活动 | `ProfileApi` 查询本人；iOS bridge 同时查询 Social、People、Presence | 当前 People 请求返回人员明细，Rust 只保留数组长度；人员记录可成为好友页的数据源 |
| 游戏历史 | `TitleHubApi::get_title_history(xuid)` 支持传入 XUID；bridge 从登录 token 取得本人 XUID | TitleHub 表达游玩历史；账号权益、主机安装状态和云游戏可用性分别使用各自来源 |
| 游戏成就 | Rust 按 XUID/titleId 查询；bridge 导出名称、两种描述、点数、解锁时间和进度 | 当前 Swift 页面有搜索和默认排序；新增筛选、用户选定排序与追踪目标属于增量 |
| 游玩时长 | `UserStatsApi` 查询 `Hero` 与 `MinutesPlayed`；最多 100 个标题、4 路并发 | 覆盖率由游戏上报决定；累计快照增量适合自启用日起的趋势统计 |
| 主机 | SmartGlass 设备列表；iOS 已有存储摘要、电源操作和串流入口 | 独立安装列表与状态查询可以补齐主机管理页 |
| 云游戏 | `XcloudApi` 加 `GamePassApi`，由共享 `xbox-cloud-catalog-flow` 合并目录、近期游玩与新游戏标记 | 产品元数据、账户串流权益、真实串流输入能力分别归属对应事实源 |
| 桌面复用 | Tauri data RPC 已装配个人资料、主机、电源、文本输入和云游戏目录 | 成就和好友页面需要补齐桌面 service/RPC/UI 装配 |

持续采用共享 Rust HTTP/API 与协议层；iOS 通过 UniFFI 消费稳定 record，桌面通过 Tauri RPC 消费稳定 DTO。应用状态、持久化与刷新协调由宿主负责，跨宿主共用的归一化逻辑按现有 Rust 模块边界集中维护。

游戏关联继续保留 `productId`、`xboxTitleId`、`streamTitleId` 三种身份；主机命令目标、主机串流目标和游戏身份各自有明确来源。

## 能力选项

| 产品能力 | 参考库方法 / 字段 | 当前 Rust 接入情况 | 预估复杂度与建议 |
| --- | --- | --- | --- |
| 成就筛选与追踪 | `achievements.getTitleId`，解锁状态、点数、时间和进度 | 当前接口与主要字段已覆盖 | 低；增加已解锁 / 待解锁 / 有进度筛选、选定排序、账号内本地追踪目标 |
| 最近解锁动态 | `achievements.getAchievements` 返回标题历史与 `lastUnlock`；按标题取成就明细 | 当前提供 TitleHub 与按标题成就查询；成就历史方法需新增 | 中；先选近期标题，再按需加载明细，提供明确覆盖范围 |
| 好友列表与在线动态 | `people.getFriends`、`userpresence.getFriends/getUser` | People 已发相同明细请求；Presence 当前仅装配本人 | 中；保留头像、Gamertag、游戏、在线状态和关系字段，复用稳定记录与按页刷新 |
| 好友资料与成就对比 | `profile.getUserProfile/getByGamertag`、`social.getSummaryByXuid`、`titlehub.getTitleHistory/getTitleId`、`achievements.getTitleId` | TitleHub、Achievements 底层已接受目标 XUID；bridge 固定使用本人身份 | 中；扩展目标 XUID 合同，按同一游戏匹配双方成就，提供共同完成 / 各自待完成视图 |
| 最近一起游玩的玩家 | `people.recentPlayers` | 当前 People 只查询社交人员列表 | 中；新增分页方法与详情入口，可复用好友记录与页面组件 |
| 个人游玩统计 | `titlehub.getTitleHistory`、`userstats.getUserTitleStats` | 当前已覆盖最近时间、成就汇总和累计时长 | 低至中；展示已有标题时长排行、全成就游戏和累计分数；周期趋势需要账号作用域快照 |
| 主机已安装游戏与容量 | `smartglass.getInstalledApps/getStorageDevices/getConsoleStatus` | 当前提供主机列表与嵌入存储信息；三个独立查询需新增 | 中；筛选 `isGame` / `contentType`，展示大小、安装时间、存储位置，并提供主机串流入口 |
| 手机远程文字输入 | 主线 `ConsoleApi::send_text` 已实现 `Shell / InjectString` | 桌面已装配，iOS data bridge 需新增 | 低至中；面向主机当前文字输入场景，处理受理、失败与刷新反馈 |
| 更丰富的游戏发现与详情 | `catalog.searchTitle/getProductId/getProductFromAlternateId`；`gamepass.getSigl/getProducts/getProductsDetailed` | Rust 已有 SIGL、products、hydration；目录归一化保留部分字段 | 中；补开发商、发售日、平台与游戏功能标签，按地区 / 语言读取；商店搜索增加独立 Catalog 方法 |
| Xbox 截图与录像相册 | `screenshots.getScreenshots`、`gameclips.getGameclips` | 需要新增 Media HTTP 模块与 bridge | 中至高；按游戏 / 时间浏览，播放、保存与用户主动分享；处理分页、过期资源和空相册 |
| Xbox 固定项与稍后游玩导入 | `pins.getPins/getSavedForLater` | 需要新增 Pins 模块 | 中；读取列表并关联产品 / 游戏身份，先验证当前账号与主机列表语义 |
| Xbox 服务状态提示 | `xnotify.getLiveStatus` | 需要新增状态查询 | 低至中；为登录或串流错误提供服务事故上下文，按场景 / 设备展示 |
| 消息收件箱与会话阅读 | `messages.getInbox/getConversation` | 需要新增 Messaging 模块与内容 record | 中至高；读取未读摘要与文字部分，处理内容类型、隐私、分页和消息可见性 |

### 成就增强的数据细节

- 当前列表已按待解锁优先、进度降序和解锁时间排序。用户可选择筛选与排序，追踪固定成就后可在游戏详情页提供入口。
- Rust record 已保留 `description` 与 `locked_description`；Swift 目前合并为可见描述。秘密成就的主动揭示需要在 Swift 模型中保留两种描述，并核对服务实际返回内容。
- 当前进度算法只读取 `progression.requirements` 的首个要求。复杂成就适合保留逐项要求及操作语义，展示可确定的进度事实。
- 本地成就 DTO 有 `serviceConfigId`、`productId`、`estimatedTime` 等可选扩展线索。成就对比适合保留 SCID，并以 SCID / 成就 ID 匹配；附加字段需要按真实响应覆盖率决定 UI。
- 稀有度与全平台解锁比例进入待研究项，需要带相应字段的真实响应样本和接口合同。

### 好友功能的实施形态

- 首屏以 PeopleHub 人员记录提供头像、Gamertag、状态文本和当前游戏；详细 Presence 按需求补充。
- 当前 `UserPresence` 只导出游戏名称，好友记录适合增加稳定 `xboxTitleId`，以此关联游戏目录与成就。
- 查看好友游戏历史及成就沿用当前登录者的凭据，目标身份使用独立 `targetXuid`；可见性由 Xbox 账户隐私设置决定。
- 成就缓存范围扩展为登录账号作用域、目标 XUID、titleId 和 locale；换号与请求代际继续遵循已有隔离合同。
- 刷新以当前可见页面、手动刷新及受限轮询驱动；好友状态展示采样时间，后台与串流期间按应用生命周期降低请求频率。

### 主机管理的实施形态

- 已安装应用返回 `titleId`、`oneStoreProductId`、`name`、`isGame`、`contentType`、`sizeInBytes`、`installTime` 和 `storageDeviceId`，足以形成真实主机游戏页。
- 安装条目可进入游戏详情、成就或该主机的串流入口。启动指定游戏的命令合同进入独立研究阶段。
- 存储设备返回默认盘、空闲容量和总容量；主机状态返回电源、播放状态与焦点应用 AUMID。
- 电源操作和文字输入分别使用已经明确的命令；命令受理、设备状态变化和串流就绪各自保留事实来源。

### 游戏发现的数据边界

- Game Pass 商品类型包括开发商、发行商、发售日、部分即将加入日期、平台、触控和功能标签。
- 现有云游戏目录已提供地区 / 语言隔离、产品关联、近期游玩、新游戏与账户串流 overlay，可复用其组织方式。
- 4K、HDR、120fps 等商品标签表达游戏的平台功能；串流实际画质、帧率和输入能力由串流服务与当前会话确定。
- 商品价格、折扣、付款和完整权益需要对应的商品 / SKU / 账户合同；本地 Catalog 目前定义的是 autosuggest 产品结果。
- 即将加入或离库信息需要按 SIGL、日期字段和账号市场实测；具体离库日期属于单独的覆盖率调查项。

## 推荐方向与阶段

1. **基础校准与小步成就增强**：补充真实响应 fixture，核对 Social 数量变体；提供用户可选择的成就筛选 / 排序与本地追踪。
2. **首个完整功能：好友与成就对比**：好友列表、好友详情、游戏历史、同一游戏成就对比顺序实施。已有 API 复用度高，可直接延伸移动端成就页。
3. **主机已安装游戏**：补三个 SmartGlass 查询，把主机页扩展为安装游戏、容量、成就和串流入口；远程文字输入可按同一主机管理主题接入。
4. **个人统计与游戏发现**：利用历史、累计时长和目录数据形成更多游戏选择入口；周期统计从快照开始建立。
5. **媒体相册**：以分页与资源可用性探针确认服务覆盖率，再安排图片 / 视频页面。
6. **辅助模块**：服务状态适合挂到现有错误页；消息与 Pins 在真实账号探针后确定投入。

优先级依据是已有数据复用、与成就 / 串流主流程的联系，以及持续刷新、媒体和多账号缓存的维护成本。

## 待补证据与研究项

- **在线权限与覆盖率**：用本人、公开好友、受隐私限制好友及空数据账号核对结果；记录 401/403/404/429 和服务错误对应的页面语义。
- **Social 响应变体**：参考库类型使用 `targetFollowerCount/targetFollowingCount`；当前 Rust 解析 `peopleCount.followers/following`。真实 fixture 决定兼容合同与统计含义。
- **媒体响应与分页**：参考类型使用 `values/contentLocators/expirationDate`。参考 HTTP 的 `next()` 总以 GET 拉下一页；媒体搜索为 POST，Rust 接入时需要保持请求方法、查询 body 与 continuation 合同，并验证实际分页位置。
- **用户搜索**：参考库有 `/suggest` 方法，其类型中的 `text/result` 覆盖有限；需要响应样本确定身份解析和结果展示。
- **服务状态地区**：参考方法写死 `GB/en-GB`，状态类型为 `CoreServices/Titles/Scenarios`；地区、语言与状态语义需要单独验证。
- **消息与列表的写操作**：消息发送、已读提交、好友关系修改、收藏写入分别需要新增接口合同与明确产品行为。
- **其他协议方向**：派对语音、邀请 / 加入多人会话、远程安装 / 卸载、指定游戏启动、云存档读写、统一排行榜、付费购买需要额外协议研究。
- **统计口径**：累计游戏时长可直接展示；周 / 月趋势由持续快照差值计算并标明区间和采样缺口。游戏自行上报的 Hero 统计需要按标题验证字段和含义。

参考库版本、类型和方法证明接口研究线索；真实账号验收决定当前上线能力。其现有测试覆盖客户端构造，在线端点需要独立验证证据。

## 候选后续任务

| 任务 | 类型 | 明确交付边界 |
| --- | --- | --- |
| 成就详情页筛选与用户选定排序 | simple | 复用当前数据，增加筛选 / 排序控件与状态展示 |
| Social Summary 响应变体兼容 | simple | 基于真实或可靠历史 fixture 对齐数量字段与缺失语义 |
| 本地成就追踪与秘密成就揭示 | complex | 账号内持久化、描述字段保留、揭示交互与换号清理 |
| 好友列表、详情与成就对比 | complex | Rust typed API / record、目标身份、缓存、隐私降级和移动端页面 |
| 主机已安装游戏与独立状态 / 容量查询 | complex | SmartGlass API、游戏身份关联、主机页与已有串流入口 |
| iOS 主机远程文字输入 | complex | 现有 Rust 命令的 bridge、输入场景门、操作反馈与真实主机验收 |
| 个人累计统计与周期快照 | complex | 覆盖率说明、快照采样、账号作用域与统计页面 |
| Xbox 媒体只读服务探针 | simple | 查询、响应样本、分页与资源失效验证；产出可用性报告 |
| Xbox 截图与录像相册 | complex | API、媒体 record、分页、预览 / 保存、资源刷新与页面 |
| Xbox 服务状态查询 | simple | typed 查询与响应 fixture；页面接入按现有错误处理范围安排 |

探索结论以本文保存；各功能在选题明确后由 `task-run` 进入执行追踪，复杂任务先形成 RFC。

## 证据索引

- [参考库 provider 注册](../../archived/xbox-webapi/src/lib.ts)
- [好友列表与最近玩家方法](../../archived/xbox-webapi/src/provider/people.ts)、[人员字段](../../archived/xbox-webapi/src/types/people/index.ts)
- [好友 / 指定用户 Presence](../../archived/xbox-webapi/src/provider/userpresence.ts)
- [成就查询与历史](../../archived/xbox-webapi/src/provider/achievements.ts)、[成就字段](../../archived/xbox-webapi/src/types/achievements/index.ts)
- [SmartGlass 查询](../../archived/xbox-webapi/src/provider/smartglass.ts)、[安装条目和容量字段](../../archived/xbox-webapi/src/types/smartglass/index.ts)
- [Game Pass 方法](../../archived/xbox-webapi/src/provider/gamepass.ts)、[商品字段](../../archived/xbox-webapi/src/types/gamepass/index.ts)
- [消息方法](../../archived/xbox-webapi/src/provider/messages.ts)、[Pins 方法](../../archived/xbox-webapi/src/provider/pins.ts)
- [媒体分页底层](../../archived/xbox-webapi/src/lib/http.ts)、[截图字段](../../archived/xbox-webapi/src/types/screenshots/index.ts)、[录像字段](../../archived/xbox-webapi/src/types/gameclips/index.ts)
- [Rust 当前 People 处理](../../crates/xbox-webapi/src/people_api.rs)、[Social 处理](../../crates/xbox-webapi/src/social_api.rs)、[主机命令](../../crates/xbox-webapi/src/console_api.rs)
- [iOS 数据 bridge](../../crates/xbox-ios-bridge/src/data.rs)、[Swift 成就描述映射](../../iosapp/XBXRC/Platform/RustBridge/XboxDataClient.swift)
- [iOS 成就页面](../../iosapp/XBXRC/Features/Achievements/AchievementsView.swift)、[数据状态](../../iosapp/XBXRC/Shared/State/XboxDataStore.swift)
- [共享云游戏目录](../../crates/xbox-cloud-catalog-flow/src/lib.rs)、[Tauri data RPC](../../src-tauri/src/mods/data/rpc.rs)
- [成就与时长交付报告](../reports/2026-07-14-ios-xbox-library-achievements-playtime.md)
- [个人资料与社交概况交付报告](../reports/2026-07-14-ios-profile-hero-social-presence.md)
- [云游戏库与身份分层 RFC](../rfcs/2026-07-16-ios-xcloud-game-library-detail-play.md)
- [iOS 账号 / 缓存协调约束](../rfcs/2026-08-05-ios-swift-architecture-hardening.md)
- [Rust 模块组织规范](../../dev-docs/rust-mod-organization.md)
