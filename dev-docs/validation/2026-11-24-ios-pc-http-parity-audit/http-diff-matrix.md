# iOS / PC HTTP 请求全量边界与差异矩阵

## 结论

**PC 比 iOS 稳定的观察有可解释的实现差异，但不能简单归因于 iOS 使用另一套业务 HTTP 库。** 两端业务协议和多数 HTTP provider 都走同一份 Rust reqwest；最重要的差异在平台代理发现、宿主额外 deadline、错误保真、目标凭据恢复、缓存和资源下载。

优先调查：系统代理的真实出网路径 → 首次认证逐请求诊断 → iOS 主机 8 秒截断及类型化错误 → Cloud 查询 401 恢复接线。不要先全量延长超时、无条件重试 POST，或把 Rust 协议迁移到 URLSession/TypeScript。

## 1. 审计范围与证据等级

- 审计对象为 UTC 2026-11-24 07:09:23 的 `main` 工作树快照，HEAD `b6ac29c0599b7651ff9db84395bbbb6fdc7da748`，包含既有暂存改动。以下源码结论以该快照为界；本任务不改业务代码。
- 扫描 146 个候选源文件并逐类追踪装配：15 个共享 WebAPI provider、55 个公开异步方法、12 处 Swift 平台托管网络调用。方法数包含包装、兼容/保留入口，不等于独立 URL 数或当前 iOS 实际请求次数。
- 并发提示：审计快照截止 UTC 2026-11-24 07:09:23。期间 8 个候选文件有其他任务改动，全部保留；已核对共享身份 payload/触屏阶段日志、6 处页面刷新/请求消费者变化。新增商品详情请求仍复用现有缓存；12 处平台托管调用参数与相册下载主体未变。原始基线其余 138 文件未变；工作树不锁定，不宣称后续源码静止。
- 续接边界：UTC 07:41:59 另记录 9 个来源（含生成绑定）与上述快照不同，全部保留，未纳入本轮当前代码验收。见 [快照后变更](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-pc-http-parity-audit/post-snapshot-drift.json)；不将工作树指纹不一致改记为静态 PASS，不继续追逐其他任务的后续修改。
- 覆盖认证、个人资料/好友/动态、主机查询/命令、历史/成就/时长、安装信息、云目录/商品/等待、串流 HTTP 控制、图片/相册/视频、官方触屏资源、PC 更新。
- 不把测试、样例、Data(contentsOf:)本地文件读取、reqwest URL 类型解析、前端业务 Store.fetch 方法或 WebRTC/UDP 媒体包误算成 HTTP 请求。
- 依赖特征离线对照：iOS `aarch64-apple-ios` 与 PC `aarch64-apple-darwin`。Windows 系统代理分支只核对上游源码，未执行 Windows 构建。
- **事实**：源码、固定版本依赖和静态断言可证。**影响判断**：该差异具备相应的失败/恢复机制。**现场根因**：需要配对运行包和网络 trace，本轮未证明。

相关历史：[11-08 HTTP 审计](/Users/guo.xu/Documents/code/games/xbxrc/docs/reports/2026-11-08-non-streaming-http-audit-fixes.md)、[11-23 启动对齐](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-11-23-ios-startup-pc-parity.md:83)、[本次登录前置失败诊断](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-login-instability/diagnostic-39a03964/README.md)。旧版重复全量续期不能直接当成当前实现。

## 2. 请求发起层级

| 层级 | iOS | PC |
| --- | --- | --- |
| 业务数据 | Swift Store → Rust*Client → UniFFI → Rust bridge → WebAPI/领域 flow → reqwest | Vue Store → typed RPC → Tauri Data/Auth Service/SessionResolver → WebAPI/领域 flow → reqwest |
| 初始认证 | AuthStore → start_login → AuthFlow → 设备令牌/SISU，成功后才打开系统网页登录 | AuthService → AuthFlow → 相同设备令牌/SISU，成功后才打开 OAuth WebView |
| 凭据续期 | Keychain/credential coordinator → Rust auth_session → renew_auth_tokens，checkpoint 回 Swift 保存 | Rust AuthService/持久化 owner → 相同 renew_auth_tokens，checkpoint 落盘 |
| 串流 HTTP | Rust bridge session/credential adapter → 共同 session gateway/signaling provider | Tauri streaming service → 共同 session gateway/signaling provider |
| 图片/视频 | AsyncImage、URLSession loader、AVPlayer，由平台网络栈管理 | RemoteImage/img/video，由 WebView 网络栈管理 |
| 登录网页 | ASWebAuthenticationSession；默认非临时，可配置 ephemeral | Tauri OAuth WebView；明确 incognito |
| 相册文件保存 | URLSession.shared.download → 临时导出文件/系统分享 | Rust reqwest 流式下载 → 校验 → 原子替换用户目标文件 |

