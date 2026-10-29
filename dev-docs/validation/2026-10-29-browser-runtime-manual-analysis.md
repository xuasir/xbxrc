# 2026-10-29 Mac 浏览器运行时手动验收日志分析

用户进一步要求聚焦策略正确性与运行效果，补充审计见[显示/SR/观测策略复核](2026-10-29-browser-render-policy-correctness-audit.md)。已确认解码后SR供给下降、自动输出降采样、内容FPS合同冲突和观测热路径问题；后续优先级以补充审计为准。

## 结论

本次包含两次主机串流失败和一次云游戏启动、播放、前后台切换及正常退出。主机故障位于公共会话准备阶段；云游戏连接和本地资源生命周期具备正向证据，网络、帧供给和恢复后的显示质量仍有明显问题。当前优化任务继续保持待验状态。

- 主机两次创建会话成功，随后持续处于 `Provisioning/pending`，本地约 30 秒注册等待期限触发 `homeServerRegistrationTimeout`。两次媒体运行时均未启动。
- 主机能力快照为 `ConnectedStandby`；计划 `wakeConsole=false`、`requireConsoleReady=false`。唤醒和就绪前置核验是优先检查点。能力 API 的远程管理/串流开关字段为 `missing`，实际开关状态保持未知。
- 云游戏全采样区间累计包损失约 5.00%，RTT 中位数 233ms、P95 1260ms、最大 3357ms。前台首段已经存在冻结和回调长尾；最后恢复到前台后的 24.7 秒进一步恶化。
- 退出时资源计数归零；同样本面板/监督事实一致；GPU 对象数量稳定、重复绘制计数为 0。画质重复评估、Mac 源节拍缺失、显示预算切换及报告时序处理继续待收口。

## 样本与方法

- 日志：`runtime-trace-1793241116241-1.jsonl`，来源 `/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/`。
- 时间：2026-10-29 10:31:56.242 至 10:39:53.227（Asia/Shanghai），476.985 秒、5769 行。
- 构建：macOS debug，Git `bec3acfb`，工作树 dirty，Rust 构建源码摘要 `72caf2c802633e62da8a348d44c5048ad2f40c5b03dce3d847bc89316ea79dad`。
- Schema/profile：v3 / dev；`traceBudgetNotice=0`。本样本缺少浏览器 receive ledger 和原生 submit/present 细节属于观测边界。
- 先执行 `summarize_runtime_trace.py`，再执行浏览器行为报告 `--require-stability` 与 lifecycle reconnect gate，最后回查原始事件和源码。
- 浏览器累计计数按 `sampledAtMs` 排序；窗口指标使用窗口内首末计数差，RTT 分位数使用有效候选对样本。指标见 [计算结果](2026-10-29-browser-runtime-manual-analysis.metrics.json)。
- 约 62.057 秒处于后台，其中连续后台段为 10:38:08.416–10:39:09.817。前后台边界来自前端事件经 RPC 落盘后的时间，存在边界传输误差。
- `xbxengine` 的无 session 空闲快照持续报告 priming / 0 FPS；云游戏媒体事实以浏览器事件为准。

## 主机故障证据

| 尝试 | 最小时间线 | 结果 |
| --- | --- | --- |
| 第一次 | seq 84 能力快照；88 编译计划；97 创建会话；103–190 持续 Provisioning；191 等待阶段失败；195 远端清理成功；197 结构化错误 | 注册等待 30053ms；runtimeAvailable=false |
| 第二次 | seq 462 能力快照；464 编译计划；473 创建会话；479–566 持续 Provisioning；567 等待阶段失败；573 远端清理成功；575 结构化错误 | 注册等待 30050ms；runtimeAvailable=false |

两次能力返回均为 `ConnectedStandby`，且 `remoteManagementEnabled` / `consoleStreamingEnabled` 缺失，地址数量为 0。这些字段支持“就绪事实尚待确认”；开关关闭、主机唤醒失败、主机到服务端注册受阻及服务端准备异常继续作为候选原因。

源码定位：

- `crates/xbox-streaming/src/session/startup.rs` 的 `prepare_remote_console` 按计划执行唤醒/就绪检查；本次两个计划均关闭这两项。
- `crates/xbox-streaming/src/session/flow.rs` 的 `evaluate_home_server_registration_wait_timeout` 对持续 `Provisioning/pending` 使用固定 30000ms 截止。编译计划同时记录 `startupTimeoutMs=90000` / `readyTimeoutMs=90000`，实际先命中 30 秒期限。
- 错误归类为 `hostRegistrationRetryExhausted`，两次均 `boundedRetry=null`。本样本只支持两次用户重新发起和本地超时证据；自动重建重试次数需独立表达。

