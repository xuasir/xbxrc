# 统一 token 自动续期与重新登录选择

## Status

- Completion: 代码与自动验证完成，实机待验
- Current State: validation-pending
- Owner: 主窗口，单窗口直接执行
- Last Updated: 2026-11-09
- 执行依据：用户已明确要求统一 token 处理，可续期时自动刷新，续期失效时提供重新登录选择。

## Background

[排查证据](../../dev-docs/validation/2026-11-09-xhome-auth-refresh-investigation.md)确认 Web-only 身份使启动恢复跳过、串流入口只读缓存、失败后重新登录动作清理核心凭据。旧逻辑对照通过，当前启动恢复目标行为失败。

## Goal

所有桌面 token 消费入口采用相同规则：有效凭据直接复用；有效 SISU 继续派生下游；需要核心续期时使用 refresh token；网络类失败保留凭据并支持重试；明确刷新授权失效后让用户选择重新登录。

## Scope

- Rust auth 的有效期、目标范围、串行恢复、代次保护、落盘与错误分类。
- Web 查询、Home/Cloud 串流准备、会话与信令 HTTP 401、cloud 目录和 transfer token 的凭据消费。
- 现有 Vue 登录/串流错误操作，以及主机、云目录、个人数据共用的重新登录选择；数据 RPC 保留鉴权错误类别与安全详情。
- 复用当前 `xbox-webapi`、`xbox-auth-flow`、`xbox-streaming`；公共库现有 iOS 调用保持兼容。

## Plan

1. 建立 Web/Home/Cloud 的统一目标校验和恢复规则；目标失效绕过基于其他 token 的缓存短路。
2. 将 OAuth refresh grant 错误与下游鉴权区分；下游 401 可以触发核心恢复一次；有效 SISU 复用依据真实有效期。
   - 本地过期允许复用有效 SISU；Web/Home/Cloud 接口明确 401 时通过 refresh grant 重建认证链，网络中断后的重试沿用对应目标的核心恢复路径。
3. OAuth 续期成功后及时保存轮换的 refresh token；后续失败保留最新核心凭据，提交保持 generation/账户边界。
4. token 消费改为异步确保有效；安全查询和明确拒绝的 HTTP 请求仅恢复并重试一次；临时恢复失败保持真实错误分类。
5. 补齐生产 trace 安全范围与结果，验证恢复成功、暂时失败、授权失效和并发/取消路径。
6. 数据请求透传原始错误类别，shell 使用现有确认弹层提供取消与重新登录；用户点击重新登录后清理凭据并保留原页面跳转目标。账户或目录切换导致的旧请求返回可重试错误。

## Validation

- [x] 过期 Home/有效 Web、过期 Home/有效 Cloud、过期 Web/有效 SISU。
- [x] 下游 401 恢复一次、重复拒绝、invalid_grant、网络/限流/权限错误。
- [x] 轮换 refresh token 遇下游失败保留、并发请求共用恢复、退出取消与账号切换。
- [x] 串流和数据 UI 对最终鉴权失效提供重新登录选择；临时恢复失败提供重试。
- [x] Rust 定向与相关包回归、cargo fmt/check；前端定向/全量、lint:fix/typecheck/build；trace 隐私与文档检查。
- [ ] 实机一次登录跨过 4 小时后再次启动主机串流由用户验收。

## Risks

- 请求恢复保留一次重试上限，POST 仅在明确 401 拒绝后重试。
- 目标刷新保留其他目标的有效缓存，账号切换替换账户凭据。
- OAuth 轮换落盘受 generation 保护；设备实机跨有效期结果继续单独验收。

## Progress

- [x] 明确范围与规则，用户确认执行。
- [x] 公共恢复规则与协议编排。
- [x] 所有桌面消费入口接入。
- [x] 自动验证与代码交付收口。

## Execution Notes

- 2026-11-09：基于已定位缺口直接实施用户确认的统一续期规则；保留已有渲染/导航任务编辑，主线构建入口按仓库约定保持暂停。
- 2026-11-09：Rust 全量 418 项、前端 82 文件 760 项、定向 42 项及编译/格式/lint/类型/构建/生产 trace 检查通过。[交付报告](/Users/guo.xu/Documents/code/games/xbxrc/docs/reports/2026-11-09-unified-token-renewal.md)记录实现、验证与实机验收边界。
