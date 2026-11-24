# iOS 接口调用、返回与异常行为对齐 PC（P0–P2）RFC

## Status

- Completion: 代码与自动验证完成；实机验收待用户
- Current State: validation-pending
- Owner: 主窗口，单窗口直接执行
- Last Updated: 2026-11-24
- 执行授权：用户明确要求“系统代理、HTTP 库不是关注点……以 PC 为蓝本去对齐，实施 P0-2”；本 RFC 将既有审计建议按这一边界收敛为可执行合同，不重新开展代理调查。

## Background

[已关闭的审计](/Users/guo.xu/Documents/code/games/xbxrc/docs/reports/2026-11-24-ios-pc-http-parity-audit.md)记录 iOS 在主机查询预算、响应容器、错误事实、Cloud 目标拒绝恢复、缓存与资源失败处理上的落差。该审计仍以其固定快照为准，本轮重新核对当前代码。

两端串流已共用 Rust SessionFlow。当前共享流程对明确 ServerNeverRegistered 终态停止继续重建，并投影 HostRegistrationRetryExhausted；PC 给出主机未完成注册、等待开机后再连接的提示，iOS 直接显示重试用尽。iOS UI 对部分 HostRemotePlayUnavailable 的 retryable 也未应用 PC 的最终禁止立即重试规则。修复需要同时对齐领域事实、用户含义和恢复动作，而不是扩大自动重试。

[启动流程历史 RFC](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-11-23-ios-startup-pc-parity.md)已经完成共享凭据缓存、目标续期、checkpoint 与 90 秒 startup scope；其完整实机验收仍待用户，不在本任务中代为关闭。

## Goal

- PC 是行为基准：相同服务事实得到相同错误类别、恢复建议和重试边界。
- 合法空响应与无效响应明确区分；慢请求使用既有公共预算，不提前截断。
- Cloud 目标 401 只向对应凭据 owner 提交被拒 token，恢复一次后重试读取；临时失败保留登录和成功内容。
- 初刷失败可在冷却后重新激活，详情与下载具备有界缓存、取消和失败清理保证。

## Scope / Milestones

| 阶段 | 实施合同 |
| --- | --- |
| P0 | 对齐主机不可用/尚未注册的用户含义及禁止立即重试策略；补齐 Query 网络细分、安全服务/协议码，UI 与 trace 消费同一类型化事实。已有首次认证逐阶段安全诊断继续复用。 |
| P1 | 去掉 iOS 主机查询独有的 8 秒外层截断；异常主机容器不再成功空列表；普通 Cloud 等待/目录/资源读取接入 rejected-token 单次恢复，失败和换账户遵循现有 owner。 |
| P2 | PC 方向的 15 秒失败冷却/重新激活、成功内容保留、详情按市场/语言的有界持久缓存与 stale 回退、同键在途合并；入口取消沿既有 UniFFI 生成/修补机制；相册下载对齐续址、状态分类、预算/大小/格式和临时文件清理。 |

影响范围：`crates/xbox-ios-bridge`、必要的共享错误分类、iOS RustBridge/State/Models/Streaming/相册下载及其测试、UniFFI 绑定生成修补脚本。PC 业务逻辑原则上保持不变，仅用作基准与回归验证。

不包含：系统代理、HTTP 库替换/版本变更、第二套 transport/bridge、全局加长超时、无条件写 POST 重试、自动反复重登、证书绕过、认证网络实测或对移动登录根因作推定。缓存只保存可公开商品资料，不持久化访问令牌或签名下载地址。

## Plan

1. 保存当前工作区基线，核对相关历史和 PC 合同，登记本 RFC。
2. 实施 P0/P1，添加主机/Query/Cloud 拒绝恢复回归。
3. 实施 P2 生命周期、缓存、取消与下载失败处理，添加有界与隔离回归。
4. 运行 Rust fmt/test/check、Swift 定向测试、绑定复现和 iPhoneOS 编译；如修改前端代码执行 pnpm lint:fix。
5. 归档本轮独立差异、测试摘要、失败诊断和限制；仅清理本轮登记的 scratch/构建产物，保留其他任务和共享缓存。