故障定位置信度高；主机/服务端的具体注册失败原因仍需要相应事实。下一轮主机验证采用已开机且远程功能状态明确的主机，验证从就绪到 `ReadyToConnect` 的进展，再验收浏览器媒体链。

## 云游戏连接与画质

| 阶段 | 证据 |
| --- | --- |
| 启动 | seq 671 开始；688 创建；696 ReadyToConnect；后续 Provisioning；749 会话准备成功 |
| 协商 | seq 794 remoteAnswerSet；H264 `42e02a` constrained-baseline，PT 98 |
| 连通 | seq 822 ICE connected；830 peer connected，bind→connected 5585ms |
| 首帧 | seq 835 首包；845 首呈现；847/848 首解码/关键帧解码；connected→present 约 773ms |
| 尺寸变化 | seq 849 为 2560×1440；1440 在 10:37:07.069 变为 1280×720；面板和 SR 重新观察源尺寸 |
| 退出 | seq 5718 用户退出；5725 本地资源归零；5727 本地 runtime 清理 17ms；5742/5745 远端清理约 6.2 秒成功 |

点击启动至首呈现约 33.6 秒，主要耗时包含服务端准备和连接阶段。配置请求 H264 high / 1440p / 100Mbps，实际 answer 采用 baseline，实际源降至 720p；requested、negotiated 和 observed 需继续分别解释。

本次 cloud 编译计划选择 `WESTUS`，home 选择 `WestUS2`；配置包含强制区域 IP。云游戏选定候选对持续为 Direct UDP（srflx→host），候选对身份保持一致。区域与网络路由属于高 RTT 的检查范围，具体丢包位置仍需要路径侧证据。

## 网络、帧供给与恢复

| 指标 | 首段前台 78.904s | 最后恢复前台 24.724s |
| --- | ---: | ---: |
| 证据区间 | seq 836–3828 | seq 4888–5716 |
| 累计差值包损失 | 4.748% | 6.440% |
| RTT 中位数 / P95 / 最大 | 228 / 582 / 777ms | 1259 / 2494 / 3357ms |
| 接收 / 解码 FPS | 29.682 / 28.135 | 28.353 / 16.421 |
| 视频码率 | 1.591Mbps | 0.510Mbps |
| 浏览器丢帧增量 / 接收帧增量 | 113 / 2342 | 295 / 701 |
| jitter buffer 每出队帧平均等待 | 266.555ms | 1591.985ms |
| jitter buffer 目标平均等待 | 64.189ms | 114.621ms |
| 每解码帧平均解码耗时 | 3.546ms | 7.648ms |
| 浏览器 freeze 增量 / 时长 | 104 / 38.126s | 25 / 19.275s |

首段前台约 30fps 的帧接收供给与约 28fps 解码同时伴随冻结，稳态帧平均值与逐帧连贯性要分别验收。末段接收仍约 28fps，解码降至约 16fps，缓冲等待接近 1.6 秒；网络迟到、重组/缓冲和本地显示压力均需关联核验。单帧解码耗时支持将网络/缓冲等待列为主要问题，本地开销列为共同压力。

浏览器累计记录 `NACK=3265`、`PLI=4`、`FIR=0`、`keyFramesDecoded=10`。首个 stats 样本已含 1 次 PLI，所选采样区间内 PLI 增量为 3。RTX 接收增量为 1531 包。重传与关键帧解码具有活动证据；NACK 的按时/迟到/过期归属、PLI 的抑制/合并、请求→响应配对和恢复效果率继续待证。

恢复结构化事件 `pictureRecoveryTransition`、`pictureRecoveryBlockerObserved`、`videoIngressTermination`、`firstFrameLatencyObserved` 在本样本均缺失；浏览器的首帧 timeline 另有正向证据。`cleanAnchorCommitted` / `DisplayStable`、chain build success rate、NACK effective rate、repairability 连续性和 recovery effectiveness 评分标记为 N/A。当前摘要脚本对空闲 Rust 数据计算出的 0 分应按缺证解释。

本次没有应用级 `recoveryActionExecuted`、`bandwidthDownshiftApplied` 或媒体重连事件。持续拥塞时的现有监督大部分执行评估/本地显示调整，实际 SDP 降码率动作继续需要结合触发条件审查。

