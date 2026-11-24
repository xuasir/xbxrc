# iOS 启动流程对齐 PC

## Status

- Completion: 代码与自动验证完成，实机超时概率复验待用户验收
- Current State: validation-pending
- Owner: 主窗口，单窗口直接执行
- Last Updated: 2026-11-23
- 执行授权：用户明确要求“启动流程完全对齐PC”；沿用 [串流行为对齐 RFC](2026-11-19-ios-streaming-pc-behavior-parity.md) 已确认的公共 Rust 流程与现有 iOS 媒体边界。

## Background

新日志记录四次 Cloud 凭据准备超时（6.342–14.032 秒）。源码确认 iOS 丢弃 OAuth/SISU/Home/Cloud 缓存，每次准备访问都重走 OAuth、device、SISU、Web XSTS、Streaming XSTS、Home、Cloud。PC 复用有效 Web 与目标串流令牌，目标失效后优先复用 SISU；401 执行一次核心恢复，OAuth 轮换先保存，再继续下游请求。

iOS 授权与云商品复核发生在 Rust startup scope 建立之前；Home 上下文增加 8 秒 Smartglass 查询。两端的唤醒、创建、排队、信令和清理已共用 `SessionFlowService`，本轮补齐进入公共流程之前的差异。

## Goal and Scope

1. Web/Home/Cloud 共用 PC 的 60 秒有效期余量、JWT/时长过期判定、SISU 复用、目标续期、一次 401 核心恢复和同账户合并规则。
2. iOS Keychain 保存完整凭据缓存，兼容已有四字段会话；核心轮换立即提交，后续失败与取消保留已经保存的新凭据。
3. 授权、商品复核与上下文解析使用同一 Rust startup scope 和 PC 的 90 秒上下文预算；取消、替换与账户撤销吸收迟到结果。
4. Home 沿公共跨区发现、主机事实、唤醒、就绪等待和创建重试；Cloud 在创建前确认启动身份与已知游玩资格。
5. 错误携带稳定分类与重试语义，认证步骤记录脱敏耗时，继续保留现有 HTTP 建连/请求/总预算。

公共协议与认证续期保持 Rust，Swift 负责 Keychain 提交、UI 投影及现有 libwebrtc/Metal 动作。保留工作区已存在的 UI、显示、诊断和测试修改。

## 差异与实施合同

| 阶段 | PC 基准 | 本轮 iOS 实施 |
| --- | --- | --- |
| 热凭据 | 有效 Web 与目标令牌直接使用 | 持久缓存与共享纯 policy |
| 目标续期 | 有效 SISU，按 Web/Home/Cloud scope 执行 | 共享 renewal 编排与独立目标 |
| 核心恢复 | 一次 refresh grant；轮换先保存 | UniFFI checkpoint 回调与代次校验 |
| 并发与失败 | 串行合并；短失败缓存；临时故障保留凭据 | 现有 coordinator 与 session credential owner 接线 |
| 上下文 | 开始前建立 scope，90 秒 | 授权到 Plan 贯穿同一 scope |
| Home 发现 | 公共区域发现，300 秒区域偏好缓存 | 去除启动前额外列表等待，保留主机身份与电源事实 |
| Cloud 商品 | 确认 title 与 entitlement_known/has_entitlement | 相同创建前准入条件 |
| 会话后续 | 公共唤醒/创建/排队/信令/清理 | 继续消费公共 flow，保留小栈包装 |

## Plan

1. 保存修改前基线，抽取共享 token policy 与续期编排，接回 PC。
2. 扩充 iOS 缓存与 checkpoint，接通准备授权、Web 恢复及会话绑定续期。
3. 统一 startup scope、Home 上下文和 Cloud 准入，补齐取消及错误收口。
4. 运行 Rust、Swift、PC 回归与 UniFFI 小栈门，生成 bindings，完成 iPhoneOS 构建。
5. 归档源码与验证证据，清理本轮隔离临时目录，登记实机验收清单。

## Validation

- [x] 有效缓存零授权请求；目标续期覆盖 Web/Home/Cloud 作用域。
- [x] SISU 复用、401 单次恢复、JWT 较早过期、同账户合并与跨账户隔离。
- [x] 核心 checkpoint 先于下游；下游超时保留新 refresh token；Keychain 失败阻止继续使用。
- [x] 授权期间取消、队列取消、账户撤销及重试吸收迟到结果。
- [x] 统一上下文预算、Home 发现/待机唤醒合同回归、Cloud 未知与拒绝资格。
- [x] Rust fmt/check/test，PC 认证与启动回归，Swift 定向测试、生成 bindings、iPhoneOS 构建与边界检查。
- [x] 保留 8 KiB future 构造预算与 544 KiB 调用线程测试。
- [ ] 真实 Wi-Fi 裸连：热启动、令牌失效、待机唤醒、失败重试与取消由用户实机验收。

