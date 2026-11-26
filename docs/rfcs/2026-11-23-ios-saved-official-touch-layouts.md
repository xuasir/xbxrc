# iOS 游戏触屏布局本机保存与串流实时切换 RFC

## Status

- Completion: 未完成
- Current State: validation-pending
- Owner: 主窗口，单窗口直接执行
- Last Updated: 2026-11-25
- 执行授权：用户于 2026-11-24 明确回复“实施”，主窗口按本 RFC 直接执行。
- 已确认来源：官方触屏布局；用户于 2026-11-24 扩展授权，官方与 better-xcloud 社区布局统一进入可保存列表。

## Background

用户希望在云游戏页面选取某个游戏的官方适配触屏布局并保存到本机，在 Home 串流中按正在玩的游戏实时切换布局。

当前 iOS 正式触屏使用 `TouchGamepadOverlay → TouchInputAccumulator → StreamInputSession`；`StreamingTouchLayoutController` 初始化时读取通用 Xbox 资源。现有本地自定义模板记录标准控件的位置和外观，模板控件身份由 `TouchLayoutTemplate.standardControlIDs` 校验。

云游戏通过公共 Rust `XcloudApi.get_titles_by_ids()` 的 `POST /v2/titles` 按商品 ID 查询标题详情，布局目录来自 `details.supportedTabs`。公共 `input_configs()` 对应 Home 串流中的动态标题输入配置。官方布局下载 API、真实 `supportedTabs` 返回和 TAKX 包消费进入本任务实施范围。游戏列表中的 `supportedInputTypes` 用于展示输入能力，实际布局可用性由标题详情和布局包共同判定。

官方 TAKX 分发为 ZIP 包，包含 Manifest、Context、多个 layout JSON 和图片资源；历史原文显示先按 `manifestVersion/layoutVersion/tabVersion` 选择包，再按语言和 DPI 加载布局与资源。当前 Swift normalizer 支持直接 Layout 和常用控件，并对 `$ref`、部分 schema、资源及行为生成兼容性诊断。

相关历史和约束：

- [原生触屏方向与公共资源边界](../isu/2026-11-17-ios-touch-controls-implementation-options.md)：Rust 承接官方资源、引用和纯数据规则，Swift 承接多指、几何与绘制。
- [布局与输入组件](2026-11-18-ios-touch-layout-playground.md)：通用、Forza 社区样本、官方最小 Layout 和能力报告已验证。
- [本地模板与可视化调整](2026-11-20-ios-touch-layout-customization.md)：自定义模板、独立横竖屏位置和外观持久化已交付。
- [官方资源入口审计](../../dev-docs/validation/2026-11-18-ios-touch-sampling/official-layout-resource-audit.md)：官方 `getTouchAdaptationBundle(user,id)` 调用链有历史依据，当前实际下载合同需要核对。
- [官方包格式与 Context 规则](../references/better-xcloud-touch-layout-protocol.md)、[官方触屏协议](../references/official-touchscreen-protocol.md)：用于包版本、默认布局、引用、资产和动态状态的兼容依据。

当前安全区与串流菜单任务仍在验收。本任务沿用工作区内的最新实现，布局库和选择面板共用正式安全区、公共浮层及输入接管机制。

## Goal

1. 云游戏详情页提供“触屏布局”入口，并行查询官方包和 better-xcloud 社区布局，预览并选择需要保存的布局。
2. 已保存的布局、必要资源和默认状态持久保存在本机；应用重启和后续 Home 会话均可本地加载。
3. Home 串流菜单提供“切换触屏布局”，统一展示通用 Xbox、本地自定义模板和已保存的游戏布局。
4. 点选后当前串流即时使用对应布局，媒体会话持续运行；选择面板和菜单关闭后恢复游戏输入。
5. 每次布局替换完整释放旧触点、按键、摇杆、扳机、toggle/sticky 和 layer 贡献。
6. 串流页提供一个快捷按钮，在标准布局与最近使用的自定义/官方布局之间循环切换。
7. 删除本机布局时清理其编译资源；共享包在最后一个引用删除后回收。
8. Cloud 串流布局选择绑定已通过资格校验的当前游戏；进入选择列表只查询目录，点击后下载、校验并切换，成功包在本次会话内复用。

