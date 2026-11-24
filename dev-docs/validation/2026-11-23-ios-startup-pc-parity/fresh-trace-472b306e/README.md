# 新实机日志：Cloud 授权网络失败与诊断接续

输入：`/Users/guo.xu/Downloads/XBXRC-Diagnostic-472B306E-7D42-452E-AC23-844411487EBF.jsonl`。一个会话、78 行、38,665 字节，schema v3 / production，时间为 2026-11-23 17:47:09.155–17:49:45.029（UTC+8），跨度 155.874 秒。原始文件保留于 Downloads，仓库保存安全锚点、分析和源码证据。

## 诊断结论

Web 热缓存已实际复用，登录恢复耗时 89ms。Cloud 访问准备两次在 deviceToken 请求出现可重试网络错误；一次实际串流启动在 deviceToken 成功后，SISU 授权请求于 5011ms 超时。最早异常位于 seq 38 的 deviceToken 请求，失败发生在 Cloud 会话创建之前。

PC 和 iOS 都沿 `AuthFlow::new → AuthApi::new → HttpTransport::for_commands`，配置为连接 5 秒、单次请求 12 秒、整体请求 18 秒；startup 上下文另有 90 秒预算。SISU 5011ms 高度吻合连接限时。DNS、TCP、TLS 或响应体等具体错误类别在当前 iOS observer 投影中丢失，下一轮由新增字段定位。

PC 成功来自用户此前的使用反馈。本轮文件包含 iOS 一次实际串流启动；当前网络是否始终为 Wi-Fi 裸连、后台期间是否切换网络，以及导出时 App 的最终状态，等待用户补充。

## 门禁

- schema、序列、隐私、预算压力及必需 App/目录锚点检查通过；seq 1–78 连续，格式损坏、隐私违规和压力通知均为 0。
- iOS flow gate 为 FAIL：seq 65 `cloudAccessBoundaryStarted` 缺同 operationId 的外层成功/失败事件。导出最后一行是 seq 78 `cloudAccessUniFFIBoundaryFailed`，末行 JSON 完整；App 最终状态由后续证据确认。
- streaming core gate 为 FAIL，8 项均为会话、完整 peer epoch、playing、terminal 及清理锚点缺失。该尝试停在授权阶段，媒体与清理路径的执行情况仍缺证据。
- 文件属于聚合导出。单个 writer 文件及轮转保留数量由原始文件目录核验；此导出未记录压力事件。

## 最小时间线

| 场景 | OAuth / checkpoint | deviceToken | SISU | 终态锚点 |
| --- | --- | --- | --- | --- |
| Web 恢复 | seq 9 复用 Web，seq 12 恢复成功 89ms | 热路径复用 | 热路径复用 | seq 12 |
| Cloud 准备 1，17:47:13 | 615ms，seq 35 核心轮换保存 | seq 38，156ms 网络失败 | 前置请求失败 | seq 40–42，外层误报 requestFailed / retriable=false |
| Cloud 准备 2，17:47:21 | 564ms，seq 51 核心轮换保存 | seq 54，132ms 网络失败 | 前置请求失败 | seq 56–57，同样误报 |
| 实际串流启动，17:49:33 | 1251ms，seq 71 核心轮换保存 | seq 74，4839ms 成功 | seq 76，5011ms 超时 | seq 78 内层失败，合计 11117ms；外层终态缺证据 |

前两轮是启动之前的 Cloud 访问准备；`streamLaunchStarted` 出现一次，位于 seq 63。三轮核心 checkpoint 均已记录，后续认证失败仍保留已保存的轮换凭据。

## 诊断修复

`CloudLibraryDiagnostics` 原来从 `NSError.localizedDescription` 推导总类和可重试性。实际错误为类型化 `XboxBridgeError.Query(.network, retriable=true)`，外层写成 `requestFailed / false`。现在直接消费 Query 的 kind、status、retriable 和冷却事实，保持原额外上下文字段。

`XboxAuthStage` 增加固定 `networkErrorKind`，Rust observer 仅传递 dns/connect/tls/timeout/body/request/unknown；Swift 认证 trace 保留该字段及原步骤、耗时和总类。URL、凭据和 source 自由文本继续留在 HTTP 边界内。

## 验证

Rust bridge 93 项、iOS 定向 XCTest 37 项通过，覆盖六类网络错误、自由文本隔离、Query 网络/超时/限流与可重试性、真实 Swift→Rust 回调、失败后凭据保留，以及页面终态和重试。iPhoneOS Debug 构建成功；8 KiB future 构造门、544 KiB FFI 调用栈、格式、串流边界、diff 和绑定重复生成哈希检查通过。暂存区基线哈希一致。

全量 Rust 回归首次发现现有主机发现测试缺少 `home_host_facts` 导入。测试模块补齐导入后 93 项通过，初次编译失败与最终结果均归档。模拟器与结果提取工具所需的系统服务/报告缓存访问经自动审核通过。

## 六维健康证据

| 维度 | 状态 | 证据 |
| --- | --- | --- |
| 网络健康 | FAIL | 两次 deviceToken 网络失败，一次 SISU 超时 |
| receive 恢复链 | INSUFFICIENT_EVIDENCE | 授权阶段结束，peer 与媒体锚点缺失 |
| media supply | INSUFFICIENT_EVIDENCE | 会话创建之前失败 |
| steady supply | INSUFFICIENT_EVIDENCE | 会话创建之前失败 |
| submit / present 长尾 | INSUFFICIENT_EVIDENCE | 会话创建之前失败 |
| 帧供给 delta | INSUFFICIENT_EVIDENCE | 会话创建之前失败 |

## 证据与接续

- [事件与原因分析](incident-analysis.json)、[安全时间线](safe-timeline.json)、[iOS 门禁](ios-analysis.json)、[串流门禁](core-analysis.json)、[运行摘要](runtime-summary.txt)
- [自动验证](validation.json)、[模拟器摘要](simulator-summary.json)、[设备构建摘要](device-summary.json)、[绑定复现](bindings-reproduction.json)
- [基线](baseline.json)、[本轮独立差异](task-changes.diff)、[源码哈希](source-hashes.json)、[暂存区核验](staging-preservation.json)，`sources/` 保存最终源码与公共请求配置
- `logs/` 保存构建、测试及初次失败输出；[临时产物清理](cleanup.json)记录清理范围、释放量和保留项

安装本轮代码的新构建，在同一 Wi-Fi 复测 Cloud 授权并导出完整失败日志。新增网络细分类别、外层终态和账户/启动代次锚点用于下一轮定位；Home 待机唤醒和实际串流六维健康沿原实机验收清单继续确认。
