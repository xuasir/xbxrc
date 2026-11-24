# Wi-Fi 裸连与待机主机连接失败对照

结论：本轮 Wi-Fi 裸连已完成一次主机串流授权，之后的远程会话准备在 54.837 秒后以 `session.failed` 收敛；紧接着的重试在授权阶段 6.782 秒后超时。阶段与终态有证据，具体会话底层错误需要补充诊断。

## 范围

输入：`/Users/guo.xu/Downloads/XBXRC-Diagnostic-33399546-02B7-40A6-AAF9-BCFA84B28947.jsonl`。共 171 行、81,801 字节、1 个 session；Asia/Shanghai 时间范围 2026-11-23T11:59:41.035000+08:00 至 2026-11-23T13:46:08.789000+08:00。traceProfile 为 production；应用标识为 0.1.0 / build 1 / debug。

用户确认 13:44–13:46 使用 Wi-Fi 裸连，Xbox 主机处于待机。前 90 行原始字节的 SHA-256 与上轮输入一致，新导出追加 81 行。上轮对照使用已归档 hash 与锚点，新输入包含完整旧前缀。

## Gate

基础 trace gate PASS：schema、sequence、startup/library coverage、pairing、privacy 与 budget 违规均为 0。串流成功合同 gate FAIL，共 26 项缺失锚点；涉及 sessionReady、完整 peer epoch、playing、末尾尚待收口的 generation=5，以及前置失败场景的 cleanup。资源结论以实际创建与收口记录为依据。

## 最小时间线

| seq | 上海时间 | 域 / 事件 | 事实 |
| --- | --- | --- | --- |
| 91–94 | 12:17:58.517–12:17:58.519 | auth/ios-streaming / homeAccessBoundaryFailed → accessPrepareFailed → terminalSelected → failed | 补齐上轮 seq=90 后的终态，retryable=true |
| 100–104 | 12:18:03.290–12:18:03.293 | auth/ios-streaming / homeAccessUniFFIBoundaryFailed → failed | generation=2 主机访问授权 797 ms 后 connection failed |
| 124 / 130 | 12:21:59.742 / 12:22:34.409 | auth / cloudAccessUniFFIBoundaryFailed | 云串流授权分别 8584 / 6605 ms 后请求超时 |
| 147 | 13:44:43.154 | xbox-data / hostsRefreshSucceeded | 主机列表 1798 ms 后刷新成功，1 台主机 |
| 149–152 | 13:44:46.082–13:44:46.131 | ios-streaming/auth / streamLaunchStarted → homeAccessUniFFIBoundaryStarted | generation=4，target=home |
| 153–155 | 13:45:04.044–13:45:04.129 | auth/ios-streaming / homeAccessUniFFIBoundarySucceeded → accessPrepareSucceeded | UniFFI 授权 17913 ms，外层 17999 ms；从启动到授权完成 18047 ms |
| 156–157 | 13:45:04.134 | ios-streaming / creatingSession → sessionCreateStarted | 进入远程会话准备 |
| 159–161 | 13:45:58.971 | ios-streaming / sessionTerminal → terminalSelected → failed | 54.837 秒后 reason=session.failed，retryable=true |
| 162–165 | 13:45:58.990 | ios-streaming / presentationDetached → iceTasksCancelled → remoteSessionClosed → accessReleased | 记录收口动作 |
| 166–170 | 13:46:01.981–13:46:02.002 | ios-streaming/auth / 新 streamLaunchStarted → homeAccessUniFFIBoundaryStarted | generation=5 重试 |
| 171 | 13:46:08.789 | auth / homeAccessUniFFIBoundaryFailed | 6782 ms 后 request timed out，导出在本行结束 |

完整 operationId、attemptId、tsMs、脱敏 payload、源码 hash 和逐尝试摘要见 `analysis.json`。本轮成功授权后失败的 attemptId 为 `A1FC94DA-D14F-4FF4-8E40-99BD0E3B925C`。

## 覆盖、预算与隐私

基础启动、目录与条件配对 gate 通过，完整行解析与隐私违规为 0，压力通知为 0。输入是聚合导出；writer 声明 8 MiB / 4 files，物理保留数待原始 writer 目录核验。

网络维度显示主机列表成功、主机串流授权一次成功和多次连接失败/超时。恢复链、media supply、steady supply、submit/present 长尾与帧供给 delta 的指标需要媒体接入与播放后形成。当前阶段停在 sessionReady 前，网络、媒体与展示指标的健康结论按适用阶段解释。

## 源码核对与原因边界

用户已确认“连接时唤醒主机”的保存值为开启，PC端可连接。实际偏好经 `StreamingRuntime.swift:752` 透传到 Rust；公共编译器根据它决定 wake_console 和 require_console_ready。该导出中的实际请求配置与唤醒结果需要更细的trace核验。

Swift 的 sessionCreateStarted 包括 Rust 控制面上下文、主机跨区域发现、条件唤醒和公共会话启动。`XboxStreamSession.start` 先准备控制面上下文，再启动公共 flow。跨区域发现单请求预算 5 秒、总预算 30 秒；启动与主机就绪预算为 90 秒。本次 54.837 秒为观测耗时，具体失败步骤待确定。

Rust 将 SessionFlowError 映射为 Http 或 Remote。Swift `StreamSessionActor.fail` 使用通用 session.failed 收口；该诊断链记录了终态类别，细分错误、主机电源事实与启动进度仍待落入 trace。当前源码通过 hash 归档，应用二进制的源码身份待更细 build 标识确认。

可确认的阻断点：generation=4 远程会话准备失败；generation=5 主机访问授权请求超时。待机主机的唤醒/就绪与请求网络路径均属于下一步排查项。

## 下一步

已完成[启动诊断补齐与编译验证](../startup-diagnostics-fix/README.md)：实际请求的powerOn、控制会话构造、信令准备边界、既有Rust启动进度与固定错误分类进入trace；10项诊断测试和iPhoneOS构建通过。两端认证策略差异与故障证据边界见补丁记录。

使用更新后的iOS构建，在相同Wi-Fi裸连、主机待机和唤醒开启条件下重试并导出日志。下一轮以最后启动阶段和结构化错误确认具体阻断，主机电源与唤醒命令的更细事实按需要接续核对。

## 归档与清理

该日志分析轮业务源码编辑为 0，构建与实机操作为 0。检查通过内联脚本执行，临时路径与清理释放量均为 0。保留新 Downloads 输入；最终诊断、gate、对照、锚点、用户澄清与源码 hash 保存在本目录。接续代码修复的验证与清理单独记录在 `startup-diagnostics-fix`。