## Scope

当前范围覆盖 iOS 云游戏详情、主机已安装游戏详情、本机布局库、布局包与资源兼容、Cloud 当前游戏按需选择与 Home 已保存布局切换，以及同一套触控运行时和输入链路的验证。官方包下载和低频资源操作通过现有 Rust/UniFFI 边界装配，UI 和输入热路径保持 SwiftUI/UIKit。

Home 按保存包中的默认 Context 加载，用户可手动切换同一游戏的多个可用布局。自动识别主机前台游戏、跟随游戏场景的服务端 Context/Patch、云端同步和游戏原生触点分别进入后续独立范围。动态状态依赖写入布局兼容说明，并以代表性官方样本验证可玩范围。

### 2026-11-25 Cloud 按需切换合同

- 串流资格校验返回的商品、串流标题、Xbox 标题及游戏名共同建立当前 Cloud 布局范围；继续复用公共 Rust API 与唯一触屏 controller。
- 打开布局选择只查询当前游戏的官方/社区目录，先展示来源与版本；用户点击对应条目后才下载包、准备默认布局并切换。包加载成功后展示包内各布局，后续点选复用会话缓存。
- Cloud 包与布局选择保持会话级身份，当前游戏或会话改变使旧查询/下载结果失效。Cloud 快捷切换使用当前游戏的最近选择，Home 最近快捷身份保持本机偏好合同。
- Home 选择器继续展示已保存游戏布局、本地模板与标准布局；Cloud 选择器展示当前游戏来源和标准布局。两种会话沿既有 neutral 帧、触点释放、菜单抑制与媒体链路完成切换。
- 目录和下载失败使用共享来源重试；失败保持当前有效布局。正常下载入口和两种串流切换分别提供成功证据，来源隔离的故障注入截图保留为回归证据。

## User Flow

### 云游戏详情：保存适配布局

- 游戏详情增加紧凑的“触屏布局”区域，入口为“查看游戏布局”；首次进入查询云标题详情和官方包目录。
- 查询过程中保留游戏详情与开始串流入口，布局区域显示局部加载、失败和重试状态。
- 列表展示包版本、布局名、默认标记、兼容状态和保存状态；选中后使用正式 Surface 本地预览。
- 点击“保存到本机”后取得包与必要资源、完成解析和能力校验，再原子提交本机记录；成功反馈显示游戏名和布局名。
- 同一游戏的多个布局共享包资源。重复保存同一版本复用已有内容；更新由用户点击“更新布局”触发，校验成功后替换本机记录。
- 仅支持原生触点的游戏和缺少可消费官方布局的游戏分别显示原因；可玩列表以实际通过校验的手柄布局为准。

### 触屏设置：管理已保存布局

- “控制器 → 触屏手柄”增加“已保存的游戏布局”，按游戏分组展示布局名称、版本、占用和删除入口。
- 保存项的删除按钮位于行尾；“默认布局”以标题右侧的小字样展示。
- 通用模板的拖动、另存、删除和横竖屏位置逻辑沿用；官方保存项持有完整布局内容及资源身份。
- 删除当前会话正在使用的保存项时，先完成通用布局切换和输入释放，再删除资源；共享资源按引用计数回收。
- 编译快照按布局引用回收；展开布局、图片资产与包原件在最后一个共享包引用删除后回收。索引提交失败保留原有效资源，清理失败提示重试，重新读取布局库时再次回收。

### Home 串流：实时选择

