# PC 主线方案迁移到 iOS App 的可行性

整体可行，推荐沿现有 **SwiftUI + UniFFI + 共享 Rust + libwebrtc/Metal** 路线扩展 iOS App。认证、Xbox API 和会话层已经具备跨平台基础；主要投入集中在桌面服务的共享边界、iOS 新功能接线、输入合同和移动端验证。画面增强与 TAK 适合在基础游玩链路稳定后分阶段推进。

评估日期：2026-11-12。代码基线：`main` / `b864eb17`，结合当前工作区文档；范围为整体 PC 能力与架构。PC 参考 11 月的有效 RFC/Report，iOS 参考现有源码与 7–8 月的设计、实施和待验记录。本次产出为探索文档与候选任务。

## 1. 问题定义

迁移目标是让两端具有一致的游戏身份、账户准入、会话状态、输入语义和诊断口径，同时用 iOS 原生界面与媒体适配实现移动端体验。评估需要分别判断已有代码的复用程度、平台适配成本和当前验证成熟度。

PC 当前主线由 Vue/TypeScript 承接界面、Browser Gamepad、浏览器 WebRTC 与 WebGL；Rust 承接认证、Xbox API、会话/信令、配置及系统能力。iOS 已有独立平台宿主与原生运行时，这为功能迁移提供了明确落点。

## 2. 当前基础与约束

### 已有跨平台架构

`Cargo.toml` 已将 `xbox-ios-bridge` 纳入 workspace。该 crate 以 `lib/staticlib` 形式复用 `xbox-auth-flow`、`xbox-cloud-catalog-flow`、`xbox-streaming`、`xbox-streaming-protocol` 和 `xbox-webapi`；Swift 通过已有 UniFFI 接口消费粗粒度业务操作。

现有 iOS App 使用 SwiftUI、Keychain、版本化 UserDefaults、libwebrtc、GameController/CoreHaptics 和自定义 Metal renderer。工程覆盖 iPhone/iPad，最低 iOS 26.0。平台职责以 [iOS 架构加固 RFC](../rfcs/2026-08-05-ios-swift-architecture-hardening.md) 和当前源码为依据。

| 层次 | 迁移方式 | 持续边界 |
| --- | --- | --- |
| 公共 Rust crates | 复用现有依赖，扩展业务合同 | 认证、API、游戏身份、会话、信令和 wire 规则保持共享权威 |
| Tauri 服务 | 提取可移植规则和查询编排，再由原有 UniFFI 导出 | AppState、IPC、窗口、文件对话框与桌面存储适配归桌面 |
| Vue/TypeScript 页面与状态 | 复用信息结构、状态语义和验证样本，使用 SwiftUI 实现 | PC 界面继续 Vue；iOS 界面沿现有 SwiftUI |
| 媒体与设备热路径 | 使用现有 Apple 平台适配实现等价能力 | libwebrtc、CVPixelBuffer、Metal、GameController 与输入编码保持现有 Swift 路径 |

11 月的 [main/rust 发行隔离](../rfcs/2026-11-02-main-rust-release-isolation.md) 已收敛 PC 来源：`main` 交付 Browser，`rust` 承载 Rust 原生媒体/手柄 WIP。iOS 的当前原生 libwebrtc 路线有既有实现与历史决策；迁移参考应采用当前共享 crates 与 Apple 适配边界。7 月 iOS 对齐文档中的旧 `xbxengine` 路径、audio sendrecv 等条目，需要按现有协议抽离及接收音频权限合同更新。

### 能力迁移矩阵

“高”表示已有协议或实现支持该能力方向；发布成熟度仍取决于下面列出的接线与设备证据。工作量采用相对判断。

