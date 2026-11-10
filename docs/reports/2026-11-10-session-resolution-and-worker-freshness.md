# 会话分辨率约束与 Worker 新鲜度修复

日期：2026-11-10；主窗口直接执行。代码与自动验证完成，远端采纳和实机新鲜度收益接续用户串流验收。

## 结果

会话创建链路已经正确使用 xHome 独立档位。cloud=1440 时，Home 720/1080/1080HQ/1440 的设备头、系统画像和实际 HTTP payload 均通过本地请求回放；cloud 保持自己的设备画像与 4096×2160 displayInfo 合同。

Browser offer 现在同时声明 H264 宏块、帧率和接收宽高：1080p 为 `max-fs=8160;max-fr=60` 与 `a=imageattr:<PT> recv [x=[1:1920],y=[1:1080]]`。约束只修改当前接收视频段中的 H264 payload，保持 RTX、其他媒体段及既有 send 属性；LF/CRLF、缺省 fmtp、被拒绝/停用/仅发送的视频段和 ICE 重启均有回归覆盖。

Worker 连续读取处理器，独立保存一个最新待发送帧和一个在途帧。匹配序号的 ACK 到达后直接发送最新槽；替换待发送帧及时 close。主线程对已知读完到收到年龄超过 50ms 的帧 close/ACK，释放后续最新帧。时间证据缺省或无效时保持 unknown 并继续现有交付；首帧截止、资源退出和现有 video 故障降级保持闭环。

## 日志排查事实

绑定日志 `runtime-trace-1794280647769-1.jsonl` 的第二会话：config、compiledPlan、Windows 画像、HTTP 设备头、Browser transport 目标均为 1920×1080。本地 offer 只有 max-fs/max-fr；远端 answer 发送尺寸上限为 2560×1440，194 个有效 inbound 尺寸样本全部为 1440p。新增接收宽高声明补齐协商证据，远端实际输出由下一份日志确认。

该日志原画、USM 与第二会话的配对窗口解码/提交均配平；首段局部新增 4 帧缺口。原画一次 175ms 冻结同窗出现 RTT 107ms、先低后高的收帧/解码供给以及 176ms 提交空窗，CPU 绘制 P95 为 1ms。旧日志的 Worker 读取、传输和 ACK 分段缺证，4 帧差额与冻结的精确归因接续新增时序。完整分析见[最新日志报告](../../dev-docs/validation/2026-11-10-decoded-supply-latest-trace-analysis.md)。

| 健康维度 | 当前基线结论与验收方向 |
| --- | --- |
| 网络 | 两会话总丢包约 0.061%/0.024%，RTC dropped 增量为 0；175ms 局部冻结同窗出现 RTT 抬升与供给突发 |
| 恢复链 | 两会话持续解码，完整播放窗 PLI/FIR 增量均为 0；Native ReferenceChain/DisplayStable 与逐次关键帧闭环按 Browser 能力标记缺证 |
| media supply | 接收/解码约 59.9fps；局部 250ms 供给窗口先 6 帧、后 21 帧，需要细分读取与传输等待 |
| steady supply | 主要段提交 59.84–60.01fps；完整 3s 最低提交在原画段降到 56.56fps，平均与短时空窗分别评估 |
| submit/present 长尾 | CPU 绘制 P95 1ms，已捕获最大本地提交间隔 176ms；GPU timer/物理上屏缺证，本次新增本地读取/收帧到提交分段 |
| 帧供给 delta | 首段新增 4 帧差额，后续主要段新增差额为 0；最新槽覆盖与陈旧交付丢弃独立记账，优先保证提交新鲜度 |

## 时序与供给缺口分解

跨 Worker/window 使用 `performance.timeOrigin + performance.now()`；帧 timestamp=0 继续作为独立帧身份诊断。固定 256 项数值窗口仅在快照时计算分位数。