- 功能菜单增加“切换触屏布局”；打开沿现有菜单接管规则抑制游戏输入，暂停自动隐藏。
- 面板展示通用 Xbox、自定义模板和按游戏分组的本机官方布局，提供搜索和当前选中标记。
- 点选时异步准备本地布局与资源，完成后在当前会话提交新布局；列表只呈现可用的本机项。
- 选择成功后关闭面板并回到串流菜单；关闭菜单后，新触点开始操作新布局。
- 读取失败时保留当前可用布局并显示局部提示；当前项失效时回退通用 Xbox。
- 布局库跨启动保存；当前选择属于串流会话。新 Home 会话以现有触屏偏好启动，最近使用的游戏布局排在列表前部。
- 菜单收起时，串流页保留一个布局快捷按钮，显示当前“标准”或“最近”；每次点击在两者之间循环。最近身份跨启动保存，覆盖本地自定义模板和官方保存项；删除最近项时清理对应记录。
- 最近布局准备期间再次点击，立即取消旧准备并切回标准；沿独立布局切换输入抑制原因完成旧触点释放与 neutral 帧交付。

## Resource and Storage Contract

| 身份或字段 | 语义 |
| --- | --- |
| `productID` | 云游戏详情与展示关联 |
| `xboxTitleID` | 官方包按商品与标题共同核验；社区按已校验的 Xbox 标题文件关联 |
| `streamTitleID` | 云串流会话目标，保持现有职责 |
| `bundleID / bundleVersion` | 官方包及版本身份 |
| `manifestVersion / layoutVersion` | 格式兼容判定；与包自身版本分别保存 |
| `layoutID / displayName / defaultLayout` | 包内布局身份、名称及默认选择 |
| `contentHash / revision` | 内容完整性、原子更新和加载匹配 |
| `locale / assetVariants` | 保存的语言及可用图片密度变体 |
| `compatibility / savedAt / lastUsedAt` | 支持状态、保存时间和最近使用排序 |

- 资源位于应用 `Application Support/TouchLayouts`，具有用户主动保留的生命周期；目录元数据与资源操作由现有 Rust 桥接统一编排。
- 保存版本化索引、官方包原件、有效布局和必要资产，以及默认 Context 和引用来源，支持离线展开与重新校验。
- 每个保存项关联游戏、包和布局，共享资产按内容身份去重。标准模板继续使用当前 `MobilePreferences` 模型和数量规则。
- 官方索引独立记录完整游戏布局。会话选择用类型化身份表达“现有触屏偏好”或“本机游戏布局”，由正式 controller 统一解析。
- 布局库为设备级内容；下载使用当前云游戏授权，异步提交校验账号请求代次。本机记录保存游戏及资源元数据，认证凭据沿现有安全存储管理。
- 保存和更新采用任务目录准备、文件校验、原子索引提交；取消和失败回收本次目录，已有有效版本继续可用。
- 解压校验路径、文件数量、压缩及展开大小；JSON 保持现有单文件 2 MB 和递归深度约束。初始包预算以代表性官方原件确定并写入常量和验收记录。

## Official Bundle Compatibility

1. 沿公共云标题查询接口提取真实 `details.supportedTabs`，同时匹配商品 ID 和 `details.xboxTitleId`，记录 `id/tabVersion/manifestVersion/layoutVersion`，按实现能力选择最新可用包。下载前沿同一接口核验包归属。
2. 核对当前公开客户端的下载请求、认证和二进制返回，新增公共 API；二进制读取采用独立有界策略，复用现有 HTTP 客户端及授权续期。
3. Rust 完成 TAKX 解包、Manifest 校验、语言回退、资产密度选择、definitions 和 Context 引用展开；交付默认状态下的低频布局快照和本地资产身份。
4. Swift 扩展现有 normalizer 和 Surface，消费已解析官方布局及必要图片资源；控件状态机、几何、来源聚合和 `TouchCapabilityReport` 继续共用。
5. 必需控件、动作、引用或资源影响可操作性时，将布局标记为待适配并显示原因；可保存并应用的布局须通过完整操作语义校验。
6. 版本和兼容性以真实官方包矩阵为依据；本机切换沿标准虚拟手柄输出，会话触屏协议能力沿当前实际实现声明。