| PC 能力 | iOS 当前起点 | 迁移可行性 / 工作量 | 主要工作 |
| --- | --- | --- | --- |
| 登录、续期、账户权限 | Rust 登录/刷新、Keychain、credential revision、cloud/home access lease | 高 / 中 | 对齐所有消费入口的有效期、一次恢复、typed error、轮换凭据提交与重新登录语义 |
| 云目录、搜索、详情、收藏 | 目录首批加载/后台补资料、游戏详情、三种标题身份、账户范围缓存 | 高 / 中到高 | 补有序栏目、完整商品/语言资料、公开浏览与启动准入、本地收藏；导出 revision、增量和时效合同 |
| 主机、安装库、区域选择 | 主机列表、电源操作、xHome、容量与主机能力模型 | 高 / 中 | 补安装库接口、动态服务器区域及主机缓存状态；核对跨区发现、configuration/ICE 的最新投影 |
| 我的、好友、成就、统计、相册 | Profile、历史、时长、成就与社交概况 | 高 / 中 | 补好友列表/详情、完整累计统计、相册分页/预览与主动保存；扩展 UniFFI 数据接口 |
| 页面设计与导航 | 四个 Tab、NavigationStack、深浅主题与 Liquid Glass | 高 / 中 | 迁移信息层级和交互语义，按手机安全区/字号及 iPad 空间重排；手柄导航沿几何寻路合同做平台适配 |
| 会话、信令、ICE/TURN | 共享 SessionFlowService/Scheduler、scoped session、原生 Peer 与四通道 | 高 / 中 | 持续补齐新 plan/错误/候选来源字段、取消与重连时序，验证两类串流 |
| 原画与低延迟显示 | latest-only mailbox、CVMetalTextureCache、Metal、submit/present 统计 | 高 / 中 | 先完成当前 Device 稳定性、首显、终态与长测，按本机刷新率测量供给和上屏 |
| SR、USM/CAS、风格与画面保存 | Metal 输出与效果链设计基础 | 中到高 / 高 | 将算法与设置语义接入同一 Metal pipeline，补采样/色彩/尺寸/资源预算与像素、功耗验收 |
| 手柄映射、组合、菜单和震动 | GameController 实体输入、Swift 编码/背压/neutral drain、四路 haptics | 高 / 中到高 | 增加公共输入归属、映射、释放屏障、快捷键吸收和最新震动时序；按系统实际暴露的设备能力提供配置 |
| 开发/生产诊断 | schema v3、operation/attempt/generation、脱敏、轮转与系统分享 | 高 / 中 | 补常规/轻量、异常窗口、失败引用、历史恢复与有界导出，按移动端成本调整预算 |
| TAK 官方触控兼容 | 现有手柄输入协议与触摸平台能力；PC 已完成协议/引擎审计 | 中到高 / 高 | 两端共同补资源下载和 schema/命令；iOS 新增原生多指控件、聚合与绘制，按实际布局包扩大覆盖 |
| 更新与发行 | Xcode 工程、Rust 静态库构建与固定 WebRTC XCFramework | 高 / 中 | 建立 iOS 独立签名、版本/构建号、分发与验证合同，保持桌面发行来源规则 |

主要 PC 依据：[目录与交互](../rfcs/2026-11-11-cloud-catalog-ux-implementation.md)、[目录性能](../reports/2026-11-12-cloud-menu-performance-implementation.md)、[主机缓存](../reports/2026-11-12-home-host-cache.md)、[个人与安装库](../rfcs/2026-11-05-pc-console-library-my-pages.md)、[统一输入与映射](../rfcs/2026-11-11-gamepad-locked-input-and-extra-buttons.md)、[默认直通与效果链](../rfcs/2026-11-10-default-passthrough-render-engine.md)、[精简诊断](../rfcs/2026-11-11-pc-diagnostics-first-version.md)、[TAK 可行性](2026-11-12-tak-compatibility-feasibility.md)。

### 源码确认的关键差距

