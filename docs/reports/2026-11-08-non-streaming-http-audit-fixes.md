# 非串流 HTTP 接口全量审计与修复 Report

## Summary

- Related RFC: [非串流 HTTP 接口全量审计与修复 RFC](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-11-08-non-streaming-http-audit-fixes.md)
- 日期：2026-11-08；主线：`main`；执行：Codex 主窗口，单窗口。
- 已完成认证、13 个非串流 WebAPI provider、数据编排、云目录、主机管理、相册下载、应用更新与 Vue 远程资源使用的源码审计和适用修复。
- 过期 Web Token 回退进入认证恢复；业务查询遇到 401 后最多恢复并重试一次。网络、权限与协议错误保留分类，凭据按明确认证拒绝处理。
- 传输沿用 `reqwest` 的 macOS 系统代理 / 直连能力。前次[网络诊断](/Users/guo.xu/Documents/code/games/xbxrc/docs/reports/2026-11-08-http-direct-proxy-diagnosis.md)的 22 项 curl 和 6 项当前客户端对照全部取得 HTTP 响应；历史 TUN 切换差异仍需固定有效凭据、地区与账户后实机对照。
- 当前用户日志的主机 6 次 parse 失败已定位为 XCCS `OK` 成功码被误判；主机管理与云游戏准入分别使用独立合同和错误分类。
- 使用当前有效凭据运行实际 Rust 服务，主机 / 资料 / 好友 / 历史 / 成就 / 安装游戏 / 存储 / 相册成功；186 款游戏时长返回 160 个数值、26 个缺失、0 个失败。
- 最终有效验证覆盖 318 项 Rust 测试与 681 项前端测试，格式、workspace 编译、前端类型和构建均通过；实际组件的隔离预览完成子页面居中和状态验证。

## Delivered

1. 有效期检查、401 恢复、并发去重、取消释放及账户切换保护组成统一查询认证路径。
2. 查询 / 只读 POST / 认证交换 / 设备写命令具有明确的请求预算与重试规则。
3. 各 API 的 URL、headers、body、分页与响应容器逐项复核；已确认的合同缺口得到修复。
4. 云目录缓存和结果按账户、区域、语言、market 隔离，后台刷新与页面结果均有迟到结果保护。
5. 下载和更新补齐超时、失败重试、取消清理与完成条件；诊断输出采用固定分类和安全服务码。
6. 当前日志驱动修复补齐 XCCS OK / 浮点容量 / 嵌入存储、PeopleHub 版本 7、stats-only 时长批量、第一方 HTTPS 图片、局部诊断和主机 / 云游戏共用错误页。

## Changes

### HTTP 覆盖清单

表中身份、地区、商品和分页参数采用占位表示。验证列区分源码 / 指南合同检查、本地故障注入、状态回归与实机边界。