入口证据：[iOS 业务 client](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/RustBridge/XboxDataClient.swift:47)、[iOS Cloud client](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/RustBridge/XboxCloudDataClient.swift:109)、[PC RPC](/Users/guo.xu/Documents/code/games/xbxrc/src/shared/rpc/client.ts:6)、[PC SessionResolver](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/session_resolver.rs:83)。PC 业务并不走 Axios/浏览器 fetch；搜索到的 fetch 是业务 Store 方法。

## 3. 共享 HTTP provider 完整清单

下列公开方法覆盖全部 15 个 provider。包装方法和保留协议入口列出，但不据此声称它们都被当前 UI 调用。URL 只描述来源域与模板，不包含用户地址、签名链接或身份值。

| Provider / 来源 | 全部公开异步方法 | 请求类别、iOS 与 PC 使用位置 |
| --- | --- | --- |
| [auth_api.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/auth_api.rs:118) | `get_device_token`、`sisu_authenticate`、`exchange_code_for_token`、`refresh_user_token`、`get_cloud_transfer_token`、`get_cloud_transfer_token_response`、`sisu_authorize`、`get_title_token`、`get_user_token`、`xsts_authorize`、`get_streaming_token` | 认证 POST：login.live.com、device.auth.xboxlive.com、sisu.xboxlive.com、title/user/xsts auth、offering GSSV。两端初始/目标续期共用；title/user authenticate 和 transfer 包装不等于独立活跃流程。 |
| [profile_api.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/profile_api.rs:55) | `get_current_user`、`get_user` | profile.xboxlive.com 资料 GET。iOS profile section/player profile；PC ProfileService/PersonalService。 |
| [social_api.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/social_api.rs:25) | `get_summary` | 社交计数 GET。iOS 当前资料 social 来源使用；PC Tauri 当前生产代码未发现 SocialApi 直接装配，不能因此说 PC 没有其他社交资料。 |
| [people_api.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/people_api.rs:19) | `get_friends_count`、`get_friends` | PeopleHub 好友 GET。两端好友/本人资料，计数与完整列表装配不同。 |
| [userpresence_api.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/userpresence_api.rs:25) | `get_current_user` | userpresence.xboxlive.com 状态 GET。两端资料页可选来源。 |
| [titlehub_api.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/titlehub_api.rs:31) | `get_title_history`、`get_title_history_page`、`get_title_detail` | TitleHub 游戏历史/单游戏 GET。iOS 新分页接口与保留全量入口；PC PersonalService。 |
| [achievements_api.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/achievements_api.rs:40) | `get_title_achievements` | achievements.xboxlive.com 成就分页 GET。两端新版分页使用共同解析；iOS 另有保留的全量聚合导出。 |
| [userstats_api.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/userstats_api.rs:34) | `get_minutes_played`、`get_minutes_played_batch` | userstats.xboxlive.com 只读 POST /batch。两端当前时长查询都是最多 100 标题、每 HTTP 20 标题、最多四路并发、部分失败保留；不能概括成 PC 串行/iOS 无界并发。 |
| [media_api.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/media_api.rs:17) | `search` | 相册搜索/分页只读 POST。两端返回缩略图与签名资源链接，资源后续下载不是这个 POST。 |
| [smartglass_api.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/smartglass_api.rs:19) | `get_installed_apps`、`get_storage_devices`、`get_console_status`、`get_consoles_list` | xccs.xboxlive.com 主机/安装/存储/状态 GET。HTTP 合同与远程管理响应检查共同；宿主 deadline 和最终解析不同。 |
| [console_api.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/console_api.rs:66) | `send_command`、`power_on`、`power_off`、`send_text` | 主机写命令 POST。普通页 power 与串流文本/关机使用不同宿主 owner；共享命令传输不自动重试。 |
| [gamepass_api.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/gamepass_api.rs:24) | `get_sigl`、`get_products`、`get_products_with_hydration` | catalog.gamepass.com 集合 GET、商品只读 POST。目录、详情与安装封面；共同 hydration 75 商品/批、四路并发、默认整轮 30 秒。 |
| [xcloud_api.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/xcloud_api.rs:44) | `get_titles_by_ids`、`input_configs`、`touch_adaptation_bundle`、`get_waiting_time`、`get_titles`、`get_recent_titles` | titles-by-ids/input-configs 为只读 POST；titles/recent/wait 为 GET；触屏包先 GET 下载地址，再 GET 签名文件。两端云目录/目标/等待；官方触屏保存下载为 iOS 新能力，PC 当前没有等价保存工作流。 |
| [session_api.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/session_api.rs:89) | `start_stream_with_payload`、`stop_stream`、`get_stream_state`、`get_configuration`、`send_connect_token`、`send_keepalive`、`get_waiting_times`、`get_consoles`、`input_configs` | GSSV /v5 sessions 等串流控制 POST/DELETE/GET，两端共同 gateway/plan。不是 WebRTC 媒体包。 |
| [signaling_api.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/signaling_api.rs:42) | `send_sdp`、`send_chat_sdp`、`get_sdp_exchange_response`、`get_ice_exchange_response`、`send_ice` | SDP/ICE HTTP POST/GET，两端共同信令协议；PC Browser 与 iOS native 媒体运行时不同不代表各自重写 HTTP。 |