| 字段 | 对应阶段 |
| --- | --- |
| readWait | reader.read 发起到读取完成，反映处理器供给等待 |
| pendingWait | 读取完成到 Worker transfer，反映最新槽等待 |
| transfer | Worker transfer 到主线程收到，包含主线程消息调度等待 |
| consume | 主线程同步消费回调，包含同步上传/提交工作 |
| ackDelivery | 主线程 ACK 发出到 Worker 收到 |
| ackRoundTrip | Worker transfer 到 Worker 收到对应 ACK |
| receiveInterval / receiveAge | 主线程交付间隔及距最后交付的时间 |
| workerObservationAge | 距最近一份 Worker 消息内计数采集点的时间；静默期间读/ACK 计数保持最近已回传事实 |
| receiveToSubmit / readToSubmit | 主线程收到/Worker 读取完成到实际 WebGL 命令提交 |

dev 沿既有 renderTelemetryObserved/renderer resources 发布完整供给快照；production 以 1Hz `decodedFrameSupplyObserved` 发布有界计数和 P95/保留窗最大值，按 sampleId、rendererGeneration 和 rendererFrameSource 配对。会话计划发布 `sessionVideoConstraintsResolved`；SDP 发布安全数值 videoImageConstraints，生产投影继续过滤原始 SDP 文本。

同一稳定代际内，分别比较 RTC decoded→Worker read、read→sent、sent→received、received→delivered、delivered→rendererSubmit。read−sent 同时覆盖 latest overwrite 与待发送槽的边界变化；received−delivered 可由 staleDrops 核验。主动覆盖和陈旧帧释放分别记账，采样边界和代际重置单列。ACK 往返、transfer 和 readWait 的变化用于区分本地消息调度压力与上游供给等待。

Worker 计数随帧消息回传；静默期依据 workerObservationAge 标记计数年龄，恢复后的覆盖计数和 ACK 最大值用于复盘该空窗。

readToSubmit 表达本地读取后的年龄；网络、解码到读取的年龄、GPU 完成和物理上屏分别需要对应证据。

## 自动验证

| 验证 | 结果 |
| --- | --- |
| 会话 offer、统计、Worker/reader、WebGL/SR 调度、播放降级、runtime、生产投影 | 20 文件、318 项通过 |
| xbox-streaming 全量库回归，含实际 HTTP 创建请求 | 146 项通过 |
| Rust/TypeScript 生产 trace 共享合同 | Rust 4 项通过；前端包含在上述回归 |
| 30 分钟生产日志与输入成本预算 | 2 项通过；1801 条健康记录，normal 307869.7B/min，编码 P95 0.126959ms，健康行最大 3744B，丢弃/超长/IPC 失败均为 0 |
| cargo fmt、pnpm lint:fix、typecheck、build、diff check | 通过；独立 Worker 资源正常产出 |
| 确定性供给回放 | 当前 Worker/reader 与显式旧调度模型比较；源码 SHA 绑定；所有测试帧最终释放 |

供给回放使用 60fps、2ms transfer、1ms 同步消费、1ms ACK 交付；主线程分别停顿 80ms、175ms：

| 主线程空窗 | 陈旧帧提交：基线→当前 | 最大已提交源年龄：基线→当前 | 空窗结束到新鲜帧提交：基线→当前 |
| --- | --- | --- | --- |
| 80ms | 1→0 | 81→17.333ms | 5→4ms |
| 175ms | 1→0 | 176→12.333ms | 5→4ms |

这是确定性模拟，基线为旧 ACK 串行策略模型。主线程停顿本身仍形成画面空窗；当前策略在恢复后及时释放陈旧在途帧并承接最新供给。目标设备的 GPU、物理上屏和真实网络收益由实机样本验证。

复算说明、源码身份和指标见[验证目录](../../dev-docs/validation/2026-11-10-session-resolution-worker-freshness/README.md)。

## 实机验收

重新启动 1080p xHome 串流，按原画、SR、锐化分别保留稳定观察段。核验 sessionVideoConstraintsResolved、localOfferAfterPatch 的 recv 范围、remoteAnswer 的 send 范围及 inbound frameWidth/frameHeight；1080 输入小于输出时，再检查实际 EASU 重建激活。

同时覆盖网络 RTT/jitter/loss、Browser 恢复/首帧与故障降级、media supply、完整 3s steady supply、分层帧供给 delta、本地 readToSubmit/transfer/ACK 长尾。Browser ReferenceChain、GPU timer 和物理 scanout 按实际能力标记；Worker 交付年龄预算由本地时间证据判定。
