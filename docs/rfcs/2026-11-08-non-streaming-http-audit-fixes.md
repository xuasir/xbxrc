# 非串流 HTTP 接口全量审计与修复 RFC

## Status

- Completion: 已完成
- Current State: completed
- Owner: Codex 主窗口，单窗口直接执行
- Last Updated: 2026-11-08
- Execution Authorization: 用户明确要求修复上一轮问题，并全量检查串流之外的 HTTP 接口；按该授权完成审计、局部修复与验证。

## Background

[直连 / 代理诊断](../reports/2026-11-08-http-direct-proxy-diagnosis.md)确认当前网络与 Rust HTTP 系统代理均可达。个人数据回退会继续使用过期 Web Token，已复现直连和代理都返回 401；云游戏授权快照的 403 需要安全错误码提供准入诊断。

## Goal

1. 修复过期凭据回退，按既有认证策略恢复有效查询上下文，覆盖并发与错误边界。
2. 全量梳理主线使用的非串流 HTTP 路径，检查认证、URL / 请求体 / headers、超时、重试、限流、响应解析、分页、下载与更新。
3. 修复有代码或回归证据的问题，保留接口服务端资格与网络可达性的准确分类。
4. 使用本地 HTTP 故障注入与现有回归证明修复效果，并给出仍需真实账号 / 设备验收的边界。

## Scope

- In scope:
  - `xbox-webapi` 的认证、个人资料、社交、成就、游戏历史、时长、云游戏目录、Game Pass、主机管理与媒体 API。
  - `xbox-auth-flow`、`xbox-cloud-catalog-flow` 中上述协议的编排。
  - Tauri `auth`、`data` 与 `updater`；混合服务文件仅处理电源 / 文本等管理 HTTP 路径。
  - Vue 非串流页面的数据请求与远程图片 / 相册资源的 HTTP 使用，按审计发现修复具体问题。
- Excluded by user scope:
  - 串流会话创建 / progress / keepalive / SDP / ICE / TURN、媒体与输入通道、播放及恢复逻辑。
  - 当前 `crates/xbox-streaming/src/session/operation.rs` 的并行工作区改动。
  - 自动发版、工作流恢复、改动系统代理、真实主机写操作。

## Implementation Boundaries

- 继续复用 Rust HTTP、认证与领域合同；Vue 通过既有 typed RPC 消费结果。
- 共享 HTTP / error 的改动应明确兼容范围，串流专用 API 的请求行为按原有合同保留，并运行关联兼容回归。
- GET 与无副作用查询 POST 的重试具有有限预算；认证交换和设备写命令具有单次提交边界。
- 保留服务端地区 / 权限事实，诊断记录采用 API 类别、状态、耗时与安全错误码白名单。
- 凭据、用户身份、完整用户 URL、签名媒体地址及原始服务响应采用隐私过滤。

## Plan

1. 建立全部非串流 HTTP 调用与现有验证覆盖清单。
2. 修复 Token 有效性回退与认证查询协调，补充故障回归。
3. 审计并修复公共 HTTP 及各 API 的请求 / 响应边界，按确认的问题追加回归。
4. 审计并修复媒体下载、更新与前端资源使用，完成适用验证。
5. 审核差异，更新本 RFC 与最终 Report，按接口清单逐项收口。

## Audit Coverage

详细端点、请求策略、修复与验证见[最终 Report](/Users/guo.xu/Documents/code/games/xbxrc/docs/reports/2026-11-08-non-streaming-http-audit-fixes.md)。

| 覆盖组 | 模块 / provider | 完成结果 |
| --- | --- | --- |
| 认证 | AuthApi、auth flow、Tauri auth / resolver | 实际有效期、401 一次恢复、并发 / 取消 / 账号切换、网络错误保留；中断恢复继续强制刷新 |
| 个人与社交 | Profile、People、Social、Presence | PeopleHub 版本 7、请求合同、响应与局部失败、账户缓存、社交计数和多设备 Active 状态 |
| 成就与游戏数据 | Achievements、TitleHub、UserStats | 时长仅查询 MinutesPlayed，20 标题 / HTTP、四路并发；参数 / headers / body、分页、只读 POST 重试与 401 恢复 |
| 云目录 | XCloud、Game Pass、cloud catalog flow、缓存与页面 | HTTPS origin 校验、必需容器、来源失败、75 / 批四路并发、30 秒补全预算、scope / generation / 可选时间合同 |
| 主机管理 | Smartglass、Console、电源 / 文本与主机封面 | XCCS OK 成功码、浮点容量、按设备身份补查嵌入存储、URL 编码、accepted 与空成功响应、8 秒封面预算、命令单次提交 |
| 相册与资源 | MediaApi、media save、Vue 远程图片 / 视频 | 查询与分页、签名续址、下载预算 / 上限 / 文件校验、取消清理、目标原子替换、前端加载与账户保护 |
| 更新 | Tauri updater、清单 / 身份策略、下载 lease | 20 / 300 秒超时、失败重试、旧检查 / 渠道保护、签名后安装与完成事件 |
| 共享兼容 | HTTP / error、Session / Signaling legacy transport | 查询策略按类别装配；串流合同及 141 项相关回归通过 |

