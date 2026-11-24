# iOS 与 PC HTTP 请求链路差异审计 Report

## Summary

- Related RFC: [审计 RFC](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-11-24-ios-pc-http-parity-audit.md)。
- 已完成只读审计及收尾，结论固定于 UTC 2026-11-24 07:09:23 的 main 工作树快照，HEAD `b6ac29c0599b7651ff9db84395bbbb6fdc7da748`。本任务业务代码修改为 0，未授权或实施网络修复。
- 两端业务协议和多数 HTTP provider 共用 Rust reqwest。稳定性差异主要在平台代理发现、宿主额外 deadline、错误保真、目标凭据恢复、缓存和下载；不能归因为 iOS 另用一套业务 HTTP 库。
- 静态审计不是现场根因证明。尚未完成同包、同网的手机/PC认证请求对照。

## Delivered

- [完整边界与差异矩阵](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-pc-http-parity-audit/http-diff-matrix.md)：15个共享provider、55个公开异步方法、12处Swift平台托管调用，覆盖发起、预算、重试、401恢复、缓存、取消、错误和下载。
- [锁定版本上游证据](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-pc-http-parity-audit/upstream-evidence.md)：reqwest 0.13.2 / hyper-util 0.1.20 的包checksum、系统代理平台条件与TLS摘录。
- [来源指纹](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-pc-http-parity-audit/source-hashes.json)：146个候选快照文件、原8处并发变化和补充关键来源。
- [验证摘要](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-pc-http-parity-audit/validation.json)、[续接验收原始记录](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-pc-http-parity-audit/closure-verification.json)、[快照后变更](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-pc-http-parity-audit/post-snapshot-drift.json)与[清理摘要](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-pc-http-parity-audit/cleanup.json)。

## Key Findings

1. **先验证实际出网路径。** 同样启用system-proxy，不代表两端出口相同：锁定依赖读取macOS/Windows系统代理，但不自动读取iOS Wi-Fi HTTP代理；iOS显式/环境代理和更低层TUN仍可能生效。现场组合未读取、未证明。
2. **主机查询存在确定的额外截断。** iOS fetch_hosts 外层8秒短于共同18秒总预算；两次最慢5秒建连加250ms退避需要10.25秒。此差异影响主机查询，不解释初始beginLogin失败。
3. **错误保真影响定位。** iOS Query缺网络细分/安全服务码，部分Data字符串又降级为unknown；PC详细错误保留更多安全分类。这是诊断落差，不是网络失败本身原因。
4. **核心续期一致，但Cloud消费者接线不齐。** PC Cloud scoped查询明确提交被拒目标token并恢复一次；iOS普通等待/资源查询不能认定具备等价的目标拒绝恢复。不能把已登录后的Cloud失败当作初始登录根因。
5. **缓存和资源路径影响“更稳”的体验。** PC详情持久缓存/stale回退、激活失败恢复和严格下载保证减少故障暴露；iOS内存缓存、平台图片/视频和通用UniFFI取消各有边界。不能说iOS无缓存，也不能说PC所有RPC都可取消。

前轮7次beginLogin失败均未进入系统网页登录；五次快速network失败不能用简单延长timeout解释。具体请求、DNS/connect/TLS/body与运行包来源仍需运行证据。

## Changes

- 仅新增/更新任务RFC、Report、任务行和本任务证据目录。
- 沿用11-08 HTTP审计及11-23共享认证续期的当前约束，不将历史“iOS每次全量认证”当作现状。
- 原快照内8处非本任务改动已保留并按HTTP范围回读；续接后的持续改动只记录，不干预、不恢复、不继续追逐。

## Validation

| 检查 | 结果与边界 |
| --- | --- |
| 原快照源码断言 | 前轮18项PASS；续接未重跑，不能宣称覆盖新源码。 |
| Provider / 方法覆盖 | 续接逐provider方法列表与矩阵精确相等，15 / 55，PASS。方法数不等于URL数或实际请求次数。 |
| Swift托管调用 | 8个AsyncImage + 图片loader/下载/AVPlayer/网页登录各1，共12，PASS。 |
| 初始文档引用与差异格式 | 续接最初78个本地文件链接及起始行有效；4份Markdown差异空白检查通过。 |
| 两目标离线依赖 | iOS aarch64-apple-ios与PC aarch64-apple-darwin各执行feature/normal树；4条cargo tree --offline --locked命令exit 0。特征与前轮一致，均含平台证书verifier和Apple Security Framework。 |
| 上游完整性 | 2份发布包checksum与Cargo.lock一致，3份关键源码hash与摘录一致；不执行下载包代码。 |
| 当前工作树指纹 | 续接原始记录13项PASS / 3项FAIL。三项失败均为当前来源与旧快照一致性检查，失败原样保留，不声称当前工作树全量静态PASS。 |
| 快照后变化 | UTC07:41:59对147个唯一来源复核时，记录9个变化来源（含生成绑定）；仅记录，未纳入原快照验收。 |
| 临时清理 | 进程和文件占用检查通过；两个目录、25文件已删除，约0.95MiB已分配文件块，目录残留0。 |

没有运行Rust/iOS/前端构建测试、格式化、手机/PC认证网络试验或WebRTC媒体验证。桌面curl或Safari成功也不能代替iOS Rust请求。

## Risks

- 手机和PC运行包与审计源码快照尚未配对，不能确认设备执行的就是这些代码。
- 实际直连/系统代理/环境代理/TUN、DNS/IPv6和信任配置未配对核验，登录网络根因未确定。
- 本Report交付的是固定快照审计；快照后源码变化及其功能回归由相应任务验收，不将其当作本轮PASS。
- 原始用户日志、共享缓存、其他任务的源码、暂存状态和证据均未由本任务改动。清理只覆盖已授权的旧r1及当前r2子目录。

## Follow-up

- P0：固定手机/PC包来源与同网条件，补首次认证逐请求stage、网络细分、耗时、状态/安全服务码及parent operationId；不得记录token、seed、JWK、完整URL、代理地址或身份。
- P1：另行确认后对齐iOS主机预算和类型化错误，并补齐Cloud rejected-token单次恢复。
- P2：按现有owner与缓存边界逐项评估持久/stale缓存、首次失败激活、显式取消及相册下载保证。

后续业务修复需单独确认范围与运行验证目标；不增加平行HTTP/bridge路线，不绕过证书校验，不无条件重试写POST。
