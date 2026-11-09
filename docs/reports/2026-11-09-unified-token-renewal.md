# 统一 token 自动续期与重新登录选择 Report

## Summary

- Related RFC: [统一 token 自动续期与重新登录选择](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-11-09-unified-token-renewal.md)。
- 日期：2026-11-09；主窗口按用户要求单窗口执行。
- Web、主机串流、云串流及其数据/会话/信令入口已统一采用目标凭据校验、后台续期、有界 401 恢复和显式重新登录选择。实现与自动验证完成，真实设备跨有效期验收待用户执行。
- [诊断证据](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-09-xhome-auth-refresh-investigation.md)确认旧主机入口读取过期缓存后直接失败，启动时 Web-only 身份又使恢复被跳过。失败发生在准备上下文阶段，耗时 14ms，此前认证检查为 0 次。

## Delivered

| 触发条件 | 统一行为 |
| --- | --- |
| 当前目标凭据有效 | 直接复用；Web/Home/Cloud 分别校验所需凭据 |
| 本地凭据过期，SISU 仍有效 | 按当前目标重新派生下游凭据 |
| 核心授权需要续期 | 使用 refresh token 更新核心授权，再生成目标凭据 |
| Web/Home/Cloud 接口明确返回 401 | 重建核心授权与目标凭据，原请求重试一次 |
| 网络、限流、协议或服务临时失败 | 保留核心凭据，维持错误类别并允许用户重试 |
| 明确 invalid_grant、刷新授权被拒绝或恢复后的鉴权继续失败 | 提供重新登录选择；用户取消时保留凭据，确认后才清理并跳转 |
| 地区、订阅或权限限制 | 显示对应资格错误，保留已有认证能力 |

## Changes

- [auth/token_policy.rs](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/auth/token_policy.rs)统一目标有效期判断，TTL/JWT 同时存在时采用较早到期时间，保留 60 秒余量；有效 SISU 按真实有效期复用。同账户刷新保留其他目标缓存，账户切换替换凭据。
- [auth/service.rs](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/auth/service.rs)、[token_refresh.rs](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/auth/token_refresh.rs)共用串行恢复入口。明确 401 和被网络中断的恢复继续使用核心刷新路径；失败结果短暂合并供并发等待者复用。
- [xbox-auth-flow](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-auth-flow/src/flow.rs)区分 refresh grant 与下游错误，支持 Web/Home/Cloud 范围。OAuth 成功后先保存最新 user/refresh token，再执行下游请求；服务端省略 refresh token 时保留既有值。transfer 响应兼容 access_token/lpt 并保存轮换凭据。
- [auth/runtime_state.rs](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/auth/runtime_state.rs)以 generation 保护 checkpoint、提交、注销和账户切换。内部串流凭据带数字代次，迟到的旧账户请求采用可重试错误；OAuth 窗口关闭只取消交互登录。取消的认证 trace 仍记录唯一 finished。
- [HttpTransport](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-webapi/src/transport.rs)和[串流恢复适配](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/src/session/api/token_recovery.rs)复用现有 HTTP、会话与信令。明确 Bearer 401 后恢复并重放一次，保持 method、headers、body；有总预算的 transport 将恢复和重放纳入同一预算。POST 命令依据明确 401 拒绝执行这一次重放。
- [数据会话解析](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/session_resolver.rs)接入目标恢复，[数据错误](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/error.rs)将安全文案和原始错误类别送至既有 RPC envelope。主机、云目录、个人数据与主机命令共用错误分类，账户/目录切换后的旧结果支持重新查询。
- [登录页](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/Login.vue)区分恢复重试与重新登录；[shell 登录选择](/Users/guo.xu/Documents/code/games/xbxrc/src/components/layout/use-auth-recovery.ts)通过既有确认弹层统一承接数据页最终鉴权失败，保留原页面跳转目标。串流沿用既有重新登录/退出动作，临时失败可重试原目标。
- 类型检查期间补齐共享 DOM 测试桩的 blur 调用签名。渲染、导航与相册任务的并行编辑保留在当前工作区。

## Validation

| 验证 | 结果 |
| --- | --- |
| cargo test --workspace --quiet | 418 项通过；包含 Tauri 138 项和 iOS 公共调用 50 项 |
| cargo check --workspace | 通过 |
| cargo fmt --all | 已执行 |
| 前端认证/错误/路由定向回归 | 6 文件、42 项通过 |
| pnpm test | 82 文件、760 项通过 |
| pnpm lint:fix | 通过 |
| pnpm typecheck | 通过 |
| pnpm build | 369 modules，5.42 秒通过 |
| pnpm test:trace-production | 2 项通过 |
| git diff --check | 通过 |

覆盖目标过期与 Web-only 状态、SISU 有效期、403/429/网络分流、一次 401 恢复及重复拒绝、轮换 refresh token 保留、并发成功/失败、注销/取消/账户切换、transfer 协议兼容，以及用户确认前凭据保留。HTTP 回环测试校验完整请求、raw form、重试次数和总预算；token/error trace 测试校验安全详情与取消配对。

前端完整输出：[全量测试](/private/tmp/xbxrc-unified-token-frontend-tests.log)、[类型检查](/private/tmp/xbxrc-unified-token-typecheck.log)、[构建](/private/tmp/xbxrc-unified-token-build.log)。这些临时日志对应本次记录的工作区验证结果。

历史复现固定读取 20348f4c 的诊断源码与测试：17 项通过、1 项按预期暴露旧启动恢复缺口，历史对照通过。该预期失败作为诊断基线保留；当前实现的全量测试结果见上表。

## Risks

- 原故障的 refresh token 已由旧重新登录动作清理；其服务端有效性需以真实续期响应判断。自动验证使用合成凭据和本机 HTTP 回环。
- 实机一次登录跨过 4 小时后的 Home 续期、真实 Microsoft 授权拒绝与设备会话重放，按仓库约定由用户手动验收。
- 诊断失败窗口位于媒体启动前；实际串流的 WebRTC 网络健康、恢复链、media supply、steady supply、submit/present 长尾与帧供给 delta 继续采用独立实机窗口验收。

## Follow-up

- 使用新构建登录一次，间隔超过 4 小时后再次启动同一主机串流，核验后台恢复后可直接开始。
- 在恢复时出现网络故障后，恢复网络并重试，核验登录凭据保留；出现明确授权失效时核验取消与重新登录选择。
- 根据实机 trace 中的目标校验、核心/下游续期阶段与 finished 结果补充跨期验收证据。