装配索引：[iOS profile sources](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-ios-bridge/src/profile_queries.rs:18)、[iOS personal queries](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-ios-bridge/src/personal_queries.rs:46)、[iOS installed sections](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-ios-bridge/src/installed_queries.rs:29)、[iOS catalog](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-ios-bridge/src/cloud_catalog.rs:60)、[iOS official touch](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-ios-bridge/src/touch_layouts/mod.rs:139)、[PC personal sources](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/services/personal_service.rs:58)、[PC catalog](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/services/xcloud_service.rs:77)。

### Swift 平台托管入口与 PC 对应路径

| iOS 发起入口 | 个数 | PC 对应 | 关键不同 |
| --- | --- | --- | --- |
| AsyncImage：游戏截图、个人游戏图片、头像、安装封面、相册图片 | 8 | RemoteImage/img | iOS 多处直接使用框架 phase，不统一经过自有缓存/类型化错误入口；PC RemoteImage 共用候选切换、懒加载、激活恢复，但同样不提供原始 HTTP 状态。 |
| [URLSession 图片 loader](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Shared/Components/CircularCardCarousel.swift:124) | 1 | [RemoteImage](/Users/guo.xu/Documents/code/games/xbxrc/src/components/common/RemoteImage.vue:16) | iOS 自有 NSCache 256/64MiB、URLCache、同 URL in-flight 合并；网络诊断为 debug。不能说 iOS 没有图片缓存，也不能说所有 AsyncImage 都走这份 loader。 |
| [URLSession 相册下载](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Features/Profile/XboxAlbumView.swift:322) | 1 | [Rust media_save](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/services/media_save.rs:42) | iOS 无显式下载预算/大小与文件签名验证，非 2xx 丢失状态；PC 显式 5/300 秒、2GiB、MIME/签名校验、临时 guard、原子替换。两者都不能被说成自动无限重试。 |
| [AVPlayer 网络视频](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Features/Profile/XboxAlbumView.swift:370) | 1 | WebView video | 平台管理 Range/播放；iOS 自有失败回调、后台暂停。不能把这里的网络错误与 Rust 业务请求视为同出口。 |
| [系统网页登录](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Features/Authentication/WebAuthenticationPresenter.swift:37) | 1 | [OAuth WebView](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/auth/rpc.rs:114) | iOS ephemeral 默认 false；PC incognito true。发生在设备令牌/SISU 成功后，不能解释本轮 beginLogin 前置失败。 |
| iOS 无自有应用 updater 请求 | — | [Tauri updater](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/updater/service.rs:88) | PC 检查 20 秒、下载 300 秒、签名校验与 generation/lease；这是 PC 独有安装更新流程，不是 iOS 登录差异。 |