## Live Switching and Input Contract

- 会话保存 `TouchLayoutSelection` 与单调递增的选择 revision；controller 保持当前 `TouchInputAccumulator` 和唯一 `StreamInputSession` 绑定。
- 切换分为准备和提交。本地读包、资产解码和能力检查在准备阶段完成；结果须匹配 session generation、选择 revision 和资源 revision。
- 提交时取消旧触点、持续贡献和 layer，清理触控捕获及旧布局配置，经既有输入出口交付 neutral，再装载新布局并生成最终 sockets。
- 菜单输入抑制期间继续执行内部取消和装载；恢复输入从新的触点生命周期开始，发送器沿现有唯一消费出口运行。
- 快速连续选择只消费最后一次有效结果；面板关闭、会话结束和资源删除使相应旧请求失效。
- 通用和自定义模板保留现有位置与外观解析；官方布局的位置和动作来自包内容，透明度与缩放取现有全局触屏配置。
- 横屏全窗口、竖屏下半区、安全区、至少 44pt 命中与绘制几何共用正式 Surface；旋转、后台、实体手柄、菜单和关闭沿现有释放合同验证。
- 诊断按需记录布局准备/应用/失败、来源类型、revision、持续时间和释放结果；输入热路径沿当前采样与限频策略。

## Impacted Modules

| 模块 | 职责 |
| --- | --- |
| `crates/xbox-webapi/src/xcloud_api.rs` | 配置类型和官方二进制下载 |
| `crates/xbox-touch-layouts/` | TAKX、Manifest、Context 引用和资源的纯数据处理，沿 Rust 模块组织规范独立承载 |
| `crates/xbox-ios-bridge/src/touch_layouts/` 及现有导出 | 复用授权租约、本机资源库编排、粗粒度 UniFFI 接口 |
| `iosapp/XBXRC/Platform/RustBridge/` | 布局客户端协议、桥接和可替换测试客户端 |
| `Shared/State/`、`AppRootView.swift` | 本机布局库状态和根级装配 |
| `Features/Library/GameDetailView.swift`、官方布局选择页 | 查询、预览、保存和更新 |
| `Features/Controllers/ControllerView.swift`、本机管理页 | 保存项管理和删除 |
| `Features/TouchControls/` | 官方布局兼容、资产绘制和已有 Surface/输入复用 |
| `Platform/Streaming/StreamingRuntime.swift`、`TouchGamepadOverlay.swift` | 会话选择、异步匹配、实时装载和释放 |
| `StreamingControlsOverlay.swift`、`StreamingControlsState.swift` | 选择浮层、菜单接管和自动隐藏协调 |
| 现有绑定生成脚本、工程、语言资源及测试 | 绑定复现、中英文、源码注册和验证 |

## Plan

1. 官方合同和原件：核对配置、真实下载及认证，取得代表性官方包，固定支持矩阵与资源预算。
2. 资源和存储：包处理、引用与资产、版本化索引、原子保存、更新和删除。
3. 官方兼容：按样本补齐必要 schema/资源与控件覆盖，接通现有预览和正式 Surface。
4. 产品入口：云游戏详情保存、本机布局管理和 Home 菜单实时选择。
5. 验证收口：定向单元/UI、设备编译、实际 Home 输入、证据归档、Report 和清理。

## Validation

