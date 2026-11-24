# iOS 启动流程对齐 PC：代码与自动验证证据

任务：[启动流程对齐 RFC](../../../docs/rfcs/2026-11-23-ios-startup-pc-parity.md)。代码与自动验证完成，实机网络和连接成功率进入用户验收。

## 交付

- PC 和 iOS 消费同一 Rust 凭据有效期与续期实现：60 秒余量、较早过期时间、SISU 复用、目标 scope、一次核心恢复和同账户缓存合并。
- iOS Keychain 保存完整凭据缓存并兼容旧会话；核心轮换先 checkpoint，后续超时、取消保留已保存结果，存储失败终止继续使用。会话 401 与 Web 唤醒恢复沿同一认证 owner，保持并发合并与 1 秒会话续期失败缓存。
- 四个生产入口在授权前创建同一 startup，授权、商品复核和 Plan 共用 90 秒预算。Home 沿公共发现、唤醒、就绪和创建流程，Cloud 在创建前核验启动身份和已知资格。
- Rust 领域错误保留 kind/status/retryable，Swift trace 与页面消费同一结果。认证请求记录固定步骤、耗时和安全分类。
- UniFFI foreign async 回调在入口消费 FFI 缓冲区，Task 捕获 Sendable Swift 值；生成脚本可复现，原完成和取消合同保留。

## 自动验证

| 范围 | 通过 | 失败 |
| --- | ---: | ---: |
| 共享认证 | 21 | 0 |
| iOS Rust bridge | 91 | 0 |
| PC 认证 | 30 | 0 |
| PC 串流投影/启动 | 18 | 0 |
| 公共串流 | 179 | 0 |
| iOS 定向 XCTest | 102 | 0 |

iPhone 17e / iOS 27.0 模拟器执行 102 项 XCTest；iPhoneOS Debug 构建成功，签名由实际安装构建处理。Swift 6 strict concurrency 保持 `complete`。8 KiB future 构造预算和 544 KiB FFI 调用线程回归通过。

三项真实 Swift→Rust 测试使用有效假缓存和无效 seed，验证热路径在解析 seed 与网络请求前完成；强制恢复的 checkpoint 成功/失败从实际 C 回调往返，并核验 foreign future 句柄释放。其余用例覆盖核心轮换后超时/取消、旧 Keychain 兼容、排队取消、账户撤销、Cloud 身份/资格、统一 startup、重试与清理。

Rust check/fmt、会话边界、绑定重复生成哈希和 `git diff --check` 通过。设备构建的两条 Text 拼接弃用警告、Rust check 的三条既有未使用查询函数警告记录在日志中。

## 证据索引

- [验证摘要](validation.json)、[XCTest 结果](simulator-summary.json)、[设备构建结果](device-summary.json)
- [绑定复现](bindings-reproduction.json)、[失败诊断](build-failures.json)、[原暂存内容核验](staging-preservation.json)
- [修改前基线](baseline.json)、[本轮独立差异](task-changes.diff)、[源码哈希](source-hashes.json)、`sources/` 最终源码快照
- `logs/` 保留 Rust、设备构建、模拟器测试与门禁输出；Swift 格式仅作用于本轮修改范围
- [输入与范围](task-input.json)、[清理前清单](cleanup-before.json)；最终清理结果记录在 `cleanup.json`

隔离临时目录已删除，清理约 0.893 GiB 文件占用；磁盘可用空间增加约 0.763 GiB。55 份最终源码快照与必要测试/诊断输出继续保留，原暂存内容哈希一致。详见[清理结果](cleanup.json)和[进程核验](cleanup-process-check.json)。

用户指定的三份 Downloads 日志在最终快照时均已无法从原路径读取；本轮代码任务对这些路径的修改次数为零。既有日志分析结论沿 RFC 背景保留，实机验收使用新构建导出的 trace。

## 实机验收

1. Wi-Fi 裸连连续启动，确认热缓存复用及连接成功率。
2. 旧会话或令牌将过期时启动，确认目标续期、核心 checkpoint 及失败后重试。
3. 主机待机，开启“连接时唤醒主机”，验证唤醒、就绪等待和创建重试。
4. 授权/商品复核期间取消后重新启动，验证迟到结果隔离和资源释放。
5. 从新 trace 核对认证各步骤、启动阶段及实际网络、媒体供给、恢复链和显示长尾。

超时概率改善与实际连接成功率由这轮实机证据确认。

## 17:47 新实机日志与诊断接续

`472B306E` 导出确认 Web 热缓存复用，登录恢复 89ms。三轮 Cloud 准备有两次 deviceToken 网络失败和一次 SISU 5011ms 超时；认证客户端与 PC 共用 5/12/18 秒连接、单次和整体请求预算。网络具体成因需要固定细分类别，导出尾部还缺外层终态。

网络细分类别已穿过 Rust observer；Cloud 诊断保留类型化 Query 的 kind/status/retriable。Rust bridge 93 项、iOS 37 项、iPhoneOS 构建与绑定复现通过，完整连接和媒体健康继续实机验收。详见[新日志与修复证据](fresh-trace-472b306e/README.md)。