AsyncImage 位置：[游戏详情](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Features/Library/GameDetailView.swift)、[个人游戏](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Features/Profile/PersonalGameViews.swift)、[个人页面](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Features/Profile/PersonalPages.swift)、[安装游戏](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Features/Hosts/HostInstalledGamesView.swift)、[相册](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Features/Profile/XboxAlbumView.swift)。12 是调用点数，不是 URL/并发连接数。

截止快照的消费者补充：个人游戏详情同时加载 TitleHub详情、时长与 Game Pass 商品详情；安装游戏详情新增商品详情读取/强刷。它们调用现有 loadProductDetail，因此适用 600 秒内存缓存及请求合并。[刷新包装](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Shared/Components/AppInformationPage.swift:536) 用原生 refreshable + isRefreshing guard 执行一次 action，分区按钮受 pull-refresh 状态互斥；不额外设置 HTTP timeout 或传输重试。

## 4. 共同策略：不是 iOS 独有原因

| 项目 | 当前共同实现与边界 |
| --- | --- |
| 初始 OAuth | PC 与 iOS 都用 AuthFlow::new/start_oauth_login，每次建立新的 AuthApi/client。设备令牌 → SISU → 网页登录，前两项协议与参数共同。上轮指出新建 client 是放大因素，**不是 iOS/PC 的区别**。 |
| 普通 HTTP client | 业务 provider 多用 for_queries，认证/写命令用 for_commands；没有发现生产 custom proxy、自定义 DNS、强制 HTTP/1 或跳过 TLS 验证装配。测试 no_proxy 不算生产设置。 |
| 业务预算 | 5 秒建连、12 秒单次、18 秒单个请求整体预算。GET/显式只读 POST 最多三次应用层尝试，250ms 起退避、Retry-After 与响应大小门禁共同。 |
| 认证/写命令 | 应用层 RetryMode::Never 是共同安全边界；retriable=true 不等于已重试。也不要把应用层策略与 reqwest/hyper 的协议级恢复混为一谈。 |
| 串流 HTTP | 共同 gateway 默认 HttpTransport::new：12 秒单次，SafeOnly；不自动套业务查询的 5 秒建连/18 秒总预算。整链还有共享 session 阶段 deadline。不能把它称为 iOS 的另一套配置。 |
| 核心 token | 两端均调用 renew_auth_tokens，复用有效目标/Web/SISU，明确拒绝时恢复，核心轮换 checkpoint 先落盘。当前 iOS 已不再简单丢弃所有缓存重走全链。 |
| HTTP/2、TLS | 两目标均解析出 Rustls、HTTP/2、system-proxy、平台证书 verifier。PC 额外 stream/rustls-no-provider 不证明业务请求更“原生”。 |
| 请求 headers/body | 认证协议共用；串流由共同 compiler 生成 User-Agent/x-ms-device-info/X-Forwarded-For。设备画像受配置/分辨率影响，并非简单按物理 iOS/PC OS 生成；同代码不代表不同用户设置下每个字段完全一样。 |
| 批次与部分失败 | 当前时长两端都是 20 标题/HTTP、最多四路；目录 hydration 共享 75/四路/30秒。现代接口保留部分成功，保留的旧全量导出不能当成当前主页面入口。 |

证据：[PC 初始认证](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/auth/service.rs:62)、[iOS 初始认证](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-ios-bridge/src/lib.rs:131)、[公共 transport](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/transport.rs:109)、[共同 session gateway](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/src/session/api/session.rs:19)、[请求画像 compiler](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/src/policy/session/compiler.rs:42)、[iOS 续期](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-ios-bridge/src/auth_session.rs:321)、[PC 续期](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/auth/token_refresh.rs:58)。

## 5. 真正的差异与影响范围