- [x] 两个微软官方发布的完整 TAKX 样例通过解析、默认布局、原始图片、预览、保存和重启加载；七份原始布局文档均含 UTF-8 BOM。通用来源、缓存刷新、晚到旧结果和两种下载重试完成单轮定向回归。
- [x] 两个详情下载入口的官方／社区成功流程、Cloud 当前游戏按需查询下载和包内缓存、Home 保存布局与标准／最近快捷通过 7 项单元及 3 条 UI；横屏选择器增高后的滚动点选通过，修复前诊断和最终截图归档。
- 用户于 2026-11-24 明确要求每轮收尾使用最小冒烟；当前复用已通过结果，新增修复做定向回归和一次设备编译。实际包验收收敛为一份应用内保存的官方布局，更多游戏和长测进入后续实机验收范围。
- [ ] 实际官方游戏包的来源、版本、Manifest、默认 Context、资产和能力矩阵可复核；下一步读取已登录应用内正常保存的布局资源。
- [x] 构造包中同一游戏多个布局、语言回退、图片密度、引用和默认状态离线加载有效。
- [x] 保存、重复保存、重启恢复、更新失败保留旧版、删除、损坏索引、资源预算和回收错误路径完成定向验证。
- [x] 云游戏详情入口使用测试客户端完成查询、重试、预览、多个布局保存和删除。
- [x] 串流菜单构造会话完成通用、自定义和两个游戏布局切换，单按钮循环切换与游戏输入回放有效。
- [x] 按键、双摇杆、扳机、toggle/sticky、layer 和多指完整释放；旧布局和旧会话回调按代次失效。
- [x] 快速选择、输入抑制、旋转、实体手柄 neutral gate 和当前资源删除通过定向回放。
- [x] 手机/iPad、左右横屏及竖屏 UI 流程通过，操作尺寸断言与界面截图归档。
- [x] 会话边界检查通过，资源准备在现有低频客户端执行，输入与媒体复用原链路。
- [x] Rust 定向测试与 `cargo fmt`、Swift 单元/UI、绑定生成、iPhoneOS 编译、工程/会话边界和仓库 lint 已通过；本轮格式检查通过。
- [ ] 实机 Home 动作、手感与持续串流由用户验收；trace 同时覆盖网络、恢复链、media/steady supply、submit/present 长尾和帧供给 delta。
- [x] 截图、测试摘要、失败诊断和源码指纹归档；确认进程结束后清理本轮登记路径并核验空间。

## Risks

- 官方 API 的真实下载合同仍需验证；首个里程碑固定证据，再将确定接口接入产品入口。
- 官方版本、图片与控件范围较广，兼容判定须基于完整操作语义，默认状态依赖在预览和实机验证中确认。
- Home 场景动态联动依赖主机游戏提供控制消息；首版本机切换以默认 Context 和手动选择建立可玩范围。
- 异步加载与菜单抑制交接可能产生旧结果覆盖和输入残留，需通过代次匹配和统一释放回放验证。

## Progress

- [x] 核对模板、官方协议、云数据接口、正式触屏和菜单接管。
- [x] 确认来源为云游戏官方触屏布局。
- [x] 完成本机保存、多个布局、会话切换、边界和验收方案。
- [x] 用户确认实施。
- [ ] 官方合同和真实包验证。
- [x] 资源库和布局兼容实现；真实包兼容矩阵接续验收。
- [x] 云游戏/设置/串流入口实现；手机完整流程通过，iPad 接续验收。
- [ ] 验证、Report 和清理。

## Execution Notes