| 类别 / 源码 | 方法与端点 | 核对与修复 | 验证 |
| --- | --- | --- | --- |
| [认证 API](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/auth_api.rs) | POST `login.live.com/oauth20_token.srf`；`device.auth.xboxlive.com/device/authenticate`；`sisu.xboxlive.com/authenticate`、`/authorize`；`xsts.auth.xboxlive.com/xsts/authorize`；保留的 title / user authenticate；`{offering}.gssv-play-prod.xboxlive.com/v2/login/user` | OAuth 表单、PKCE、签名、RelyingParty、合同 headers 复核；交换单次提交；实际 `NotAfter` 检查；云授权保留 HTTP 状态与安全具体原因 | 请求合同检查、auth flow / Token 策略 / 恢复状态回归；有效账号登录与云准入由实机补充 |
| [个人资料](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/profile_api.rs) | GET `profile.xboxlive.com/users/{me或xuid}/profile/settings` | XSTS `XBL3.0` 凭据、数字身份与 settings 合同复核；异常响应提示；资料缓存按账户隔离 | provider 合同、401 resolver 回归、账户缓存回归；前次过期凭据直连 / 代理均 401 |
| [好友](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/people_api.rs) | GET `peoplehub.xboxlive.com/users/me/people/social/decoration/...` | 补齐版本 7 和 Accept-Language；XSTS、decoration、people 容器和查询预算复核；局部失败保留个人资料和安全提示 | 实际 400 / 200 对照、1 位好友、请求合同和前端账号 epoch 回归 |
| [社交摘要](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/social_api.rs) | GET `social.xboxlive.com/users/me/summary` | 补齐历史指南合同中的计数字段；采用查询预算 | DTO 回归、workspace 编译；当前由 iOS bridge 复用 |
| [在线状态](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/userpresence_api.rs) | GET `userpresence.xboxlive.com/users/me?level=all` | 跨设备优先选择 Active 游戏；保留合法空状态 | 多设备 Presence 解析回归 |
| [成就](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/achievements_api.rs) | GET `achievements.xboxlive.com/users/xuid({id})/achievements` | titleId、maxItems、语言、合同版本复核；分页正文 Token 与响应头兼容；统一有效凭据和 401 恢复 | URL / headers、分页及 resolver 回归 |
| [游戏历史 / 详情](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/titlehub_api.rs) | GET `titlehub.xboxlive.com/users/xuid({id})/titles/titlehistory/decoration/...`、`/titles/titleid({id})/decoration/...` | 数字身份、decoration、分页参数复核；响应头分页回退 | URL、解析、分页与前端列表回归 |
| [游戏时长](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/userstats_api.rs) | POST `userstats.xboxlive.com/batch` | 批量仅请求 MinutesPlayed，20 标题 / HTTP、四路并发；按十进制 titleid 匹配；支持整数数字字符串，缺失与失败分别保留；校验响应容器、只读 POST 有限重试、401 恢复 | Hero + stats 在 15 / 20 标题时为 400；stats-only 20 / 50 标题为 200；实际服务全量 186 标题及 body / 解析 / 错误回归 |
| [云游戏目录](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/xcloud_api.rs) | GET `https://{region-origin}/v2/titles`、`/v2/titles/mru?mr={count}` | region origin 统一 HTTPS，校验 userinfo / 路径 / query；必需 `results` 容器校验；recent 可选失败保留诊断 | URL origin、必需响应、来源失败及缓存 scope 回归 |
| [Game Pass](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/gamepass_api.rs) | GET `catalog.gamepass.com/sigls/v2`；只读 POST `/v3/products` | market / language / hydration trim 与校验，`MS-CV: 1.0` 对齐指南；Products 容器校验；75 商品 / 批，四路并发，默认整轮 30 秒预算 | URL / headers / body、非法容器、批量去重、并发上限、超时保留完成批次及取消回归 |
| [主机管理查询](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/smartglass_api.rs) | GET `xccs.xboxlive.com/lists/devices`、`/lists/installedApps?deviceId={id}`、`/lists/storageDevices?deviceId={id}`、`/consoles/{id}` | 接受 OK / Success / 0；容量支持整数形式浮点；独立存储查询为空时按设备身份读取列表嵌入摘要；设备编码、列表 / 服务状态、封面 8 秒预算复核 | 实际主机 1、安装游戏 18、存储 1；成功状态、容量、身份隔离、空列表补查 / 401 恢复回归 |
| [主机命令](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/console_api.rs) | POST `xccs.xboxlive.com/commands`：开机、关机、文本 | 单次提交；服务状态检查；保留根 / `data` 中 `accepted:false`，拒绝非法显式类型；兼容成功空响应 | 命令响应回归、传输提交次数检查；实际设备写操作由用户验收 |
| [相册查询](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/media_api.rs) | 只读 POST `mediahub.xboxlive.com/screenshots/search`、`/gameclips/search` | OwnerXuid 查询与合同版本复核；续页保留 body；Token 编码及响应头分页兼容；有效凭据恢复 | 请求 / 分页合同、媒体解析和 resolver 回归 |
| [媒体保存](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/services/media_save.rs) | GET 相册返回的 HTTPS 签名媒体地址 | 保存路径选定后重新恢复凭据并获取地址；连接 5 秒 / 下载 300 秒；流式 2 GiB 上限；MIME、空文件、图片 / MP4 签名校验；临时文件 guard 与原子替换 | 本地 HTTP 成功、过期 / 缺失、429 / 5xx / timeout、非法文件、目标保留与取消清理回归 |
| [应用更新](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/updater/service.rs) | GET GitHub stable `releases/latest/download/latest.json` / beta `releases/download/beta/latest.json`，以及清单资产地址 | 检查 20 秒 / 下载 300 秒；保留身份、渠道、版本、来源与资产 URL 校验；插件校验签名后安装；失败 / 取消保留候选供重试，generation 阻止旧检查和旧渠道安装；成功才发布 Finished | manifest / 身份策略与下载 lease 回归、workspace 编译；实际下载安装由用户验收 |
| [Vue 远程资源](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/library/AlbumPanel.vue) | WebView GET 头像、封面、成就图片、相册缩略图 / 图片 / 视频 | 第一方 HTTP 图片规范化为 HTTPS；RemoteImage 共用加载 / 失败占位 / 换源重置；业务数据走 Rust typed RPC；签名续址、账号 epoch、云目录 scope / generation / 时间保护复核 | 实际封面 / 相册资源 206；失败图片预览；全量前端回归、类型与构建 |