## 本地显示与策略问题

- 100 次 render telemetry 全部使用 `videoFrameCallback`；全部 `mediaTimeDeltaSec=0`，`sourceFpsEstimate/sourceFrameIntervalMs=null`，原因 `mediaTimeDeltaTooSmall`。Mac WebView 回调字段兼容性和源 FPS 置信度是直接优化点；30fps 接收平均值只能描述接收供给。
- 首段前台 sample 内最大回调间隔 P95 733ms、最大 1083ms；末段前台 P95 1934ms、最大 4516ms（seq 5056，10:39:17.825）。末段长尾发生在最后一次回到前台之后。
- 呈现提前量多数为负，整体 sample 范围 -24–0ms。呈现跨度最大 18，callback gap 和 presentedFrames jump 均反复出现；`renderFrameDropped=1119` 表达 dropped-like 事件，真实浏览器丢帧另见 stats。
- `renderBackpressureChanged=627`，telemetry 中 10/100 个 sample 为 true，压力反复起落。显示档位变化 55 次，目标 FPS 变化 25 次；前后台转换之外仍频繁在 30/60 FPS 间切换。
- 长后台段的回调计数积累到 711、呈现推进 1739，集中在 seq 4817 sample。后台供给与恢复窗口需单独裁剪后进入健康度评估。
- `qualityLadderPolicyEvaluated=14`，全部 `unsupported/missingVideoSender`；1 次 L1→L2、13 次 L2→L2。当前 recvonly 能力短路有效，评估事件与实际动作继续需要明确标识并消除相同目标重复记录。
- 活跃 renderer 为 `webgl2_sr`，实际 CAS performance 路径；最大 JS/WebGL 提交耗时累计观察到 204ms。它是一次或多次长尾的上界，实际 GPU 执行与 submit→present 时延继续待测。

## 优化实现与报告门槛复核

- 480 次已记录 getStats 采样属于同一 client/peer/video；按生产单调时钟排序后，采集区间均保持顺序，调用耗时最大 25ms，相邻采集空隙最小 228ms。
- 279 次观察消费、274 次 fresh、14 个 panel/supervision 共用样本；同样本事实一致。
- 77 次资源观察均符合当前门槛：至多 1 个 current + 1 个 pending、至多 1 个原生帧回调；本次 current 持续为 1。GPU 对象稳定为 8，duplicateDrawCount=0；退出时所有检查资源为 0，关闭后迟到提交门槛保持通过。
- 两次截图 GPU readback 有计数证据；麦克风和 AudioContext 始终为 0，相应交互路径继续待验。堆/RSS 与系统 GPU 回收继续待长期样本。
- 浏览器行为报告的 SDP/连接/首包/解码/首呈现证据通过；`--require-stability` 总结果 FAIL，四个条目需按原因解释。
- seq 5507 是 sample 1:477（454679ms），seq 5509 是较早 sample 1:476（454427ms）。前端 trace 使用异步 RPC 落盘，报告直接按文件顺序判断导致 overlapping / nonMonotonic 告警。按采样时间重算后三个告警消失。
- 重算保留一个 `collectionRateAbove4Hz`：sample 1:482–486 的五次起点跨 995ms，属于 250ms 定时器边界抖动。报告需使用适当容差及长期频率，再保留真实并发/超频的失败门槛。
- lifecycle reconnect gate 返回 `missingHealthyTwccOutputWindow`；其健康窗口依赖 Rust 视频/TWCC 快照。本样本使用浏览器媒体事实，按缺证解释，浏览器连接 timeline 与应用事件支持此次保持同一 peer。

## 后续优先级

1. P0：公共主机准备流程核对 `ConnectedStandby`、显式唤醒偏好、就绪事实、注册等待期限和错误语义。保留用户配置意图，通过真实主机 ready 事实确认下一段媒体验收入口。
2. P1：修正稳定性报告的单调时钟排序与采样边界容差，保留 owner/代际、真实重叠、实际超频和关闭后提交门槛。
3. P1：P4 收口同有效目标的画质评估去重、Mac 源节拍缺证处理、显示驻留和后台样本隔离，降低策略切换与日志重复。
4. P2：复核持续拥塞且仍间歇出帧时的实际降码率条件，按原 RFC 的标准协商路径与有界事务执行；以网络健康、稳态供给、缓冲长尾和恢复后帧差验证效果。

本轮工作范围为日志分析与验收记录更新。自动实机运行继续暂停；主机故障、完整功能、平台和长期验收保留待验状态。
