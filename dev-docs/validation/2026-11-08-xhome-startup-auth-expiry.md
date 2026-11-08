# XHOME 启动令牌失效与首错修复验证

- 日期：2026-11-08
- 状态：代码与自动验证完成，本地应用提示和重新登录由用户重启验收。
- 原日志：[runtime-trace-1794143800426-1.jsonl](/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1794143800426-1.jsonl:89)。
- 原日志 SHA-256：`1f6832db8239294c6da831c9c5fadbf20c41f87b1ffd2f7487fa63f33c0d81f3`，222 行 / 116,926 字节，dev profile。

## 原因与时间线

缓存 XHOME 令牌存在，过期时间为 2026-11-08 14:44:04.079；21:16:58 启动时有效性检查返回空值。Web API 登录凭据仍可供资料和主机列表查询，界面因此显示已登录。串流准备上下文直接读取有效串流令牌，随后抛出 `Streaming token is unavailable for home`。

旧回退分类覆盖 `auth` 和 `token missing`；本次 `token is unavailable` 进入 `unknown` 默认分支，失败页显示通用错误。准备上下文阶段的失败记录只写入阶段和文本，后端完整的 `sessionStartFailed` 位于后续会话执行失败分支。前端 `executionFailed` 记录原始异常，同时缺少已解析类型、启动阶段和 attemptId。

| seq | 时间（Asia/Shanghai） | 证据 |
| --- | --- | --- |
| 27–31 | 21:16:46.545–46.561 | 第一次准备上下文失败，同一令牌原因，启动操作耗时13ms |
| 85 | 21:16:58.240 | 第二次启动进入 ResolvingContext |
| 86 | 21:16:58.256 | 配置快照读取完成 |
| 87–89 | 21:16:58.257–58.258 | 令牌不可用，操作17ms，前端收到原始异常 |
| 100 | 21:16:58.265 | 清理6ms，输入/runtime/startup释放结果完成 |
| 101 | 21:16:58.265 | writer flush覆盖seq100，IO/关键行丢失计数为0 |

失败位于创建会话和媒体启动之前，metrics对应暂无媒体样本。

## 交付行为

1. 串流凭据缺失或过期进入 Auth 错误；旧消息格式的回退分类同步识别 `Streaming token is unavailable`。
2. 失败页沿现有流程显示“登录状态已失效，请重新登录后再试”，提供“重新登录”主操作；完成清理后跳转登录页，并保留原串流目标。
3. 准备上下文与会话执行失败共用首错入口，先写 `sessionStartFailed` 的错误类型、原始原因、启动阶段、attemptId/operationId，再发送失败阶段事件和进入清理。
4. 前端 `executionFailed` 在清理前保留 `errorKind/errorCode/startupPhase/retryable/attemptId`；原始错误与首错快照继续保持。

## 验证

- 新增后端分类回归在修复前返回 Unknown，修复后home/cloud及重复Streaming前缀均归Auth。
- 新增前端回归在修复前缺少首错语义字段；修复后验证Auth提示、无媒体样本、一次首错、清理顺序和登录后返回目标。
- `cargo test -p xbxrc --lib`：121项通过；回环HTTP测试在沙箱外运行。
- `pnpm test`：77文件 / 710项通过。
- `cargo fmt`、`cargo check -p xbxrc`、`pnpm lint:fix`、`pnpm typecheck`、`pnpm build`：通过。
- diff和本轮文档链接：通过。

重启当前开发应用，使用现有过期凭据进入XHOME串流，验收登录失效提示与“重新登录”入口；登录成功后继续原串流目标并导出新日志。