| 差异 | iOS | PC | 稳定性影响；能否解释本次登录前失败 |
| --- | --- | --- | --- |
| F1 系统代理发现 | 当前依赖无 iOS 系统配置分支；默认读 env，未见 app 显式注入 Wi-Fi proxy | macOS 从系统配置补充 HTTP/HTTPS proxy，Windows 也有系统分支 | **最值得先验证的出口差异**。若 PC 实际使用系统代理而 iOS Rust 直连，可影响全部 Rust HTTP，包括初始认证；现场路径尚未验证。 |
| F2 主机外层 deadline | fetch_hosts 强制 8 秒，提前终止共同 HTTP 预算 | HostService + HostCache 查询没有这层 8 秒截断 | 可截断本来能在 18 秒内恢复的慢请求；**只影响主机查询，不是 beginLogin 的 6 秒限时**。 |
| F3 错误保真 | Query 仅 kind/status/retriable/retryAfter，非 timeout 网络种类压成 network；部分 Data 字符串归 unknown | DataDetailed/安全 summary 保留 networkErrorKind 和安全 serviceCode；也仍有 Message/下载 String 路径 | 直接妨碍本次 DNS/connect/TLS 定位；不是网络失败本身原因。PC 也不是所有出口都完美类型化。 |
| F4 Cloud 目标 401 恢复 | loadWaitTime 只在 lease 缺失/到期时 prepare，查询失败后 authentication 直接提示重登；其他 helper 重新 prepare 但没提交被拒 gsToken | run_scoped_query(Cloud) 明确将被拒目标 token 交给 owner，恢复后重试一次 | 可解释已登录后的偶发云等待/资源查询失败或过早要求重登，**不能解释尚未打开网页的 beginLogin**。 |
| F5 商品详情缓存 | 内存状态、600 秒；scope 清理后丢失；首轮失败 phase=failed 默认激活不自动重试 | 后端内存+磁盘，24 小时新鲜、失败时最多 7 天 stale，30 槽/两路 | PC 更容易“网络不好仍显示内容”；iOS 冷恢复更依赖网络。属于资料页稳定性/请求暴露差异。 |
| F6 主机激活/失败恢复 | activateOnce 在请求前消费激活标记，返回页面不会自动再次初刷；手动刷新可重试 | backend snapshot/in-flight 合并；frontend 失败后 15 秒冷却，可再次 fetch/强刷 | iOS 初次失败更容易保持失败态；PC 并非无限即时重试。两端都保留已成功的旧内容，不能说 iOS 完全无缓存。 |
| F7 取消生命周期 | 通用 UniFFI helper 无 Swift task cancellation handler；cancel 主要由上层检查/丢弃结果，Rust HTTP 可能继续到终态 | 普通 RPC 也不等于可取消 HTTP；Cloud wait 另外提供 requestId + Notify/select 显式取消 | 需要按入口治理在途请求与 busy 尾部；**不能声称 Swift 切页必定取消网络，也不能把它当本次死锁证明**。 |
| F8 相册下载 | 仅 item.isExpired 时主动续址；非 2xx 变 unavailable；OS 默认 download，无显式大小/签名验证 | 保存窗口前后重新取签名地址；5/300 秒、2GiB/格式校验/原子替换；错误日志有安全分类 | iOS 更难分辨签名过期、429、5xx 与错误资源，恢复与失败诊断不齐；属于下载路径，不是初始认证。 |
| F9 主机结果容器 | 找不到可识别 host 容器时 unwrap_or_default 成空列表 | 无有效容器时 parse error | 前置 HTTP/远程管理检查共同，但宿主解析层仍可能把异常响应当空主机成功；尚无本轮异常 JSON 实机样本。 |
| F10 图片路径一致性 | 自有 loader 与多处 AsyncImage/AVPlayer 并存，失败和缓存策略不完全统一 | RemoteImage 共用候选/激活恢复，但媒体视频仍由框架管理 | 可解释资源表现差异；图片成功不证明 Rust 业务 HTTP 可达，不能以此排除 F1。 |

### F1：代理 feature 相同不等于出口相同

已经下载并按 Cargo.lock 校验 reqwest 0.13.2、hyper-util 0.1.20。默认 ProxyMatcher::system 最终进入 Builder::from_system；该函数仅对 macOS/Windows追加系统配置，iOS 保留环境变量回退。详见 [固定版本上游证据](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-pc-http-parity-audit/upstream-evidence.md)。

准确含义：**不会自动读取 iOS Wi-Fi HTTP 代理，不是 iOS 不支持任何代理/VPN。** TUN 在更低层路由，可能覆盖 Rust socket；PC 关闭 TUN 也可能仍使用系统/环境 HTTP 代理。只有确认这些开关和同网条件，才能说“两端都裸连”。本轮未读取或输出任何实际代理地址。

