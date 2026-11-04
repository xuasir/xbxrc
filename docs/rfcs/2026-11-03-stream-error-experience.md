# 串流错误体验 P0 / P1 / P2 RFC

## Status

- Completion: 实现与本地验证完成；真实串流和物理手柄由用户验收
- Current State: completed
- Owner: 主窗口直接执行
- Last Updated: 2026-11-04
- Authorization: 用户在审阅规划后要求“解决P012”，已授权完整实施。

## Background

[规划](../isu/2026-11-03-stream-error-experience-plan.md)已确认错误身份丢失、翻译文本参与分类、NAT 固定归因、两套终态界面、现场信息释放、等待与局部操作反馈缺口。本轮以 main Browser 串流为基线完成全部阶段。

## Goal

- 错误正文使用可理解的本地化解释，分类、阶段与动作资格保持一致。
- 统一终态卡，提供清理 / 重试 / 登录 / 返回 / 复制反馈和可靠的手柄交接。
- 等待与恢复采用低干扰提示，局部操作失败保留草稿与当前有效状态。

## Scope

修改 `src/streaming` 的错误投影、view-state 与执行交接，`Stream.vue` 与现有 stream sheets，中英文词条及对应测试。复用 Rust 公共错误 DTO、single-flight、取消与清理、runtime 恢复、主题、几何导航和输入所有者。

媒体与网络策略继续由现有执行路径决定；本轮通过页面投影完善错误体验。共享 Rust DTO 沿用当前合同。

## Plan

1. P0：保留 category / retryable / phase；Browser 原始代码规范化；未知错误使用稳定正文；失败前固定安全字段快照。
2. P1：复用 StreamAlertSheet 形成统一错误卡；主次动作与详情 / 复制；菜单接管、连续 UI 捕获与正确焦点；清理未完成限制重试，显式返回随时可用。
3. P2：首帧 / 播放停滞使用现有 60 秒警告期限；持续恢复沿现有监督与生命周期事实显示被动状态；主动查看问题后可继续等待或结束。
4. P2：文本、显示保存、麦克风、截图、关机、全屏与超分操作逐类补齐局部反馈、处理中、迟到结果隔离与重试。

## Decisions

- 后端 retryable 决定默认重新连接资格；主机注册耗尽 / 主机准备 / target 错误提供返回与检查指引。自动补偿预算继续沿原合同。
- 登录恢复先完成旧执行清理，再沿现有 auth/logout 与 login redirect 流程恢复认证；原目标从现有路由重入。
- 终态卡遮罩保持当前界面；B/Esc 对应返回列表。详情展开时返回先折叠详情。等待卡返回恢复游戏输入。
- 错误快照采用安全字段白名单，技术标识只保留已知枚举 / 内部代码；原始字符串、账号、URL、SDP、ICE 与目标身份继续留在现有日志层。
- 当前执行 reducer 保存终态错误与阶段；页面临时操作和弹层由现有 page UI owner 管理。

## Validation

- [x] 错误身份、阶段、动作、未知兜底与脱敏字段矩阵
- [x] 失败快照穿过清理、清理 pending / 重试 / 导航 / 登录闭环
- [x] 单卡、旧事件隔离、菜单接管与连续输入所有权
- [x] 首帧等待、播放停滞、frameReady 解除、同一故障等待 latch
- [x] 局部操作失败保留状态、重复请求与迟到结果隔离
- [x] 前端 lint:fix / typecheck / test / build / diff check
- [x] 窄窗口 / 深浅主题 / 长文案视觉检查，真实串流与物理手柄按仓库约定由用户验收

## Risks

- auth router 的 guestOnly 会依据缓存身份重定向；重新登录动作需显式退出旧身份并保留原目标 redirect。
- closeExecution 撤销输入和清空资源与错误卡接管存在时序竞争，终态 UI 捕获应在资源清理期间持续有效。
- 单次 frameReady 与持续输出分别处理计时，展示监督复用现有期限并保持恢复策略边界。

## Progress

- [x] P0 错误身份与快照
- [x] P1 统一卡片、动作与输入交接
- [x] P2 等待与局部操作反馈
- [x] 验证与报告

## Execution Notes

- 2026-11-03：按用户授权进入实现；当前 checkout 除规划文档外保持干净。

- 2026-11-04：P0/P1/P2 实施完成。统一错误卡接管终态；清理 gate、导航与登录代次、关机迟到结果、局部反馈和回滚闭环通过验证。
- 2026-11-04：`pnpm lint:fix`、`pnpm typecheck`、55 文件 574 项测试、`pnpm build`、`git diff --check` 通过。真实 Vue + 模拟执行/RPC 验证中文错误卡、英文浅色窄窗口、低高度详情、复制、文本草稿、显示保存、等待、导出和关机失败。
- 实施证据与实机验收项见 [Report](../reports/2026-11-03-stream-error-experience.md)。
