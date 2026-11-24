# 主机串流授权连接失败诊断

结论：本次主机串流在授权准备阶段被上游认证网络连接失败阻断。最后一个 Rust/UniFFI 边界耗时 1182 ms，返回 `RetryableAuthentication("Network error: connection failed")`。

## 范围与 gate

输入：`/Users/guo.xu/Downloads/XBXRC-Diagnostic-01B7BE96-21B2-47D5-8A3D-5B65F95CF37B.jsonl`。共 90 行、1 个会话、42046 字节；Asia/Shanghai 时间范围为 2026-11-23T11:59:41.035000+08:00 至 2026-11-23T12:17:58.516000+08:00。应用标识为 0.1.0 / build 1 / debug，traceProfile 为 production。

基础 trace gate 为 PASS：schema、sequence、startup/library coverage、配对、隐私与预算违规均为 0。串流成功合同 gate 为 FAIL，8 项涉及 sessionReady、完整 peer epoch、playing、terminalSelected 与 cleanup 锚点。前置授权失败解释会话建立锚点缺失；日志末尾的外层收口需要原始后续记录确认。

## 最小时间线

| 行 / seq | 上海时间 | 域 / 事件 | 事实 |
| --- | --- | --- | --- |
| 10 | 11:59:47.289 | auth / uniffiBoundaryFailed | renewLogin 6196 ms 后请求超时 |
| 12–13 | 11:59:47.386 | auth / authSessionFallbackSucceeded → authRestoreSucceeded | 保存凭据回退成功，UI 登录态为 signedIn |
| 19 | 11:59:47.406 | cloud-library / cacheRestoreHit | 恢复 2865 项目录缓存 |
| 20 | 11:59:49.406 | xbox-data / hostsRefreshSucceeded | 主机列表请求成功，1 台主机 |
| 30–32 | 12:00:05.508–12:00:05.510 | auth/cloud-library / cloudAccessUniFFIBoundaryFailed → catalogRefreshFailed | 云串流访问准备 13743 ms 后请求超时，目录继续展示缓存 |
| 36 | 12:00:33.584 | xbox-data / libraryContentCommitted | 个人游戏库请求成功，189 项 |
| 85–86 | 12:17:56.999 | ios-streaming / preparingAccess → streamLaunchStarted | target=home，generation=1 |
| 87–89 | 12:17:57.331–12:17:57.332 | ios-streaming/auth / accessPrepareStarted → homeAccessBoundaryStarted → homeAccessUniFFIBoundaryStarted | 进入主机串流访问凭据准备 |
| 90 | 12:17:58.516 | auth / homeAccessUniFFIBoundaryFailed | 1182 ms 后 connection failed |

完整 operationId、tsMs 和脱敏 payload 见 `analysis.json`。最后一个 UniFFI operationId 为 `54D802CA-D12D-450C-836C-152160DBF53E`；启动 attemptId 为 `E6745030-6EB4-4EF7-93CE-855A91A4E0B1`。

## 覆盖、预算与隐私

startup/library 的关键锚点齐全。网络维度呈现认证请求超时/连接失败，同时主机列表与个人游戏库请求成功。恢复链、media supply、steady supply、submit/present 长尾与帧供给 delta 的观测窗口需要授权完成、建连与播放后产生。

本文件为聚合导出，42,046 字节；writer 声明生产配置为 8 MiB / 4 files。压力通知为 0，完整行解析与隐私 gate 均通过；原始 writer 目录的物理保留数待核验。

## 原因与源码核对

`crates/xbox-ios-bridge/src/cloud_access.rs:186` 的 `prepare_home_access` 调用共享 `AuthFlow::refresh_and_finalize` 准备下游串流凭据；`XboxCloudDataClient.swift:174` 将边界错误记入 trace 后继续向外抛出；`StreamingRuntime.swift:242` 在 prepareAccess 返回成功后进入会话启动。因此失败阶段可以锁定在串流前的认证网络请求。当前 `xbox-webapi/src/error.rs:32` 将固定消息 `connection failed` 映射到 Connect。

保存凭据回退和游戏目录缓存解释了 UI 可以维持登录态并显示内容；主机串流另外需要当次访问凭据。具体认证步骤、目标端点类别与连接原因需要请求级脱敏记录。当前源码身份通过 SHA-256 归档；导出中应用 build 标识的粒度仅到 0.1.0 / 1 / debug。

发现诊断字段偏差：Rust 错误枚举为 `RetryableAuthentication`，Swift trace 写入 `retriable:false`。当前 `CloudLibraryDiagnostics.isRetriable` 依靠文本、NSError、超时和 HTTP 状态推断，连接错误落到 false；当前串流准备阶段的业务状态支持用户主动重试。该诊断字段需要按类型化错误投影。

## 证据缺口与下一步

导出结束在 seq=90 的 UniFFI 失败记录。外层 `homeAccessBoundaryFailed`、`accessPrepareFailed`、`terminalSelected` 和 failed 状态需要后续原始 writer 文件确认。会话资源生命周期也需要以实际创建和终态记录为依据。

建议先切换手机网络，验证 Xbox 认证服务连通性后重试；随后补齐脱敏认证步骤、端点类别、networkErrorKind、类型化 retriable 和外层失败收口。

## 归档与清理

本轮直接读取输入并通过内联脚本执行分析，临时产物为 0、释放量为 0。原始 Downloads 输入保留，最终诊断、gate 输出、脱敏锚点和源码 hash 保存在本目录。业务源码保持本轮开始时的状态。