1. **桌面服务共享边界。** 最新主机 TTL/single-flight、目录 publication/revision、个人查询与下载编排位于 `src-tauri/src/mods/data`。iOS bridge 直接依赖公共 crates；这些桌面服务的复用需要拆出纯规则与平台适配。[主机缓存](../../src-tauri/src/mods/data/host_cache.rs)、[目录发布](../../src-tauri/src/mods/data/catalog_projection.rs)、[个人服务](../../src-tauri/src/mods/data/services/personal_service.rs)是具体起点。
2. **错误与合法空结果。** iOS `fetch_hosts()` 对查询失败和 8 秒超时返回 `Ok(Vec::new())`，使失败与合法空列表共享结果。迁移 PC 的缓存、刷新终态与错误诊断前，应保留 typed failure 与最近成功数据。[查询源码](../../crates/xbox-ios-bridge/src/data.rs#L89)。
3. **目录排序与新能力投影。** 公共 flow 已提供 `recent_rank/new_rank` 和完整商品能力；iOS `XboxCloudGame` 保留布尔标签，最后按名称排序。当前 bridge 导出目录与资料补齐接口，新栏目的顺序、完整详情、等待、公开浏览、收藏和增量版本需要补合同。[公共目录](../../crates/xbox-cloud-catalog-flow/src/lib.rs#L40)、[iOS 导出](../../crates/xbox-ios-bridge/src/cloud_catalog.rs#L10)。
4. **个人查询范围。** iOS 现有时长查询以最多 100 个标题、单标题四路并发为基础；PC 已引入按批查询及更广的统计覆盖。好友、安装库、相册的完整业务接口也需要在 iOS bridge 增加导出。[iOS 数据层](../../crates/xbox-ios-bridge/src/data.rs)、[PC HTTP 审计](../rfcs/2026-11-08-non-streaming-http-audit-fixes.md)。
5. **输入已经接通，语义需要追平。** 当前 Swift 直接读取活动 `GCExtendedGamepad` 并编码发送；PC 的 InputController 还管理 UI/stream/blocked、虚拟输入、独占录入、映射与交接释放。iOS 应在既有输入链上接入这些归属规则，并用共享字节/行为样本验证。[iOS 输入](../../iosapp/XBXRC/Platform/Streaming/XboxStreamDataChannels.swift#L350)、[公共输入控制](../../src/shared/gamepad/input-controller.ts)。
6. **震动存在具体版本差。** PC 最新有效实现已消费 12/13 字节格式和 `delay/duration/repeat`，保留弱短脉冲与长效果；iOS 百分比解码起点为长度至少 13，effect 仅保存 duration，haptics 将时长下限设为 10ms。应补格式及时序的同源 fixture，并在真实电机上验证。Share 试验已撤销，迁移基线采用最终保留的输入模型。[iOS 震动](../../iosapp/XBXRC/Platform/Streaming/IOSStreamInputProtocol.swift#L131)、[PC 最新结果](../reports/2026-11-12-official-rumble-timing-and-keyboard-share.md)。
7. **移动诊断具有既有 writer。** iOS production 预算为 4 个文件、每个 8 MiB；当前导出为 JSONL。PC 新方案的 12 条/64 MiB 问题历史、前后窗和 ZIP 清单需要适配到现有 Swift writer。当前 iOS 进入 background 会停止串流，采集持续性应按 scene 生命周期表达。[预算](../../iosapp/XBXRC/Shared/Diagnostics/IOSRuntimeTracePolicy.swift)、[导出](../../iosapp/XBXRC/Shared/Diagnostics/IOSRuntimeTraceWriter.swift#L178)、[后台终态](../../iosapp/XBXRC/Platform/Streaming/StreamingRuntime.swift#L388)。
8. **平台能力需要精确声明。** iOS 当前以 `browser_webrtc=true/rust_owned=false` 接入共享能力档案，实际 Peer 为原生 libwebrtc。新功能要核验对应消费点，例如 rVFC、VideoFrame/Worker 或反馈代理字段；原生适配按可观察证据实现。TAK 还需对齐 `touch_surface=true` 与当前握手 `touchInputEnabled=false`，通过实际引擎覆盖决定能力声明。[iOS plan](../../crates/xbox-ios-bridge/src/streaming.rs#L1006)、[共享握手](../../crates/xbox-streaming-protocol/src/stream_data_channel.rs#L53)。

## 3. 路线选择

| 路线 | 主要价值 | 成本与适用条件 |
| --- | --- | --- |
| **A：扩展现有 SwiftUI App，复用共享 Rust（推荐）** | 延续已有登录、会话、输入、音频和 Metal 投资，原生系统能力有稳定落点 | 数据/服务合同逐项共享，SwiftUI 和平台热路径按相同样本验证；当前成本最清晰 |
| B：将 PC Vue 装配到 Tauri Mobile | 页面及部分 TypeScript 状态有较高代码复用机会 | 需要重新评估 WKWebView 媒体/输入能力、移动插件装配和已有 Swift 模块接入，进入独立架构决策 |
| C：把 Rust Native WIP 扩展为手机媒体/输入宿主 | receive/recovery 内部事实与 Rust 实现可能形成更深的统一 | 需要证明 Apple 目标构建、解码/Metal/音频/手柄装配及手机功耗；当前迁移工作量最高，作为长期研究方向 |

路线 A 与仓库的现有 iOS 实现、单一 bridge、共享协议及平台职责一致，适合作为本轮迁移方向。

## 4. 推荐迁移方向

### 共享合同先于页面复制

优先在现有 crates 内整理游戏身份、资料与权益、请求错误、目录顺序/版本、设备事实和查询预算；桌面与 iOS 各自保留存储、凭据提交和生命周期适配。UniFFI 输出稳定 DTO 与粗粒度操作，Swift Store 在跨 `await` 提交时继续核验账户 scope 和 owner revision。

目录性能方案中的有界窗口、增量交付、缓存即显、图片按显示尺寸取样具有移动端价值。iOS 使用 SwiftUI 惰性视图、原生图片解码和当前缓存 owner 消费相同规则；TTL、LRU、批次和持久化频率通过手机测量确定。

页面迁移按“云目录/详情 → 主机/安装库 → 我的扩展 → 控制器/诊断”推进。先沿现有四 Tab 装配新能力，设置继续作为二级入口；iPhone 以触屏可达与安全区为基础，iPad 按有效空间规划列表/详情。新增手柄导航延续几何寻路、唯一焦点和返回恢复合同，平台适配先写清算法与验证边界。

### 媒体与输入按等价目标适配

保留单一 libwebrtc Peer 与现有 Metal 主线。PC 逐解码帧直通、latest-only 和尺寸变化的目标可沿 iOS mailbox、GPU 纹理与 presenter 实现；PC GLSL/Worker/rVFC 的具体执行点分别需要 Metal/native 适配。SR、锐化、风格组合先采用相同参数语义及图像样本，再检验端点、有限值、色彩、像素、GPU 时长和功耗。

输入继续沿当前 Swift 高频编码路径，接入映射后的逻辑态、UI/游戏归属、虚拟控件贡献和统一释放。共享 wire 与时序 fixture 应覆盖多控件同键、菜单组合吸收、断连/后台/退出归零，以及震动替换、delay/repeat 和停止。

TAK 使用统一 schema、资源和命令合同；PC Vue/TypeScript 与 iOS 原生控件消费同一语义样本。实际官方 bundle 的下载/解包、版本、资源、动态 Context 和代表性游戏覆盖是两端共同的前置证据。

### 以当前设备证据决定交付成熟度

| 验证维度 | iOS 当前证据接口 | 迁移后的验收要求 |
| --- | --- | --- |
| 网络健康 | RTCStats：RTT、jitter、loss、candidate pair、NACK/PLI/FIR | 协商与选中路径可复盘，统计使用同一有效窗口与单位 |
| 恢复链健康 | Peer 重连、代次、终态与恢复事件 | 健康/受损/恢复/终态形成闭环；ReferenceChain/Ledger 等内部事实按库可观测能力声明，缺项保留 unknown/unsupported |
| media supply | 入站字节、packet、framesDecoded、首帧/最近媒体时间 | 分开验证接收、解码和进入 mailbox，明确停顿发生的位置 |
| steady supply | steadyMediaObserved 与连续 RTC/display 快照 | 用持续窗口检验稳定供给和空窗，首个 steady 事件作为阶段事实 |
| submit/present 长尾 | Metal 提交、真实 presentedTime、P50/P95 | 分开测量帧到提交、GPU/上屏及首显；验证 terminal 后旧代次呈现为零 |
| 帧供给 delta | frameSupplyDelta、mailbox/drop、submit/present 计数 | 对齐接收/解码/合并丢弃/提交/呈现差分，解释 intentional local drop 与供给缺口 |

历史 Simulator 样本曾达到稳态约 30fps、submit/present 4375/4375、submit-to-present P95 1.969ms；同一样本记录了首显 27.283s 和 terminal 后 3ms 的一次晚到 present。这组证据属于当时的 Simulator 环境；当前 iPhone/iPad、xCloud/xHome 与后续生命周期修复需要 fresh trace 验收。[历史 Metal 进度](../rfcs/2026-07-24-ios-zero-copy-metal-render-pipeline.md)。

本次实际检查：

- `./iosapp/scripts/check-streaming-session-boundary.sh`：通过。
- `IPHONEOS_DEPLOYMENT_TARGET=26.0 cargo check -p xbox-ios-bridge --target aarch64-apple-ios --target-dir target/ios --offline`：通过，含当前五个共享 crates 与 iOS bridge。
- 当前主机工具链：Xcode 27.0 / iPhoneOS SDK 27.0；工程 deployment target 仍为 26.0。旧文档中 Xcode 26.1/历史 destination 阻碍应通过当前环境重新核验。
- 本次验证覆盖 Rust Device 目标类型检查与静态边界；完整 Swift/UniFFI 链接、XCTest、设备媒体与长测进入后续验收。

## 5. 待补证据与决策

- 当前 Xcode 环境下的完整 App 构建、Swift 6 strict checking、绑定一致性、XCFramework 校验与 XCTest 结果，以及现有 iOS 架构/音频/权限/Metal 待验任务的当前状态。
- 真实账户的续期轮换、公开与受限目录、空列表/错误、好友隐私、媒体过期地址、主机安装数据和跨区发现结果；缓存须继续满足账户/区域/市场/语言隔离。
- fresh Device xCloud/xHome 的连接、首显、音频中断、网络切换、后台停止/返回、重连与重复启停；按六个维度给出同窗证据。
- 目标 iPhone/iPad、60/120Hz 显示、持续游玩功耗、温度与内存预算，以及系统公开的手柄扩展键/四路触觉能力。
- 首批分发方式、签名身份与构建号策略。现有 workflow 集中在桌面构建和 beta/stable 发行，iOS 分发应形成独立合同；桌面 main 开发构建状态沿当前约定。

## 6. 候选后续任务

| 优先级 | 类型 | 候选任务 | 完成条件 |
| --- | --- | --- | --- |
| P0 | simple 审计；发现修复按影响定级 | 更新 iOS 当前构建与待验基线 | 当前 Xcode 下的 App/绑定/依赖/XCTest 证据和准确的待验清单 |
| P0 | complex | 共享数据与错误合同、iOS bridge 接线 | 区分合法空/失败，保留排序、时效、账户 scope 和最新凭据，两个宿主消费同源样本 |
| P0 | complex，沿既有 RFC 收口 | iOS 基础串流与生命周期验收 | fresh Device 两目标通过六维 gate，音频、输入释放、终态和重复启停闭环 |
| P1 | complex，按页面域拆分 | 云目录/详情、主机安装库与我的扩展 | 实际 API、分页/部分失败、缓存/返回与原生页面完成一致性验收 |
| P1 | complex | 输入归属、输出映射及最新震动语义 | 同源字节/时序 fixture、配置恢复、设备切换及实际控制器结果通过 |
| P1 | complex | 移动端精简诊断与问题历史 | 异常窗口、错误引用、重启恢复、有界导出和移动端成本证据闭环 |
| P2 | complex | Metal 画面增强与参数合同 | 共享图像样本和实际 GPU/呈现/热功耗测量通过 |
| P2 | complex | TAK 官方资源与原生触控兼容 | 真实 bundle、版本/Context/layer/多指/释放/输入响应和资源预算通过 |

候选开发任务由 `task-run` 接手形成对应 RFC 与阶段验收。推荐先完成共享合同与 iOS 基础串流待验项，再迁移目录、主机和个人能力；Metal 增强与 TAK 沿各自样本和设备证据推进。