## Validation

- [x] 主机不可用、注册失败、预算耗尽、网络/认证/运行时错误的类型与动作回归。
- [x] 主机合法空列表、嵌套容器、无效容器与公共请求预算回归。
- [x] Query 网络细分、安全服务/协议码与敏感正文过滤回归。
- [x] Cloud rejected-token 单次恢复、二次拒绝、临时失败、取消与 owner 隔离回归；Swift 三个消费者共用 Rust 恢复边界。
- [x] 15 秒失败冷却、取消后重新激活、成功快照保留、详情新鲜/stale/过期、市场语言隔离、合并与容量回归。
- [x] UniFFI 取消传递、Rust future 释放回归、Swift 6 编译及绑定复现。
- [x] 相册续址接线、HTTP/空文件/大小/格式分类、取消和校验后导出保证；300 秒资源总预算。URLSession 的连接专用预算保留平台差异，见执行说明。
- [x] Rust、Swift、绑定与设备编译证据归档，暂存区指纹保留，本轮临时产物清理。
- [ ] 用户实机验收主机再次连接、真实 Cloud 401、相册服务响应及长测。

自动测试采用无凭据的本地 fixture，不发起真实认证请求。真实手机上的主机等待/再次连接、Cloud 401、相册服务回包与长测由用户手动验收；未完成时明确记录，不宣称根因已验证。

## Risks

- 当前工作区包含大量其他任务改动。修改前基线与暂存区指纹已保存在本轮 scratch；采用局部 Edit，不恢复、不暂存、不提交无关内容。
- 可恢复读取只执行一次目标恢复；写命令仍沿公共不自动重试合同。
- UniFFI 新字段与生成代码必须同步，禁止手工维护平行绑定；取消必须保证句柄一次释放和结果竞态安全。
- 持久缓存遵循现有市场/语言键与有界容量，不跨账户使用认证结果，不缓存签名下载 URL。

## Progress

- [x] PC 基准、历史约束、当前基线与执行授权核对。
- [x] P0/P1 实施与回归。
- [x] P2 实施与回归。
- [x] 自动验证、归档与清理。
- [ ] 实机验收。

## Execution Notes

- 2026-11-24：登记独立实施任务。基线 265 文件 / 4,746,863 字节，存于 `$PI_SCRATCH_DIR/ios-pc-http-behavior-parity-r1/baseline`；branch main，HEAD b6ac29c0599b7651ff9db84395bbbb6fdc7da748，保留全部原 staged/unstaged 变更。
- 2026-11-24 续接：P0/P1 和 P2 代码改造已收齐。Rust 三包 200 项通过；Swift 三次定向验收覆盖 42 项唯一测试，最终新增的三个恢复场景全部通过；iPhoneOS 编译、Swift 6 绑定类型检查、生成复现、fmt 与 session boundary 通过。
- Cloud 等待、目录、来源与官方触屏资源使用同一 owner-bound Rust 恢复入口；Swift 目录/来源/资源层直接消费最终拒绝事实。页面取消继续传至内层 Rust future。
- 详情公开资料缓存采用 24 小时新鲜、7 天 stale、30 槽、两路请求，取消等待者按引用计数结束在途任务。相册每次导出前续址，2 GiB 上限及真实文件签名校验通过后发布文件，HTTPS 重定向保持受控。
- URLSession 资源总期限和等待配置使用 300 秒；其公开 API 对连接阶段的独立预算支持有限，PC 的 5 秒 connect timeout 保留为平台差异，避免把 5 秒误用于整个响应读取的空闲期限。
- 本轮续接差异、失败诊断、测试摘要与清理记录归档至 [证据](../../dev-docs/validation/2026-11-24-ios-pc-http-behavior-parity/README.md)，实施报告见 [Report](../reports/2026-11-24-ios-pc-http-behavior-parity.md)。历史 startup 实机验收保持原任务状态。
