# Mac 云游戏可靠性改造首批日志验证

## 状态与结论

- Date: 2026-10-28
- Sample Status: 部分验收完成，结束方式为用户强制关闭程序。
- RFC Status: In Progress，P4 继续分组合验收。
- 关联：[串流公共主线可靠性与代码精简 RFC](../rfcs/2026-10-28-streaming-common-reliability-simplification.md)。

本批通过构建身份、云会话创建与准备、SDP/ICE、首帧和播放区间生命周期检查。首帧后的可见启动段存在媒体供给停顿；后续大部分采样处于页面后台。正常退出、实际断网恢复、长时间前台播放和性能准入继续保留待验状态。

## 样本与构建身份

| 项目 | 证据 |
| --- | --- |
| 原始文件 | `/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793169376818-1.jsonl` |
| 时间 | 2026-10-28 14:36:16.821–14:39:39.815，Asia/Shanghai；202.994 秒 |
| 记录 | 1,923 行，seq 1–1923，单个 traceRunId |
| 平台与运行时 | macOS，cloud，webrtc-direct；详细 WebView 版本保留待补 |
| 区域与配置 | WESTUS3；2560×1440；初始目标 60fps，接收端采样中位数约 30fps；实验超分配置为 fsr1Experimental |
| 构建 | debug，Git `02f6db40`，workspaceDirty=true；seq 2 / 224 |
| 源码 SHA-256 | `59c93f67fa6cf6a5690b93d53cd3b852746136692d59d34197944ac8c710e595`，与当前源码及 RFC 记录一致 |
| 原始文件 SHA-256 | `56b823316dc3ce2fbfee08198b5273bff875cecca4b4ba30ab235e4f9e7a9d02`；5,646,838 bytes |
| 诊断预算 | dev；essential 3 / key 1920；traceBudgetNotice=0 |
| 结束条件 | 用户明确“是我强制关闭的程序”；最后一行仍为播放侧 renderFrameDropped |

分析使用固定字节快照，读取前后源文件大小一致。原始日志保留在用户本机及临时分析目录；仓库保存汇总指标和必要事件字段，资源编号在报告中使用 S1。

## 最小时间线

| 时间 | seq | 事件与含义 |
| --- | --- | --- |
| 14:38:01.087 | 225 | startup-1 开始，startup generation=1 |
| 14:38:08.113 | 248 | S1 创建，resource generation=2 |
| 14:38:27.805 | 302 / 305 | 公共准备完成，startup 成功，耗时 26,718ms |
| 14:38:31.794 | 349 | remote answer 已设置，H264 constrained-baseline `42e02a`；bind→answer=3,470ms |
| 14:38:39.265 | 390 | PeerConnection connected |
| 14:38:40.464–40.466 | 400 / 401 / 408 | 首个关键帧解码及呈现回调；运行时报告 connected→firstPresented=1,204ms |
| 14:38:42.836–44.150 | 525–563 | 收包继续，解码一度保持 64；PLI 2→6；seq 557 呈现回调间隔 1,297ms；随后 keyFramesDecoded 4→5、framesDecoded 64→67 |
| 14:38:48.645 | 757 | documentHidden，渲染预算 30→0 |
| 14:39:32.436–32.489 | 1543 / 1565 | 页面短暂恢复约 53ms，随后再次 documentHidden |
| 14:39:39.790–39.815 | 1922 / 1923 | 最后 RTP 统计仍 connected，后续一条回调记录；程序由用户强制关闭 |

## 门禁结果与覆盖边界

| 检查 | 结果 | 解释 |
| --- | --- | --- |
| revision / source fingerprint / trace 新鲜度 | PASS | 实际构建与改造源码绑定成功 |
| browser WebRTC behavior | PASS | SDP、连接、关键帧解码、呈现里程碑完整 |
| 公共生命周期播放区间，`--allow-active` | PASS | 1 次启动、1 个资源；operation start/finish 配对；超时、连接提交和动作 drain 均为 0 |
| 完整关闭，`--require-cleanup --require-closed` | FAIL / 证据不足 | 输出 cleanupEvidenceMissing、remoteResourceUnclosed；用户强制终止后，正常 teardown 和服务最终回收分别保留待验 |
| 连接故障恢复合同 | 待验 | recoveryProposed/Granted/Committed/Observed 均为 0；实际断网、授权竞态和恢复预算等待独立故障样本 |
| P4 整机与性能准入 | 待验 | 当前包含约一分钟媒体数据及后台阶段；前台长期稳定样本、正常退出、2h/8h soak 和支持组合矩阵继续取证 |

播放段 observed startup timeout=0、connection rebuild=0。远端 monitor 为 Provisioned/sessionReady 的 25 个快照继续服务当前资源。V1 的 180 秒准备状态回退、V2 的长排队、V3 的取消竞态、V4 的正常重复退出、V5/V6 的连接恢复正样本以及 V7/V8 的控制面和旧信令故障仍按 RFC 矩阵验收。

## 可见与后台窗口

边界按 documentHidden/documentVisibleResume 的结构化事件取值；累计 RTP 计数使用边界前最近样本，保留具体 seq 便于复算。

| 窗口 | 时间与长度 | 冻结计数增量 / 冻结时长增量 | 最大回调间隔 |
| --- | --- | --- | --- |
| 首帧后可见启动段 | 14:38:40.466–48.645，8.179s；RTC seq 404→755 | 7 / 3.021s | 1,297ms |
| 首个后台段 | 14:38:48.645–14:39:32.436，43.791s；RTC seq 755→1536 | 36 / 10.757s | 667ms |
| 最后后台段 | 14:39:32.489–39.815，7.326s；RTC seq 1561→1922 | 5 / 1.319s | 267ms |