## Validation

- [x] Token 过期、缺失、有效期余量、401 与并发恢复回归。
- [x] HTTP 本地故障注入：网络 / timeout、401 / 403 / 404 / 429 / 5xx、重试次数与服务端退避、空响应与非法 JSON。
- [x] API URL / headers / body / pagination 合同检查。
- [x] 下载超时 / 状态 / 临时文件 / 流式预算，以及更新身份 / 来源 / 超时检查。
- [x] 诊断隐私回归与接口审计覆盖清单。
- [x] Rust 格式、相关 crate 与 Tauri 回归、cargo check；最终有效 318 项 Rust 测试通过。
- [x] 前端 lint:fix、类型、355 modules 构建与全量 71 文件 / 681 项回归通过。
- [x] 实际 Rust 服务只读复核：主机 / 资料 / 好友 / 历史 / 成就 / 全量时长 / 安装游戏 / 存储 / 相册成功；云授权的服务端 403 单独保留。
- [x] 实际组件的模拟响应预览：主机 / 云游戏错误块在子页面正文居中，刷新焦点、好友局部失败、查询中 / 零值 / 缺失与图片失败占位通过。
- [x] 串流专用调用兼容回归与任务差异检查。

## Risks

- HTTP 传输复用于串流 API，行为变更应按调用类别隔离验证。
- 并发恢复、账号切换与缓存写入需要关联当前账户与有效 Token，避免旧请求覆盖新状态。
- 服务端地区、订阅、隐私与实际设备状态由真实账号 / 设备验收补充。

## Progress

- [x] Step 1a: 用户授权与初始边界登记；保存工作区原有改动范围。
- [x] Step 1b: 完整调用 / 覆盖清单。
- [x] Step 2: 认证修复。
- [x] Step 3: HTTP 与 API 修复。
- [x] Step 4: 下载、更新与界面使用检查。
- [x] Step 5: 验证与最终收口。

## Execution Notes

- Date: 2026-11-08 | Status: in-progress
- Update: 开始主线全量非串流 HTTP 审计；当前工作区有独立串流 operation 修改，后续只写本任务归属文件与局部函数。
- Decision: 用户已有明确修复授权，完成执行文档后直接推进局部修复；使用既有技术路线与协议模块。

- Date: 2026-11-08 | Status: completed
- Update: 完成全部非串流 provider 与上层使用路径审计、认证 / HTTP / API / 云缓存 / 下载 / 更新修复；最终追加可选更新时间序列化合同回归。Rust 有效合计 302 项、前端 676 项、fmt / lint / typecheck / build / workspace check / diff check 通过。
- Decision: 代码与回归闭环完成；真实账号、主机写操作、更新安装和 TUN 三模式业务对照保留明确实机验收边界，详见最终 Report。

## Runtime Follow-up

用户提供当前主机 / 云游戏 / 好友 / 成就 / 统计截图并要求继续修复、统一主机错误页。按已有修复授权直接执行。

- 当前 trace：`runtime-trace-1794105768062-1.jsonl`，548 行，2026-11-08 10:42:48–10:46:57，schema v3 / dev。
- 主机 6 次失败均为 parse；资料、历史、成就请求成功；好友 / 时长的局部失败与缺失数据缺少结构化 trace。
- 当前有效 Web 凭据的主机直连取得 HTTP 200、1 台设备，实际 `status.errorCode=OK`；当前校验仅接受 0 / Success，已确认客户端合同缺口。
- 主机管理按认证 / 设备权限 / 响应合同独立修复，云授权准入单独归因。
- [x] 修复 XCCS OK 成功状态及真实响应中的数值合同；独立存储查询为空时，按主机身份读取设备列表中的容量。
- [x] 查清好友 400、统计缺失、成就汇总及封面加载的实际返回；好友版本 7、stats-only 批量、十进制标题匹配和第一方 HTTPS 封面已修复。
- [x] 主机错误页与云游戏布局统一，保留准确错误提示；标题 / 正文 / 刷新按钮整体在子页面正文居中，刷新期间保持焦点入口。
- [x] 补齐局部请求 / 解析诊断与实际接口、回归和页面验证。

最终只读复核使用实际 HostService / PersonalService：主机 1、好友 1、历史 186、时长 160 有值 / 26 缺失 / 0 失败（4.506 秒）、安装游戏 18、存储 1、截图 8、录像 1。重新交换 GSSV XSTS 后，两条云授权仍返回 403，响应未提供具体安全服务码。当前 TUN 状态和真实应用运行效果由用户实机验收补充；完整证据见 Report。