### F2/F3：主机预算和类型化错误存在确定落差

[iOS fetch_hosts](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-ios-bridge/src/data.rs:100) 套 8 秒 timeout；超时返回 Data("主机查询超时，请重试")。[PC query_hosts](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/service.rs:56) 通过 resolver/cache/HostService 使用共同底层预算，没有这层截断。

按配置推导：两次 5 秒建连等待加首次 250ms 应用退避需要 10.25 秒，已经超过 iOS 外层 8 秒。该推导是预算关系，不是已跑过的网络复现。

[Swift 错误投影](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Shared/Models/SectionRequestFailure.swift:30) 不识别 Data 为 timeout，默认 unknown；[fallback trace 分类](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Shared/State/CloudLibraryDiagnostics.swift:63) 也不把中文“超时”识别为 timeout。Query 正常类型化路径没有这个问题，但 [Query 记录](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-ios-bridge/src/query_failure.rs:18) 仍丢弃网络细分与服务码；[PC DataError](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/error.rs:48) 保存 safe_summary。

HTTP Retry-After 的传输层处理是共同的；iOS Query DTO 的 retryAfterSeconds 当前构造为 None，PC safe_summary 同样未提供完整 Retry-After UI合同，不能把它泛化为仅 iOS 缺少传输层限流恢复。

### F4：核心续期一致，不代表每个 consumer 都接齐恢复

普通 iOS资料/分页查询已经有 [一次 authentication 恢复](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Shared/State/XboxDataStore.swift:1057)，[AuthStore](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Features/Authentication/AuthStore.swift:399) 合并并发 Web 续期，网络失败保留凭据。因此不能说 iOS 所有 401 都要求重登。

但 [iOS loadWaitTime](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Shared/State/CloudLibraryStore.swift:1265) 没走同样的目标拒绝恢复；[PC Cloud wait](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/cloud_queries.rs:404) 经 Cloud scoped resolver，其 [401 分支](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/session_resolver.rs:107) 明确传入 rejected token。

[iOS withCloudAccess](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Shared/State/CloudLibraryStore.swift:1225) 有一次重新 prepare 的重试，但没有明确被拒目标 token 参数；不能认定它与 PC forced target rejection 等价，也不能将保留有效期的旧 token 当作恢复已完成。串流 live credential adapter 已有目标恢复，本条针对普通 Cloud 查询消费者。

### F5/F6：PC 的“更稳”还包含缓存遮蔽与失败态恢复

[iOS 商品详情](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Shared/State/CloudLibraryStore.swift:1185) 600 秒内存新鲜；[PC 详情](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/cloud_queries.rs:56) 24 小时缓存与 7 天失败回退。后者确实减少网络故障直接暴露给页面的概率，但不是网络连接更健康的证明。

[iOS activateOnce](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Shared/State/XboxDataStore.swift:949) 请求前即消费首次激活；[PC HostsStore](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/home/hosts-store.ts:49) 失败冷却 15 秒后允许再次 fetch，后端还有 [缓存与 in-flight 合并](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/host_cache.rs:53)。这是恢复节奏差异，是否改变应以目标交互合同确认，不应无条件增加后台重试。

### F7/F8：不要误读取消与下载保证

[通用 UniFFI async helper](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/RustBridge/Generated/xbox_ios_bridge.swift:7461) 在 Rust future 完成后 free，没有 Swift取消传播。[PC Cloud wait](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/cloud_queries.rs:436) 有显式 cancel signal。生成文件仅作为审计证据；后续若修复，必须沿现有绑定生成/补丁流程实现，不手工维持一套平行 bridge。

下载保证对照见 [iOS save](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Features/Profile/XboxAlbumView.swift:322) 与 [PC save](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/services/media_save.rs:82)。iOS 有手动续址/取消后清理，PC 有更严格的续址时机/校验/原子提交；不是 iOS 完全没处理错误，也不是 PC自动无限恢复。

## 6. 对当前登录失败能说到哪里

前轮 7 次均失败在 beginLogin，5 次 149–398ms network，2 次 6.121/6.722 秒 timeout。没有进入系统网页登录或账号密码提交阶段。