### 请求与错误策略

- [HTTP transport](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/transport.rs) 的非串流客户端：连接 5 秒、单次 12 秒、整轮 18 秒，最多 3 次尝试；JSON 16 MiB、错误正文 64 KiB 上限。
- GET 和明确标记的只读 POST 使用有限重试。认证交换和设备命令单次提交；429 / 可恢复服务故障支持 `Retry-After` 秒数与 HTTP 日期，服务端退避超出预算时保留原始分类。
- 空响应按 API 合同处理；查询 JSON 的空正文、非法 JSON、超限正文与正文中断得到明确分类。收到 401 / 403 后，即使读取错误正文失败，已收到的 HTTP 状态仍保留。
- [错误类型](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/error.rs) 区分 DNS、连接、TLS、timeout、body 和请求初始化；服务码使用白名单。底层 source 链只在内存中参与分类。
- 数据 HTTP 结果在 production / dev trace 中保留 API 类别、耗时、结果、HTTP 状态及安全错误分类。Token、UHS、XUID、完整用户 URL、签名媒体 URL 与原始响应采用隐私过滤，相关回归通过。
- 个人数据来源增加 20 秒外层预算及 `httpSourceFinished`，局部失败写入 `httpPartialQueryFailed`，解析数量写入 `httpResponseParsed`；缺失容器、主机状态和统计 schema 使用固定 `protocolErrorCode`。Data RPC 保留安全用户消息。

### 认证、并发与数据完整性

- [Token 策略](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/auth/token_policy.rs) 检查真实 `NotAfter`，预留 60 秒余量；SISU 复用遵循凭据自身有效期。
- [认证服务](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/auth/service.rs) 合并并发检查，lease 在取消时释放，generation 保护注销 / 换账户后的迟到提交。401 核心刷新遭网络中断后，下一轮继续强制核心刷新，成功提交才清除拒绝标记。
- [数据 resolver](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/session_resolver.rs) 保留 web-only 身份与有效期；401 仅恢复并重试一次，第二次拒绝结束查询。认证恢复的网络错误向数据层传播；迟到结果检查账户与登录状态。
- 部分查询的 401 进入统一恢复；其他来源失败保留已取得数据与安全提示。Profile 缓存按账户隔离，云 overlay 按账户 / 区域 / 语言 / market 隔离。
- [云目录 flow](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-cloud-catalog-flow/src/lib.rs) 并发查询 recent / newest，必需目录与商品容器有校验；可选失败保留具体来源原因。商品补全在预算结束时保留已完成批次并取消其余请求。
- 云目录刷新 lease 取消释放等待者，后台写入检查账户、区域与 locale；[页面请求状态](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/xcloud-catalog-state.ts) 拒绝旧账户、旧 scope 与较早时间结果。[RPC / 事件 DTO](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/types.rs) 在缺时间时省略 `updatedAt`，匹配 TS 可选数字合同。
- [时长页面查询](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/library/use-playtimes.ts) 合并重复激活，只补查新增标题，保存真实 null，失败项允许重试；离页、换玩家和换账户丢弃迟到结果。加载中显示“查询中…”，查询结束区分时长、零值和缺失。

