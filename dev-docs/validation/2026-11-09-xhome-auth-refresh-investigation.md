# 主机串流自动续期与登录提示排查

> 2026-11-12 整理：本页保留验证摘要；带 Git 链接的截图、夹具和中间输出按采样时提交读取。

- 日期：2026-11-09，Asia/Shanghai。
- 状态：诊断与离线复现完成；20348f4c 为固定诊断基线，后续实现见[统一续期交付报告](/Users/guo.xu/Documents/code/games/xbxrc/docs/reports/2026-11-09-unified-token-renewal.md)。
- 范围：`main` 的认证状态、XHOME token 获取、静默续期、串流失败后的重新登录动作。
- 输入：[设备日志](/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1794190273035-1.jsonl:115)、[11 月 8 日令牌失效记录](2026-11-08-xhome-startup-auth-expiry.md)、相关 Git 历史与登录后凭据时效元数据。
- 固定日志快照：3,701 行，2026-11-09 10:11:13.035–10:17:17.560，dev profile，`traceBudgetNotice=0`；SHA-256 为 `e3c88d94856c299dbbc39a14221000f04354b7864fd7ac3f3a242a16232aa1d7`。
- [安全指标与源码身份](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-09-xhome-auth-refresh-investigation.metrics.json) · [离线复现脚本](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-09-xhome-auth-refresh-investigation.py)。

## 结论

确认主机串流自动续期存在调用链缺口。11 月 8 日的认证改动让有效 Web token 可以维持“已登录”；应用启动仍按旧的 `isAuthenticated` 含义决定是否检查认证，XHOME token 失效时启动检查被跳过。串流入口直接读取有效 XHOME token，空结果立即归入 Auth 错误并显示重新登录。

今天 10:12:56 的失败发生在准备上下文阶段，操作耗时 14ms，HTTP 状态为空；失败前认证检查次数为 0。资料和主机列表此前查询成功。点击重新登录后，既有动作执行完整登出，核心 refresh token 随认证缓存一起清理，登录页随后记录 `silentFlowPolicy.eligible=false`。

11 月 8 日的提示修复完善了首错与重新登录入口，自动续期仍沿用既有链路。这次排查补齐了前面的续期触发问题与历史回归证据。

## 凭据用途与有效期

refresh token 用于后台换取新的 Microsoft access token，再通过既有 SISU/XSTS 流程生成 Web 与串流凭据。SISU 凭据有效且满足复用策略时，现有实现也可以直接重新生成下游 token。

| 凭据 | 用途 | 本地登录后样本 |
| --- | --- | --- |
| Microsoft refresh token | 后台续期与延续登录 | 存在；最终可用性由 Microsoft 的续期响应确认 |
| Microsoft access token | Xbox/SISU 授权 | 返回 `expires_in=86400` |
| Web XSTS token | 资料、主机列表、管理与个人数据查询 | 2026-11-10 02:13:43.594333 +08:00 到期 |
| XHOME `gsToken` | 创建和访问主机串流会话 | `durationInSeconds=14400`，4 小时 |

XHOME 样本创建于 10:13:43.676，缓存计算的到期时间为 14:13:43.676；JWT `exp` 对应 14:13:45，相差 1.324 秒。当前策略预留 60 秒请求余量。毫秒创建时间、秒数 TTL/JWT `exp` 与余量的离线核对通过。

上述元数据来自用户重新登录后的样本。原失败凭据已由应用登出清理，原 refresh token 的服务端状态需要续期请求结果验证。

## 最小失败时间线

| seq | 时间 | 事实 |
| --- | --- | --- |
| 3、7 | 10:11:13.035–13.949 | 初始化为 `isAuthenticated=false/appLevel=1`；路由查询得到 `isAuthenticated=true/appLevel=0`，Web-only 身份进入主机页 |
| 15、16、105 | 10:11:15–10:12:55 | 资料和主机列表查询成功 |
| 112–118 | 10:12:56.051–56.070 | `startup-1` 进入 resolvingContext；本地读取报 `Streaming token is unavailable for home`；归 Auth，`retryable=false`，操作 14ms |
| 152–172 | 10:13:07.065–07.090 | 完整登出后身份清空；此时首次执行认证检查，静默恢复资格为 false |
| 232–264 | 10:13:47.230–47.253 | 新登录产生 `appLevel=1`，session-ready 发送成功 |
| 283、331 | 10:13:47.286–10:14:04.914 | 第二次启动成功完成准备、会话创建和 runtime 启动，耗时 17.627s |

