# 云游戏、成就与资料接口直连 / 系统代理诊断

- 日期：2026-11-08（Asia/Shanghai）
- 范围：当前 `main` 的 Xbox HTTP API、认证回退、macOS 网络与代理；本次完成诊断与只读对照。

## 结论

成就、资料与公开游戏目录可以通过普通 HTTPS 直连。本机本次测试的 DNS、TCP 和 TLS 均正常，Rust `reqwest` 直连和 macOS 系统代理均取得服务器响应。云游戏另外受服务地区与账户资格约束；本地最近认证快照保留了两条云游戏授权 HTTP 403。

确认一处独立代码缺口：个人数据查询的 Web Token 回退会取出过期凭据，并提前返回，使该查询跳过后续认证检查。本地保存的 Web Token 已于 **2026-11-08 00:10:59 +08:00** 过期；用它只读查询本人资料，直连与代理均返回 HTTP 401。

历史现象“开启 TUN 后成功”的直接原因仍需同一有效凭据、同一业务请求的失败 / 成功样本确认。TUN 会影响 DNS、路由和出口 IP；本次证据支持优先处理 Token 生命周期与云游戏准入错误。

## 网络与客户端证据

- Clash Verge 当前配置：`enable_tun_mode=false`、`tun.enable=false`、`enable_system_proxy=true`。
- macOS 动态配置：HTTP / HTTPS 系统代理均启用，地址为 `127.0.0.1:7897`；PAC 关闭。
- 成就、Game Pass 与云游戏探测目标的路由均走 `en0`，网关为本机局域网默认网关；系统活动网络接口为 `en0`。
- 直连探测清除全部大小写代理环境变量，并显式绕过 HTTP 代理。
- Rust 对照复用当前构建的 `reqwest 0.13.2`，使用相同 TLS 与代理依赖。两种模式都清除代理环境变量；系统模式保留默认客户端，直连模式使用诊断客户端的 `no_proxy()`。系统模式的服务器连接地址为 `127.0.0.1:7897`，直连模式连接目标公网地址。
- 当前构建已包含 `system-proxy`。依赖源码在创建 Client 时读取代理环境及 macOS HTTP / HTTPS 系统设置。应用的 `HttpTransport` 使用默认 ClientBuilder，来源见 [transport.rs](../../crates/xbox-webapi/src/transport.rs)。

| 探测目标 | 直连 HTTP | 系统 / 显式 HTTP 代理 HTTP | 含义 |
| --- | --- | --- | --- |
| `achievements.xboxlive.com/` | 404 | 404 | 根路径返回响应；证明连通性 |
| `profile.xboxlive.com/` | 404 | 404 | 根路径返回响应；证明连通性 |
| 云游戏区域 `/v2/titles`，无凭据 | 401 | 401 | 服务已响应认证要求 |
| Game Pass 公开 SIGL 目录 | 200 | 200 | 公开目录直连成功 |
| Device / SISU / XSTS 认证端点，GET 探测 | 405 | 405 | 认证服务器可达，业务使用 POST |
| Live OAuth Token 端点，无参数 GET | 400 | 400 | 认证服务器可达 |
| `xgpuweb` / `xgpuwebf2p` 登录端点，GET 探测 | 404 | 404 | 路径探测已到达服务；业务使用 POST |
| 本人资料业务查询，携带本地已过期 Web Token | 401 | 401 | 两条路径均拒绝旧凭据 |

共 22 项 curl 对照与 6 项 Rust 客户端对照，全部取得 HTTP 响应。无凭据探测用于证明网络连通性；业务资格以有效认证请求的响应为准。

## 认证与错误处理证据

### 过期 Web Token 回退

1. [token_policy.rs](../../src-tauri/src/mods/auth/token_policy.rs) 已有 `is_web_token_valid()`，正常会话快照据此检查有效期。
2. [persistence_service.rs](../../src-tauri/src/mods/auth/persistence_service.rs) 的 `get_web_api_tokens()` 从保存数据提取 Token 与 UHS 后直接返回；该路径的检查覆盖字段存在性。
3. [session_resolver.rs](../../src-tauri/src/mods/data/session_resolver.rs) 在有效会话快照缺失时先选择 Web Token 回退，再调用 `check_authentication()`。保存着旧 Token 时，回退提前返回。
4. 成就、资料与游戏历史使用这份 Web 凭据。本人资料的过期凭据 GET 对照已复现 401；成就查询的同类影响来自共享凭据与调用链的代码证据。

建议复用既有 Token 有效性策略，让过期回退进入认证恢复；401 恢复应具有一次刷新与重试的明确边界，保持并发认证去重。

### 云游戏准入

本地最近认证快照的安全摘要：

| Offering | HTTP | Timeout | Retriable | 地区请求头配置 |
| --- | --- | --- | --- | --- |
| `xgpuweb` | 403 | false | false | 已配置 |
| `xgpuwebf2p` | 403 | false | false | 已配置 |

这两条记录表明服务器拒绝云游戏授权。保存的摘要覆盖状态与分类，具体服务端错误码仍需采集，才能区分地区、账户资格与其他访问条件。

[Xbox 官方地区说明](https://www.xbox.com/en-US/regions)的[当前地区表](https://www.xbox.com/en-US/regions/JS/regionsTable.json)中，中国大陆 `keyZhcn` 的四项 Cloud Gaming 值均为 `####`，美国值为 `y`；中国大陆处于当前官方云游戏服务覆盖范围之外。公开 Game Pass 目录访问与云游戏授权分别受各自服务合同约束。

[auth_api.rs](../../crates/xbox-webapi/src/auth_api.rs) 的 `force_region_ip` 会写入 `x-cosmos-ip` 请求头。真实连接出口由网络路由与代理决定，服务端对该请求头的处理以实际响应为准。

### 诊断缺口

- [personal_service.rs](../../src-tauri/src/mods/data/services/personal_service.rs) 将 401 / 403 / 429 转成固定提示，其余错误统一显示服务暂不可用。
- [error.rs](../../crates/xbox-webapi/src/error.rs) 转换网络错误时保存 `err.to_string()`；DNS、连接、TLS 的底层 source 链需要进一步保留安全分类。
- 当前最近 trace 为 `runtime-trace-1794010254867-1.jsonl`，2026-11-07 08:10:54 至 08:11:01，共 11 行，schema v3 / dev，包含启动与输入事件。该文件提供的 HTTP 失败证据为空；本次云游戏 403 来自安全读取的认证快照。

建议 HTTP trace 保留 API 类别、尝试次数、耗时、HTTP 状态、服务端错误码、DNS / TCP / TLS / timeout 分类，以及客户端代理来源与代理配置代次。Token、UHS、XUID、完整用户 URL 与响应正文采用隐私白名单处理。

## 验证边界与后续顺序

本次完成源码检查、系统配置与路由检查、trace 摘要、28 项只读网络对照及文档差异检查。应用运行代码与系统代理设置保持原有版本；本次诊断覆盖 HTTP API，游戏媒体 UDP、ICE 与上屏指标由实际串流验收提供证据。

后续优先修复过期 Token 回退与错误分类，再用有效凭据核对云游戏的地区 / 账户资格。解释 TUN 切换差异时，采集相同业务请求的直连、普通系统代理和 TUN 三组安全 trace，并固定 Token、地区与账户条件。