约 59 秒的整体 freezeCount=48、totalFreezesDuration=15.097s 包含后台统计。前台性能判断使用可见启动段，稳态前台体验需要追加更长的独立窗口。

## 六个必报维度

| 维度 | 本批观测与证据范围 |
| --- | --- |
| 网络健康 | 294 个 RTP 样本的 connection/ICE 均为 connected；已观测候选对为 srflx→host。WESTUS3 路径 RTT P50=231ms、P95=234.6ms、最大364ms；jitter P50=9ms、P95=29.4ms；本窗丢包302，received packet delta=15,878，损失比例约1.867% |
| 恢复链健康 | PLI=6、FIR=0、NACK=810，解码关键帧累计7；seq 525→563 证明一次关键帧出现后的解码续供。连接事务提交为0。浏览器内部 ReferenceChain/RecoveryLedger/cleanAnchorCommitted/DisplayStable 的逐 episode 证明超出本批可观测范围，Rust runtime 的对应门禁等待独立样本 |
| media supply | RTC seq 399→1922，59.424s；framesReceived delta=1,765、framesDecoded delta=1,737、framesDropped delta=20。平均 receive=29.702fps、decode=29.231fps；平均 decode 用时约4.712ms/frame，平均 jitter buffer delay约50.407ms/frame |
| steady supply | 整体冻结48次、累计15.097s；可见启动段7次、3.021s。seq 525→555 解码停在64时收包872→1124，随后收到关键帧并续供。后台阶段单列，长期前台稳态保留待验 |
| submit/present 长尾 | trackingSource=videoFrameCallback；37个监督窗口的最大 callback gap P95=667ms、最大1,297ms；呈现推进一次最多跨10帧。callback/draw和预期显示时间作为代理量，真实 submit-to-present / glass-to-glass 需要外部测量 |
| 帧供给 delta | 同窗 seq 408→1906，59.046s：received=1,753、decoded=1,725、callback=1,158、presentedFrames advancement=1,633；约29.689→29.215→19.612次callback/s，呈现推进27.656frames/s。callback次数与浏览器呈现推进分别记账，合批推进可复盘 |

浏览器内部反馈的逐请求配对、有效重传期限、clean-anchor/display 闭环、repairability 持续性和综合 recovery score 在本批记为 NA。重传包增量912证明重传到达；NACK 有效率需要逐包时效与恢复结果配对。关键帧累计7与PLI累计6分别反映各自计数，配对成功率等待对应 episode 证据。

renderFrameDropped 共302条，拆分为 callbackGap=101、presentedFramesJump=259，两者重叠58条；它表达回调节拍与批量呈现特征。37个监督窗口中 renderBackpressure=true 有4个，renderCause=renderStarvation 有4个，其余33个记录renderStable。持续 GPU 绘制失败的归因需要 draw/GPU 证据。

## 已确认问题与后续定位

1. **前台启动供给存在长停顿。** 首个1.297s gap位于首次documentHidden之前；packet继续推进、decoded停顿、PLI增加、关键帧解码后续供。高RTT、丢包和重传等待构成已确认网络压力；远端完整帧到达及浏览器内部恢复时序需要更细观测。其余呈现长尾同时包含后台预算与回调合批。
2. **恢复监督和展示使用不同统计口径。** 39个recoveryCauseClassified的longWindow均为decodeFpsAvg=0、jitterMsAvg=-1，和RTP实际解码、网络jitter错位。`browser-runtime.ts` 的checkMediaStalled直接消费`client.stats().snapshot()`，snapshotStats展示路径额外合并resolveBrowserStatsFallback。后续应让监督与展示复用同一浏览器统计投影，并保持当前代际校验。
3. **源节拍和显示刷新估算的证据仍需收敛。** 37个renderTelemetryObserved的sourceFpsEstimate均为空，原因mediaTimeDeltaTooSmall；本批mediaTimeDeltaSec=0。接收端FPS和呈现推进继续独立可用。refreshRateEstimateChanged触发策略重算90次，后台阶段与估算抖动的贡献需要单独核验。

## 证据与复算

- [门禁与构建摘要](trace-validation/2026-10-28-cloud-streaming-reliability/gates.json)
- [分窗与整体指标](trace-validation/2026-10-28-cloud-streaming-reliability/metrics.json)
- [必要事件锚点](trace-validation/2026-10-28-cloud-streaming-reliability/anchors.json)

计数采用累计值端点差；平均帧率使用端点单调采样时间；可见窗口保留对应墙钟和seq边界。RTT/jitter分位数采用样本排序及线性插值；记录中有一次1ms的同计数并发采样倒序，端点累计差保持一致。202条sessionId=null的原生statsSnapshot属于空闲宿主，本批浏览器验收以browserWebRtcStatsObserved和renderTelemetryObserved取证。

复跑原始文件：

```sh
python3 -B .agents/skills/analyze-runtime-logs/scripts/summarize_runtime_trace.py /Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793169376818-1.jsonl --json
python3 -B .agents/skills/analyze-runtime-logs/scripts/trace_browser_webrtc_behavior_report.py /Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793169376818-1.jsonl
python3 -B .agents/skills/analyze-runtime-logs/scripts/trace_streaming_session_lifecycle_gate.py /Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793169376818-1.jsonl --allow-active --expected-revision 02f6db40d6138787ca85a582dedf3b5028ad57ea --expected-source-sha256 59c93f67fa6cf6a5690b93d53cd3b852746136692d59d34197944ac8c710e595
```

下一批保持页面前台连续播放至少15分钟，通过串流退出入口正常退出并结束应用；独立追加一次受控断网与恢复。记录异常时刻和可见性切换，优先取得完整清理及恢复事务证据。2h/8h、50次启动退出、20次故障及其余支持组合继续按P4执行。