## Risks

Keychain 新字段采用可选缓存，旧会话首次按目标补齐。checkpoint 在 coordinator 当前事务内提交，保持原串行边界；账户撤销继续等待存储完成后删除。UniFFI 重型 future 继续显式装箱。新诊断只记录固定步骤、耗时、分类和状态码。

## Progress

- [x] PC/iOS 认证和启动源码、既有授权与工作区基线核对。
- [x] 共享 policy、renewal 和 PC 接线。
- [x] iOS 缓存、checkpoint、scope 与启动接线。
- [x] 自动验证与证据归档。
- [x] 本轮隔离临时产物清理。

## 实施结果

共享 Rust 认证 policy 与续期编排已同时接入 PC 和 iOS。iOS Keychain 保留 OAuth、SISU、Web/Home/Cloud 缓存及 Cloud 授权地区；旧四字段会话继续解码。核心轮换与完整目标结果在当前 coordinator 事务内 checkpoint，后续失败和取消保留已经保存的凭据；会话 401 与 Web 唤醒恢复回到同一认证 owner。

四个生产启动入口从授权之前建立同一 `XboxStreamStartup`。授权、Cloud 商品复核与 Plan 解析共用 90 秒上下文预算；Home 使用公共跨区发现、300 秒地区偏好缓存和 2 秒主机事实缓存，唤醒与就绪消费本轮事实。错误由共享 Rust 分类投影到 Swift 终态与页面重试动作。

UniFFI 0.32 的异步 foreign 回调在 C 入口逐参数消费 RustBuffer，转换为 `Result<Sendable Swift 值, Error>`；Task 捕获 Sendable 闭包，保留原 completion 与 dropped/cancel 合同。修复纳入绑定生成脚本，重复生成逐文件哈希一致。真实 Swift→Rust 回归覆盖 Web/Home/Cloud 热缓存、异步 checkpoint 完成、checkpoint 错误返回和 foreign future 句柄释放。

Rust 339 项、iOS 定向 XCTest 102 项均通过；iPhoneOS Debug 构建、Rust check/fmt、会话边界、绑定复现与 diff 门禁通过。源码、测试结果和构建失败诊断见[本轮证据](../../dev-docs/validation/2026-11-23-ios-startup-pc-parity/README.md)。实机连接成功率、真实待机唤醒与媒体/显示健康由新构建的 Wi-Fi 裸连日志验收。

## Execution Notes

- 2026-11-23 实机接续：`472B306E` 导出包含 78 行、一个会话。Web 热缓存实际复用，恢复 89ms；Cloud 准备两次在 deviceToken 网络失败，实际串流启动一次在 SISU 5011ms 超时。PC/iOS 均经 `AuthFlow::new -> AuthApi::new -> HttpTransport::for_commands`，连接/单次/整体请求预算分别为 5/12/18 秒。observer 已补齐固定 `networkErrorKind`，Cloud 外层诊断直接消费类型化 Query 的 kind/status/retriable；请求配置沿公共实现。Rust bridge 93 项与 iOS 定向 37 项、iPhoneOS 构建、小栈、格式、边界和绑定复现通过；测试模块补齐 `home_host_facts` 导入。新导出在内层失败事件结束，具体网络原因、外层终态与媒体验收继续等待完整 trace。[新日志与接续修复证据](../../dev-docs/validation/2026-11-23-ios-startup-pc-parity/fresh-trace-472b306e/README.md)。
- 2026-11-23：修改前基线保存到 `/private/tmp/xbxrc-ios-startup-pc-20261123-cfnj_z17/baseline`；文件 hash 和本轮清理范围登记在证据目录。共享构建缓存继续保留。
- 2026-11-23：101 份修改前文件基线与原暂存差异哈希完成核对，原暂存内容一致。代码与自动验证完成，任务进入实机验证阶段；Report 在实机复验收口后补齐。
- 2026-11-23：构建、测试与导出进程已结束；约 0.893 GiB 本轮临时产物已清理，磁盘可用空间增加约 0.763 GiB。55 份最终源码快照、独立差异、验证摘要和失败诊断留存证据目录；共享构建缓存继续保留。
