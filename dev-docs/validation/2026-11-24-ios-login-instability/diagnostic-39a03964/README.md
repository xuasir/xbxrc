# 移动端首次登录不稳定：39A03964 诊断

任务：`2026-11-24-ios-login-instability-trace`。本轮仅分析，不修改业务代码。

## 1. 范围

- 输入：用户提供的 `XBXRC-Diagnostic-39A03964-ED0C-4909-961C-CBAEE4E940C8.jsonl`，原件保留在用户附件位置。
- 1 个 iOS 会话，37 行，20,851 字节，schema v3，production trace；应用 `0.1.0`、build `1`、debug 构建。
- UTC 时间：2026-11-24 05:36:05.782 至 05:37:44.346；北京时间：13:36:05.782 至 13:37:44.346，共 98.564 秒。
- 输入 SHA-256：`e065ed89f0f32df2463c8f9bcfb632c23d6742108c504bbccff9c77e65180457`。
- 完整机器可读锚点、源文件哈希和检查范围见 [analysis.json](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-login-instability/diagnostic-39a03964/analysis.json)。

## 2. Gate

执行 iOS trace 分析器的 strict/all 与 strict/startup 两种范围：

- **startup：PASS**，退出码 0。
- **all：FAIL**，退出码 2；仅缺游戏库 `library page`、`catalog activation decision`、`library presentation` 三类锚点。尚未成功登录的导出不覆盖游戏库，不代表 JSONL 损坏。
- JSON 完整性、schema、session 内 seq、隐私与已提供预算事实均无违规；seq 连续为 1 至 37，末行完整且有换行。
- 内置 pairing 无违规，但仅评估 1 个开始事件，不覆盖交互登录。独立核对发现第 7 次登录缺外层终态，详见下文。

## 3. 最小时间线

恢复链：`auth/authRestoreStarted`（seq 3，tsMs 1795498565833）→ `auth/authRestoreSucceeded`（seq 7，tsMs 1795498565848）。共同 operationId 为 `3C7C5791-F99D-4FEE-9BD3-8D3B1BC739FA`，记录耗时 14ms，结果是 **signedOut**，不是令牌刷新失败。

下表的边界失败均为 `domain=auth`、`event=uniffiBoundaryFailed`、`operation=beginLogin`：

| 尝试 | seq 开始 / 边界失败 / 外层终态 | 边界 tsMs | 边界 operationId | 边界耗时 ms | 错误 |
| --- | --- | --- | --- | --- | --- |
| 1 | 13 / 14 / 15 | 1795498567498 | 389AB530-6B86-4BFA-B2A7-73480830BAE0 | 398 | network |
| 2 | 16 / 17 / 18 | 1795498574968 | FFD57229-BE4B-4878-979C-022B7E595679 | 6722 | timeout |
| 3 | 19 / 20 / 21 | 1795498577545 | C5625C06-3D2C-489D-9303-90458DE71828 | 149 | network |
| 4 | 22 / 23 / 24 | 1795498578302 | ED26A68E-0BA4-41A2-8DC5-D86227728AC4 | 204 | network |
| 5 | 25 / 26 / 27 | 1795498578771 | 33023814-3E0D-4EB9-A205-A180C65F7A6D | 191 | network |
| 6 | 28 / 29 / 30 | 1795498585434 | EDCB8787-3C17-4593-B175-5D257DE860D7 | 6121 | timeout |
| 7 | 36 / 37 / 未覆盖 | 1795498664346 | E96D9536-2053-4DAD-A06B-14C23069ABD2 | 387 | network |

统计：7 次 beginLogin 均失败，5 次是 149–398ms 的快速网络错误，2 次是 6.121/6.722 秒超时；不是“每次都超时”。全部标记可重试，没有记录 HTTP 状态码。

外层 signIn 与内层 UniFFI 分别生成 operationId，不应把两者当作同一个 ID。这里按相邻 signIn 开始事件之间的窗口关联，每个窗口恰有一个 beginLogin 失败；完整外层 ID 保存在 analysis.json。

## 4. 覆盖与缺口

- 启动、signedOut 恢复、cacheRestoreSkipped 和交互登录开始/失败已覆盖。
- 每次都有 beginLogin 的正向失败证据，结合当前异常传播顺序，故障在网页登录打开前，尚未走到 ASWebAuthenticationSession、finishLogin、Keychain 保存或游戏库加载。
- `uniffiBoundaryStarted/Succeeded` 在当前 Swift 中属于 debug，production 不记录它们是配置行为，不是压力丢失证据。见 [XboxAuthClient.swift](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/RustBridge/XboxAuthClient.swift:84)。
- 第 7 次导出停在完整的内层失败行，外层 authSignInFailed 未覆盖；不能把它解释成仍在等网络、死锁或损坏尾行，也不能直接判定导出缓冲导致缺失。
- 缺少初始认证的 `requestStage`、`networkErrorKind`、各请求耗时与父操作关联，不能分辨设备令牌和 SISU 哪一步失败。
- 前 6 次均在 scene active 时失败；后台切换发生在它们之后。第 7 次发生在回到 active 后，不支持“后台挂起造成这批失败”。

## 5. 预算与保留

- 该文件是聚合诊断导出，物理大小约 20.36 KiB。
- fileOpened 声明 production 配置：单文件 8 MiB、最多 4 文件、pendingRowLimit 4096。
- 初始丢失/存储失败计数为 0，没有 traceBudgetNotice，seq 无缺口；没有观察到队列压力丢失。
- 未提供原始轮转目录，**未核验实际轮转文件大小或保留数量**，不能用单个导出替代这两项审计。