- 2026-11-23 | planned：按 `task-run` 进入跨模块 RFC 路径，用户确认官方来源；本轮完成方案和任务登记，当前阶段为实施前确认。
- 官方获取链以当前源码与真实返回为准，历史冻结脚本提供结构和行为依据；现有社区 Forza 资源保留独立来源身份。
- 实施临时根预定为 `/private/tmp/xbxrc-ios-saved-official-touch-layouts-20261123/`，按轮次隔离并登记实际路径。本轮方案阶段临时产物为零。
- 最终证据目录预定为 `dev-docs/validation/2026-11-23-ios-saved-official-touch-layouts/`，按实际执行轮次归档。
- 2026-11-24 | in-progress：用户明确授权“实施”；实际临时根为 `/private/tmp/xbxrc-ios-saved-official-touch-layouts-20261124/r1/`，已登记工作区及暂存基线。按 Rust 模块组织规范将纯包处理放入 `xbox-touch-layouts`，HTTP 留在 `xbox-webapi`，本机存储适配留在现有 iOS 桥接。
- 2026-11-24 | continuation：主窗口接续原聊天实施与验收，补齐删除按钮行尾和标题右侧默认标记，修复行容器覆盖子按钮可访问性身份与 44pt 浮点比较；本轮临时目录登记为上述任务根下的 `r2/`。
- 2026-11-24 | continuation-r3：关联聊天的实施与登录验收选择已读取；手机 3 项 UI 流程、54 项定向 Swift 测试、12 项布局包测试、75 项公共 API 测试及 12 项本机资源库测试已有通过记录。接续真实官方包、iPad UI 和 iPhoneOS 编译；本轮临时目录为上述任务根下的 `r3/`。当前桌面云游戏授权已过期，已请求用户完成其选定的重新登录步骤，独立验收继续执行。
- 2026-11-24 | validation-pending：iPad 3 项 UI、本轮 60 项 Swift 定向验证及 iPhoneOS 编译通过，新增两项测试覆盖首次读取成功前保留最近布局、读取失败保留和成功空列表清理。用户要求最小冒烟，后续复用已有结果。用户确认登录位于 iPad 模拟器；外部 CLI 访问 Keychain 返回 -34018，复制应用签名/访问组的提案被自动审批拒绝，实际包采用应用内正常保存后读取资源的路径。14 张最终截图、1 张定位图、测试摘要、诊断和 47 份源指纹已归档至[证据](../../dev-docs/validation/2026-11-23-ios-saved-official-touch-layouts/README.md)；真实官方包和 Home 实机待验，最终 Report 随完整验收收口。
- 2026-11-24 | cleanup：确认相关构建、测试和导出进程结束后清理 r1/r2/r3 登记临时根，删除 4909 个文件、原占用约 2.30 GiB；观察到可用空间增加约 2.26 GiB，清理后可用约 46.38 GiB。登记路径已不存在，共享 SDK/Rust 缓存、应用本机布局库与登录保留。[清理记录](../../dev-docs/validation/2026-11-23-ios-saved-official-touch-layouts/cleanup.json)。
- 2026-11-24 | bugfix / validation-pending：用户反馈布局目录全部 404 和重试样式偏差。公开 SDK 的商品身份查询使用 `alternateIds` / `alternateIdType`；公共 Rust 原请求写入 `ids` / `idType`，本轮已修正合同。查询、下载、预览统一复用 `SectionRequestFailure` 与 `AppSectionRequestState`，新增 `titleIdentity` / `inputConfigurations` 阶段诊断。5 项 Rust 合同与 1 条 iPad UI 冒烟通过，Mac 签名应用编译成功；XCTest 启动在建立连接前被系统终止，本轮真实请求为 0，服务回包继续由正常 Xcode 应用验证。临时测试探针和方案已恢复，截图和摘要见[本轮证据](../../dev-docs/validation/2026-11-24-ios-official-touch-layouts-404/README.md)。
- 2026-11-24 | cloud-catalog-fix / validation-pending：用户重跑后仍见加载失败。核对微软 `game-stream` 调用上下文确认 Cloud 从标题详情 `details.supportedTabs` 初始化布局，Home 的动态标题才调用 `inputconfigs`。目录与下载前归属校验共用商品 ID 的 `/v2/titles` 查询，同时核验 `details.productId` / `details.xboxTitleId`，空或 null 布局目录进入空状态；诊断阶段更新为 `cloudTitleDetails`。4 项目录回归、绑定生成与 iPhoneOS 编译通过，复用已通过的共享重试 UI 冒烟；已请求正常 Xcode 应用确认一次预览和保存。r2 临时根为 `/private/tmp/xbxrc-ios-touch-layouts-404-20261124/r2`，清理与来源摘录归档到上述证据目录。