### 当前日志与真实接口复核

用户 trace：`/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1794105768062-1.jsonl`，548 行，2026-11-08 10:42:48–10:46:57，schema v3 / dev。6 次主机失败均为 parse；好友局部失败和缺失时长缺少结构化来源记录，重复时长查询各耗时约 17 秒。

[只读复核示例](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/examples/http_contract_probe.rs) 调用实际 HostService / PersonalService，并在进程内独立交换 GSSV XSTS；凭据保留在进程内，输出仅包含数量、耗时与安全错误分类。应用、设置和真实设备状态由用户实机操作。

| 实际查询 | 最终结果 |
| --- | --- |
| 主机 / 账户 | 1 台主机、1 个有容量的磁盘摘要；资料可用、1 位好友、notices 为空 |
| 历史 / 成就 | 186 款游戏、179 张 HTTPS 封面、汇总 Gamerscore 7010；抽样三标题的成就列表分别为 46 / 0 / 0，空列表按服务事实保留 |
| 全量时长 | 100 + 86 标题，160 有值、26 缺失、0 失败；总耗时 4.506 秒 |
| 主机安装信息 | 18 款游戏、18 张封面、1 个存储设备、主机状态可用、notices 为空 |
| 相册 / 图片 | 8 张截图、1 个录像，原始资源为 HTTPS；截图、录像和历史封面 Range 请求均取得 206 |
| 云准入 | GSSV XSTS 重新交换成功；xgpuweb / xgpuwebf2p 均为服务端 403，响应未提供具体安全服务码 |

独立存储查询实际返回 `result: []`，设备列表同时提供 `storageDevices` 和浮点容量。按请求主机的 id / deviceId / serverId 精确匹配后读取嵌入摘要；额外请求仅用于已确认的空存储列表，服务拒绝保留原始分类，401 进入统一恢复。

最终脱敏 trace：`/var/folders/py/9rf6bdds68140y9lkrhkz__40000gn/T/xbxrc-http-contract-probe/runtime-trace-1794112416249-1.jsonl`。data 域 44 个 key / core 事件，其中 24 个 `httpSourceFinished`、20 个 `httpResponseParsed`；来源均成功，seq 24 / 35 对应 100 / 86 标题解析结果，失败计数均为 0。

### 页面验证

本地隔离预览直接导入当前仓库 Vue 页面、AppShell、样式及几何导航，RPC 使用模拟响应；真实 HTTP 结论来自上表的 Rust 服务复核。

- 主机和云游戏共用 [PageUnavailable](/Users/guo.xu/Documents/code/games/xbxrc/src/components/common/PageUnavailable.vue)，标题 / 错误正文 / 绿色刷新按钮作为整体，在子页面正文区域居中。1280×800 实测状态区域的水平中心偏差为 0、垂直偏差为 2px。
- 首次加载后，刷新保留原错误状态和可聚焦入口，busy 时合并重复确认；鼠标 / Enter 刷新完成后 `data-focused=true`，导航机制沿用现有几何引擎。
- 好友 null 显示“好友数据暂缺”并保留个人资料；有效空列表和成功好友列表分别处理。统计 20 秒延迟时显示“查询中…”，结果分别显示 79 分钟、0 分钟和“时长暂缺”。
- 404 图片及无图片分别显示占位；有效图片正常显示。主机安装列表为空时，错误、notices 和容量仍可见。