- F1 可能直接影响此阶段；但要先证实 PC/iOS 各自真正出网路径。
- F3 已直接限制定位：错误只剩 network/timeout，没有哪项请求、DNS/connect/TLS 与服务码。
- F2、F4至F10的影响范围不在初始 beginLogin；它们能解释“整个 iOS 使用过程更不稳定”的部分体验，但不能替代本次登录的因果证据。
- 5 秒建连/Never/每次新建 client 是共同默认，不是 iOS 独有根因。只把 timeout 拉长不能解释五次不到 0.4 秒的失败。
- 不能从 statusCode 缺失断言请求未发送或 TCP/TLS 未建立；成功响应体读取失败也可能没有最终状态投影。
- 两端的实际 DNS/IPv6、信任配置、网络迁移与手机包来源仍未配对验证。前六次登录在 active 中失败，不支持先归因为后台挂起。

## 7. 后续优先级（未实施）

| 优先级 | 最小下一步 | 验证目标 |
| --- | --- | --- |
| P0 | 首次认证补逐请求 requestStage、networkErrorKind、耗时、状态/安全服务码、parent operationId；记录安装包/SDK来源标识和脱敏出口模式 | 在同一手机/同网条件下区分 deviceToken/SISU 与 DNS/connect/TLS/body；不能记录 token、seed、JWK、完整 URL/代理地址或身份。 |
| P0 | 确认 PC 系统/环境代理、TUN 与 iOS Wi-Fi proxy/TUN 的真实组合；比较同一阶段而非 PC 热缓存对 iOS 冷登录 | 判断 F1 是否真是现场差异；桌面 curl/Safari 成功不能替代 iOS Rust 请求。 |
| P1 | 对齐 iOS 主机外层预算和类型化 timeout；补齐 Query 网络细分/安全服务码，消除 Data字符串降级 | 慢请求可在共同预算内完成恢复；UI/trace 对超时、网络、认证、取消、限流的判断一致。 |
| P1 | 普通 Cloud 等待/目录/官方资源查询接入正确 target owner 与 rejected-token 单次恢复 | Cloud 401 不被误当 Web登录失效；验证成功、二次拒绝、临时失败、取消/换账户与凭据保留。 |
| P2 | 按 PC 已验证方向整理详情持久缓存/stale、首次失败激活、跨页面 in-flight 与显式取消 | 统计真实请求次数、缓存命中、失败后恢复、取消后仍在途数量；不引入平行状态或 HTTP路线。 |
| P2 | 相册下载补齐续址时机、HTTP分类、下载预算/大小与格式/清理保证；统一资源入口的必要诊断 | 403过期/429/5xx/timeout/空文件/取消可区分且旧导出保留正确。 |

不建议现在做：所有 POST Always、无限自动重登、绕过证书校验、按猜测强制 HTTP/1/禁 IPv6、搬协议到 Swift、增加第二套 native transport。任何业务修复需用户另行确认，先落清晰范围和运行验证目标。

## 8. 验证与剩余边界

- 原快照已完成入口扫描/装配回读、15 provider/55 方法覆盖、12 Swift 托管调用计数、18 条源码断言与146候选文件哈希复核。18条断言为前轮执行记录，续接未重跑，不宣称覆盖快照后源码。
- 续接重新核对方法清单、Swift调用点、文件链接、两份发布包与三份上游源码checksum，均通过。两目标 `cargo tree --offline --locked` 的feature/normal树共4条命令成功，未构建或执行下载包代码。
- [续接验收原始记录](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-pc-http-parity-audit/closure-verification.json) 为13项PASS/3项FAIL；三项失败均为“当前来源仍与旧快照一致”的检查，不删去失败，也不把当前工作树当作已通过全量静态验收。
- [来源指纹](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-pc-http-parity-audit/source-hashes.json) 保留146个候选快照文件、原8处并发变化与补充关键来源；[验证摘要](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-pc-http-parity-audit/validation.json) 区分原快照结果和续接范围。静态断言不是认证网络试验或登录功能验收。
- 未改业务代码、未编译/运行测试、未读取实际代理/用户凭据、未做手机/PC认证网络对照、未验证运行包与源码commit一致性。
- [清理摘要](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-pc-http-parity-audit/cleanup.json)：确认无任务进程/文件占用后，删除旧会话r1与当前会话r2两个独立临时目录，共25文件、约0.95MiB已分配文件块；目录已不存在。原用户日志、共享缓存与其他任务产物未动。