### 2026-11-24 官方版本兼容与社区来源接续

- 用户明确要求参考本机 better-xcloud 获取布局，官方和社区统一保存；本次“继续”接续该执行授权。
- 诊断 seq 75 → 91：目录请求返回后在 cloudTitleDetails 抛出“四段数字”解析错误；版本按 1–4 段非负数字补齐比较，保留显示原值。TAKX Manifest 采用相同兼容规则。
- 社区 HTTP 留在 xbox-webapi，按 gh-pages 不可变 commit 获取 touch-layouts/{XboxTitleId}.json 与 layouts/{name}.json，按 Xbox 文件身份关联，核验可选 product_id 与 schema_version，顺序完成所有引用合并。
- 纯 Rust 将社区文档封装为同一 TAKX 资源合同，预览、能力校验、本机保存与 Home 切换继续共用已有链路。社区目录 bundleId 携带 commit；保存 family 使用 community:better-xcloud，commit 保存为内容版本；已有官方索引保持可读。
- 两类目录独立查询；单来源失败显示共享重试卡片，另一来源仍可预览保存。页面、版本选择及空状态采用“游戏触屏布局 / 布局来源与版本”，显示官方/社区来源与原作者。
- 最小验证：Rust 定向版本、真实社区格式、保存重启/删除回归；Swift 编译及一条保存/切换 UI 冒烟。原始诊断保留在 Downloads，归档脱敏摘要；本轮临时根 /private/tmp/xbxrc-ios-game-touch-layouts-20261124/r1。

- 2026-11-24 | UI 接续：用户要求每份布局以纵向卡片呈现，卡片显示来源、版本、默认/保存状态及保存/更新按钮；点击卡片进入独立预览页。按包延迟下载，按布局按需准备预览和保存，沿用原能力校验及请求代次防护。新增 UI 验收仅覆盖卡片列表 → 全屏预览 → 横竖屏 → 返回保存。
- 2026-11-25 | 请求与资源消费接续：用户明确要求完整布局功能正确，范围覆盖全部游戏的官方/社区查询、下载、UTF8解析、无效布局隔离、预览、保存和失败重试。当前 better-xcloud 源码 `ea4d19f2800cee9cfa387a0c9c37b3711cf208c7` 和 gh-pages `4416b1ab07588a3244a5c02ce1fa5dfa64d8df74` 已读取；社区按 Xbox TitleId 的固定修订文件选源，`product_id` 为来源元数据，成功结果按标题复用。官方 TAK 先对包内 UTF8 文本进行 ECMAScript trim，再解析 JSON；文件与展开预算沿原合同执行。单份无效布局保持不可用诊断，其他有效布局继续预览与保存；用户重新查询更新目录缓存，过期查询完成保持代次隔离。地平线6作为已知上游社区选源的回归样本，普通游戏同时覆盖官方与社区路径。[本轮证据](../../dev-docs/validation/2026-11-25-ios-touch-layout-download-followup/README.md)。
- 2026-11-25 | 通用资源与请求验证完成：微软样例 commit `99d5449bb054908876ceb867f948652f3b9cb41b` 的两个完整 TAKX 包含七份 BOM 布局。布局 crate 23、bridge 15、Swift 9 与 UI 1 项通过，覆盖普通游戏双来源、失败/空目录重试、强制刷新、延迟旧结果、两种下载恢复、预览保存和重启；四张最终截图已查看。具体失败游戏包的编码与 Home 实机接续用户新构建验收。[修复 Report](../reports/2026-11-25-ios-touch-layout-download-followup.md)。