## 6. 隐私

分析器未发现敏感键值、完整 URL、callback、bearer 或账户身份违规。本报告与派生 JSON 仅保留操作关联 ID、固定分类、耗时及来源元信息，不复制凭据或自由文本错误。

## 7. 原因判断

### 已证实的故障位置

**这批失败发生在首次登录前置的 Rust 认证握手，不是账号密码提交后的拒绝，也不是网页登录回调超时。**

当前顺序为：

`signIn → beginLogin/start_login → deviceToken → sisuAuthenticate → 系统网页登录 → finishLogin`

证据来源：

- [AuthStore.swift](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Features/Authentication/AuthStore.swift:206)：先 await beginLogin，成功后才调用网页登录。
- [bridge lib.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-ios-bridge/src/lib.rs:131)：每次 start_login 创建 AuthFlow。
- [flow.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-auth-flow/src/flow.rs:86)：设备令牌与 SISU 顺序执行，任一步失败立即退出。
- [auth_api.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/auth_api.rs:129)：两项请求目标来自源码，为 `device.auth.xboxlive.com/device/authenticate` 和 `sisu.xboxlive.com/authenticate`。它们是候选步骤，**不是日志已证明的失败目标**。

### 放大不稳定的实现因素

1. **没有自动重试。** AuthApi 使用 for_commands，明确 RetryMode::Never。retriable=true 是错误属性，不代表已重试；短暂网络失败会直接变成整次登录失败。见 [auth_api.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/auth_api.rs:118) 与 [transport.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/transport.rs:125)。这符合有副作用命令不盲目重试的边界，不能直接把所有 POST 改成无条件重试。
2. **登录页依赖两个前置网络步骤，且每次点击重新创建 HTTP client。** 任一步失败都无法进入网页登录；不同尝试不共享这份 client 的连接池。它是失败暴露与冷启动开销的放大因素，不是已经证明的网络根因。
3. **网络细分丢在初始登录诊断边界。** 公共 WebApiError 已有固定 DNS/connect/TLS/body 分类，但 Query 记录将非 timeout 的网络错误统一为 network；首次登录没有续期 observer 的逐步骤接线。见 [query_failure.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-ios-bridge/src/query_failure.rs:67)、[error.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/error.rs:166) 与 [XboxAuthClient.swift](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Platform/RustBridge/XboxAuthClient.swift:28)。这是定位能力缺口，不应被说成网络失败本身的原因。

### 不能据此确定的根因

- 当前公共 transport 的建连、单次请求、单个请求整体预算分别为 **5/12/18 秒**；18 秒不是整个登录的 deadline。见 [transport.rs](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/transport.rs:109)。
- 6.121/6.722 秒是整个 beginLogin 边界耗时，包含前序步骤、本地生成和调度。它与 5 秒建连上限加额外耗时相容，但不能精确证明命中了建连上限，更不是 Swift 写死 6 秒的网页登录 timeout。
- 没有 HTTP 拒绝状态，不能以账号错误或地区 403 解释本批日志。statusCode 缺失也可能来自成功响应体读取失败，**不能证明请求尚未发送或 TCP/TLS 未建立**。
- DNS、TCP 连接、VPN/TUN 路径、TLS、IPv6 的具体异常都还未被本日志证明。按当前 safe 映射，非 timeout 且可重试的 network 优先对应 DNS/connect/body，TLS 通常不可重试；但运行包没有源码 commit，不能据此严格排除其他映射。
- 云地区 IP 设置在 complete_login 才消费，start_login 不接收该参数，调地区不能解决已定位的 beginLogin 失败。

### 历史约束

[11-23 启动对齐 RFC](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-11-23-ios-startup-pc-parity.md:83) 记录过已登录后的 deviceToken 网络失败与 SISU 5 秒超时，并补充续期 observer 的安全分类。本次启动明确 signedOut，走的是首次登录，不能把“旧版反复全量续期”的问题再次当作本次根因。公共认证与 transport 保持 Rust；不引入第二条 URLSession/TypeScript 认证路线。

源码核对基于当前 main 工作树（HEAD `b6ac29c0599b7651ff9db84395bbbb6fdc7da748`，已有暂存修改）；运行包仅记录版本、build 与配置，**未证明其二进制与当前工作树一致**。本轮保留业务文件原状，并记录 9 个核对文件的哈希。

## 8. 下一步与验证边界

1. 优先为首次登录两个步骤补齐白名单 `requestStage/networkErrorKind/elapsedMs/statusCode/attempt` 与父操作关联，复用公共 Rust 诊断；不要记录 token、seed、JWK、账号或完整 URL。
2. 用同一手机、同一运行包对照 Wi-Fi 裸连与蜂窝；按实际使用情况再对照 VPN/TUN 开关。每组保留逐步骤耗时、成功率与完整外层终态，不能以桌面 curl 或 Safari 成功替代移动端 Rust 请求验证。
3. 明确底层原因后再考虑建连预算或初始握手的有界恢复。若增加重试，先证明请求未发送或步骤可安全重建，再做有限次数与退避；**只延长超时无法解释 5 次 149–398ms 的快速失败**。

本轮已执行 trace strict/all、strict/startup、原始行回读、7 次独立关联与 Swift/Rust 静态核对；没有执行业务修改、Rust/iOS 构建或设备网络实测。用户输入原件保留，本轮未创建临时日志、脚本或构建目录，其他任务产物不清理。