首次失败位于会话创建前。网络可达性有 Web API 成功查询证据；该失败窗口的 WebRTC 网络健康、恢复链、media supply、steady supply、submit/present 长尾与帧供给 delta 均处于媒体启动之前，媒体侧验收需要独立窗口。

## 代码因果链与历史来源

1. **Web-only 状态与启动检查的含义错位。** [runtime_state.rs](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/auth/runtime_state.rs:260) 在完整串流快照缺失时根据 Web token 设置 `is_authenticated=true/app_level=0`；[service.rs](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/auth/service.rs:518) 使用该投影返回状态。该行为来自 `21fd04f0`，2026-11-08 12:41:43。旧的 [shell 启动条件](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/shell/mod.rs:218) 继续根据 `is_authenticated` 决定是否调用 `check_authentication()`，两者组合使 XHOME 失效时自动恢复入口被跳过。
2. **主机串流入口缺少目标凭据恢复。** [context.rs](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/streaming/context.rs:58) 直接调用同步 getter；[persistence_service.rs](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/auth/persistence_service.rs:149) 返回当前有效 token 或空值。这个入口自 `bec3acfb`（2026-10-28）沿用至今。运行中跨过令牌有效期后，再次启动也会受此缺口影响。
3. **本地不可用结果直接导向完整登出。** `2013bcd2`（2026-11-08 21:42:43）将空 token 的错误改为 Auth。既有 [relogin 动作](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/useStreamExecution.ts:689) 来自 `5fd7e83c`（2026-11-04），先调用 `rpc.auth.logout()`；[logout_impl](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/auth/service.rs:111) 清理 [全部认证凭据](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/auth/storage_repository.rs:130)。该路径把本可先尝试续期的状态变为交互登录流程。
4. **通用快照还存在目标区分缺口。** [get_valid_session_snapshot](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/auth/token_policy.rs:91) 在 Home/Cloud 任一 token 有效时接受整份快照，[check_authentication_internal](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/auth/service.rs:204) 随即接受它。离线样本证明：Cloud 有效、Home 过期时通用检查仍可成功。修复入口时需要显式验证当前目标的 token。

旧版在新进程中以串流快照初始化状态：XHOME 过期且 Cloud 缺失时，状态保持 false，启动代码进入认证检查。当前版本在相同条件下投影为 true，启动检查跳过。这个差异解释了此前应用重启时可以继续沿静默续期维持登录、现在更容易在点击主机串流时要求登录。

## 离线复现

隔离 harness 固定编译 20348f4c 的 `token_policy.rs`、`runtime_state.rs`、测试文件与真实 DTO，同时编译 `21fd04f0^` 的历史 runtime-state。以下现象与源码引用对应诊断基线，当前实现以交付报告为准。存储与 OAuth pending 数据使用离线桩，测试使用合成凭据。

- 18 项：17 项通过，1 项按预期暴露当前启动恢复回归。
- 历史检查通过：`historical_startup_attempts_silent_restore_for_expired_home`。
- 当前目标行为失败：`current_startup_should_attempt_silent_restore_for_expired_home`，实际状态 `authenticated=true/appLevel=0`，shell 跳过认证检查。
- 通过项覆盖 TTL/JWT 单位、60 秒余量、Web-only 身份、Home 过期而 Cloud 有效的快照，以及现有并发、取消、generation 与拒绝标记保护。
- 独立 harness 的 `cargo fmt` 通过；真实生产源码按只读方式加载。

```sh
python3 -B dev-docs/validation/2026-11-09-xhome-auth-refresh-investigation.py
```

当前基线退出 101，目标行为的那一项失败为本次回归证据。脚本在临时目录保存测试输出，输出目录在命令结果中显示。

## 修复顺序与验收边界

1. 在 Rust 主机串流准备入口确保目标 token 有效；失效时复用已有串行静默恢复，再读取并核验该目标凭据。目标失效应触发恢复并绕过基于 Web token 的短路判断。
2. 应用启动恢复依据串流凭据需求检查；资料与主机列表继续使用当前 Web-only 身份能力。
3. 本地 token 过期先自动续期，网络类失败保留核心凭据并提供重试；服务端明确拒绝 refresh grant 后，再进入重新登录。
4. 补齐过期 Home/有效 Web、过期 Home/有效 Cloud、续期网络失败、明确 invalid_grant、并发启动与退出取消的行为回归；实机验证一次登录跨过 4 小时后再次启动主机串流。

本次验证证明调用条件和错误处理缺口。原失败 refresh token 的续期结果、修复后的跨有效期实机表现接续实现验收。