- 2026-11-24 | UI r2 接续：用户要求保存按钮固定卡片右下、箭头右上；点击直接进入全屏交互预览，旋转设备或窗口后同步横竖屏布局。退出动作在布局适配通过后完成本机保存再返回列表，保存失败保留预览并显示重试原因；待适配布局可查看和退出。临时根为 `/private/tmp/xbxrc-ios-game-touch-layouts-20261124/r2`，复用 r1 DerivedData，本轮仅一条 UI 冒烟。

- 2026-11-24 | UI r3 接续：按用户反馈压紧卡片，保存状态与胶囊保存按钮同一底行；全屏预览直接共用 `TouchLayoutWorkspace` 的居中工具栏、方向胶囊、设置面板与 `TouchLayoutCanvas` 的游戏占位/分区画布，返回与保存均执行本机保存后退出。调整页默认合同保持，预览传入已准备游戏布局与资产。本轮补充真实视口方向和按键边界断言，只执行原单条 UI 冒烟；临时根为 `/private/tmp/xbxrc-ios-game-touch-layouts-20261124/r3`。

- 2026-11-24 | UI r4 紧凑化：按用户要求，左侧名称/来源版本/保存状态三行与右侧箭头/保存共用约90pt高度；12pt内边距、4pt行距，底部操作仅占元数据右侧，以真实按钮宽度保留文本空间。保存按钮继续保持44pt命中，动态字号及窄宽度沿单行截断完整无障碍文字处理。本轮临时根 `/private/tmp/xbxrc-ios-game-touch-layouts-20261124/r4`。

- 2026-11-24 | UI 自动验收完成：最终1条iPad冒烟42.597秒通过，卡片≤96pt、按钮≥44pt、竖横竖视口/按键边界、返回保存和本机库均有效。预览与调整页共用正式工作区，整屏截图修正了横屏应用矩形裁剪。共享错误字段通过正式生成脚本同步，旧四参数构造使用nil诊断值兼容。[最新证据](../../dev-docs/validation/2026-11-24-ios-official-touch-layouts-404/community-and-cards/README.md)。真实官方包、Home实机与最终Report继续待验。

- 2026-11-24 | UI r5 最终调整：用户明确改为移除卡片箭头、保存按钮在右侧垂直居中；名称/来源版本/保存状态三行统一预留操作宽度，保持紧凑高度。临时根 `/private/tmp/xbxrc-ios-game-touch-layouts-20261124/r5`，前序预览/旋转/保存验证复用，本轮仅核对卡片和编译。

- 2026-11-24 | UI r5 自动验收：卡片前缀最小冒烟25.758秒通过，保存居中/高度≤96pt/按钮≥44pt断言和当前源码编译通过；仅导出最终卡片，前序全屏旋转与返回保存证据复用。临时UI前缀探针已精确恢复，源码指纹与定向差异已归档。

- 2026-11-24 | 最终清理：登记r1–r5与前轮404排查r3及一次性社区程序产物已删除，5581个文件，原分配空间约0.99GiB；探针精确恢复，路径缺失与磁盘空间已核验。最终预览/保存/紧凑卡片实现与自动冒烟完成，真实官方包和Home实机继续待验。[清理](../../dev-docs/validation/2026-11-24-ios-official-touch-layouts-404/community-and-cards/cleanup.json)。

- 2026-11-25 | Cloud / Home r2 自动验收：两个详情入口的官方／社区成功流程、当前游戏目录只查询、点选下载与包内复用、Home 保存项及标准／最近快捷完成。7 项单元和 3 条最终 UI 通过；横屏浮层目标高度提升到 320pt，受影响的 Cloud / Home 滚动点选定向补验通过。初轮定位/高度失败、最终截图、测试摘要及源码指纹归档，登记临时根按执行清单回收。正常账号服务及实机串流继续用户验收。[本轮 Report](../reports/2026-11-25-ios-touch-layout-download-followup.md)。