页面证据：[主机错误页](/private/tmp/xbxrc-http-ui-host-error.jpg)、[云游戏错误页](/private/tmp/xbxrc-http-ui-cloud-error.jpg)、[主机成功页](/private/tmp/xbxrc-http-ui-host-success.jpg)、[成就页](/private/tmp/xbxrc-http-ui-achievements.jpg)、[统计加载中](/private/tmp/xbxrc-http-ui-statistics-loading.jpg)、[统计结果](/private/tmp/xbxrc-http-ui-statistics-ready.jpg)。预览的临时 viewport 已恢复，预览服务已停止。

### 串流兼容与工作区边界

Session / Signaling 默认沿用 legacy transport；`getRemoteConsoles` / `inputConfigs` 的实际用途属于串流上下文，完成调用归属与响应合同复核。关联 141 项 streaming 回归及 legacy 空响应回归通过。本任务改动集中于非串流 HTTP 与公共认证 / 错误边界，工作区其他串流、日志隔离及发行任务的改动予以保留。

## Validation

| 验证 | 结果与范围 |
| --- | --- |
| `cargo test -p xbox-webapi -p xbox-auth-flow -p xbox-cloud-catalog-flow -p xbox-streaming -p xbxrc --offline --quiet` | 全部通过；WebAPI 59、auth flow 1、cloud catalog 8、streaming 141、Tauri 109；最终有效合计 318 项 |
| `cargo check --workspace --offline` | 最终代码通过，覆盖共享 crate、Tauri 与 iOS bridge 编译合同 |
| `cargo fmt --all` | 完成 |
| `pnpm lint:fix` | 通过 |
| `pnpm typecheck` | 通过 |
| `pnpm build` | 通过，355 modules |
| `pnpm test` | 全量运行 71 文件，681 / 681 通过 |
| `git diff --check` / `git diff --cached --check` | 通过 |
| 实际 Rust 服务只读复核 / Vue 隔离预览 | 结果见上述接口与页面证据；TUN 三模式和真实应用验收保留明确边界 |

故障回归覆盖：401 / 403 / 404 / 429 / 5xx、网络分类、总预算 / 服务端退避、只读 POST 与命令提交次数、空 / 非法 / 超限响应、分页头、Smartglass 服务状态、Token 余量与并发 / 取消、账号切换、云缓存与批量取消、媒体临时文件和签名、更新候选及渠道变化、诊断隐私。

## Risks

- 云授权的资格条件由微软服务决定。本轮新 GSSV XSTS 的 xgpuweb / xgpuwebf2p 仍为 403，具体拒绝码缺失；[微软官方地区表](https://www.xbox.com/en-US/regions)可用于核对服务覆盖。当前证据只能确认云服务拒绝，具体地区 / 订阅 / 账户原因需要更多服务事实。
- 主机管理依赖有效 Web 凭据和设备权限，真实只读查询已成功；云准入错误按独立服务归因。直连 / 普通系统代理 / TUN 还需要同一有效凭据、账户、地区和接口现场对照；本轮当前 TUN 状态未确认。
- 真实主机电源 / 文本、签名媒体保存、更新下载 / 安装和已安装应用的运行效果由用户实机验收；本次通过本地故障注入和回归验证这些路径。

## Follow-up

1. 运行包含当前代码的应用，由用户在关闭 TUN 和系统代理的条件下复测主机、资料、好友、历史 / 时长、成就、相册和公开目录；认证恢复沿用实际凭据有效性和 401 策略。
2. 云游戏固定账户和配置比较三种网络模式，依据服务支持范围和返回码核对准入原因。
3. 在真实主机和发行环境中验收管理命令、媒体保存与更新重试，确认网络中断后的提示和再次操作结果。
