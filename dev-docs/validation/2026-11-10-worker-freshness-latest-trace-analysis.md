# 12:30 最新日志：分辨率约束与 Worker 新鲜度

1080p 创建画像和本地 SDP 接收约束已落实，远端实际仍发送 1440p。Worker 供给与 WebGL 提交约 60 fps，累计 7 帧差额由 6 次最新帧覆盖和 1 次陈旧交付丢弃配平；稳定配对窗口新增 3 帧差额。短时更新间隔长尾仍存在，跨线程时钟有效性与物理上屏证据需要继续补齐。

## 样本与复算

- 源日志：`/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1794285015731-1.jsonl`。
- 完整行快照：`/tmp/xbxrc-2026-11-10-1230-latest.jsonl`；975 行，2,110,047 bytes，seq 1–975 连续，schema v3 / dev，预算丢弃通知 0。
- 日志时间：2026-11-10 12:30:15.731–12:31:49.631；单次 Home 会话，实际渲染段约 33.2 秒。
- SHA256：`9f21810d88716cbdc19de0fc8a3fd722c73fcfa125f9b98bb3dfae9c084062ae`。
- 构建：Browser / macOS / debug，commit `76a1ea69`，dirty=true，buildTimestamp `1794285001479`。
- 源码指纹：`de93f89e2eb119c2083bc37e68b52cce4177ce4cedca9d3005ae79db75f6d3a2`；按 build.rs 的 560 个输入复算，与日志构建一致。
- 分析脚本：[2026-11-10-worker-freshness-latest-trace-analysis.py](2026-11-10-worker-freshness-latest-trace-analysis.py)；安全指标：[metrics.json](2026-11-10-worker-freshness-latest-trace-analysis.metrics.json)。本轮仅新增分析文件。

```sh
python3 -B .agents/skills/analyze-runtime-logs/scripts/trace_browser_webrtc_behavior_report.py /tmp/xbxrc-2026-11-10-1230-latest.jsonl --require-stability > /tmp/xbxrc-1230-browser-report.json
```

该 gate 退出码为 2，唯一原因是 `sharedSamples` 缺证，具体解释见下文。

```sh
python3 -B dev-docs/validation/2026-11-10-worker-freshness-latest-trace-analysis.py /tmp/xbxrc-2026-11-10-1230-latest.jsonl \
  --source /Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1794285015731-1.jsonl \
  --source-root /Users/guo.xu/Documents/code/games/xbxrc \
  --browser-report /tmp/xbxrc-1230-browser-report.json \
  --output /tmp/xbxrc-1230-worker-freshness-recomputed.metrics.json
```

## 分辨率与实际效果

| 证据 | seq | 结果 |
| --- | --- | --- |
| 会话创建画像 | 103 | Home configuredResolution=1080，Windows device/header 均为 1920×1080 |
| 执行上下文 | 141 | targetVideoWidth=1920，targetVideoHeight=1080 |
| localOfferAfterPatch | 155 | H264 PT 96/98/100/103 全部带 `recv [x=[1:1920],y=[1:1080]]`、`max-fs=8160;max-fr=60` |
| remoteAnswer | 171 | send 上限 2560×1440；最终 H264 PT98，profile-level-id=`42e02a` |
| 实际媒体尺寸 | 217–924 | 132 个有效 inbound 尺寸样本全部为 2560×1440 |
| 实际渲染策略 | 209–938 | SR 开关开启、同尺寸 passthrough，USM=3；actualSuperResolutionActive=false |

创建参数→执行上下文→本地 offer 的链路与 1080p 选择一致。远端 answer 的发送能力和实际 inbound 仍为 1440p，远端分辨率档位采纳需要继续排查。本次输入/输出同为 1440p，真实 1080→1440 EASU 重建性能需要单独实机取样。

## 稳定供给与帧供给 delta

| 测量范围 | 接收 | 解码 | 提交 | 覆盖 |
| --- | ---: | ---: | ---: | ---: |
| RTC 完整播放 delta | 59.893 fps | 59.893 fps | — | 33.059 s |
| 同 sampleId 配对：1:16→1:144 | 59.940 fps / 1936 帧 | 59.940 fps / 1936 帧 | 59.847 fps / 1933 帧 | 32.299 s |
| live renderer 计数差 | — | — | 59.860 fps / 1954 帧 | 32.643 s |
| 45 个完整 3 s 滑动窗的最低提交率 | — | — | 58.675 fps | seq358 / 1:41 |

配对起点 seq222：RTC decoded=8、Worker read=8、sent=4、delivered=4、submitted=4、overwrite=4、stale=0。配对终点 seq911：decoded=1944、read=1944、sent=1938、delivered=1937、submitted=1937、overwrite=6、stale=1。新增 decoded−submit 差额为 3，恰好对应新增覆盖 2＋陈旧丢弃 1；累计差额从 4 增至 7，之后稳定。

最后 live 快照 seq918 的分层计数：

| 层 | 帧数 | 与上一层差额 | 解释 |
| --- | ---: | ---: | --- |
| Worker read | 1962 | — | 已读取帧 |
| Worker sent | 1956 | 6 | latest-only 待发送槽覆盖 6 帧 |
| 主线程 receivedMessages | 1956 | 0 | 发送与接收配平 |
| Reader delivered | 1955 | 1 | freshness gate 释放陈旧帧 |
| WebGL realtimeDrawCount | 1955 | 0 | 16 个 live 快照全部交付/提交配平 |

duplicateDraw、rateLimitDrop、submitFailure、renderer latestOverwrite 均为 0。该计数链支持继续保留最新帧优先策略。

## 短时空窗与本地新鲜度长尾

seq305→318 的两个观测快照覆盖 529 ms，首次观察到 overwrite 4→6、stale 0→1、transfer max 35→68 ms、ACK 往返 max 40→67 ms、提交间隔 max 61→89 ms。这里采用 payload 的采样时间；日志写入时间有独立滞后。

同窗 RTC seq307/310 的两个 delta 合计 520 ms：接收 31 帧、解码 31 帧，RTT=12 ms、jitter=5–6 ms、lost/NACK/PLI/FIR/freeze 增量均为 0。局部更新空窗伴随消息交付/ACK 长尾；各最长耗时对应同一帧的因果关联仍需逐帧时间元组验证。

最后 live 快照 seq918 的有效 256 帧保留窗：

| 时序 | P95 | P99 | 保留窗最大 | 全程观测最大 |
| --- | ---: | ---: | ---: | ---: |
| Worker read 等待 | 26 ms | 41 ms | 57 ms | 1789 ms，首帧媒体等待 |
| 待发送槽等待 | 0 ms | 0 ms | 0 ms | 6 ms |
| Worker send→主线程 receive | 1 ms | 3 ms | 4 ms | 68 ms |
| 主线程 consume | 1 ms | 1 ms | 1 ms | 6 ms |
| Worker ACK 往返 | 2 ms | 3 ms | 5 ms | 67 ms |
| 主线程 ACK→Worker | 1 ms | 1 ms | 1 ms | 1 ms，有效子集 |
| Worker read 完成→WebGL 提交 | 1 ms | 3 ms | 4 ms | 41 ms |
| 主线程 receive→提交 | 1 ms | 1 ms | 1 ms | 6 ms |
| 主线程接收间隔 | 26 ms | 41 ms | 57 ms | 86 ms |
| WebGL 提交间隔 | 25 ms | 42 ms | 57 ms | 89 ms |
| CPU 绘制 | 1 ms | 1 ms | 1 ms | 6 ms，初始化 |

43 个填满的保留快照中，read→submit P95 范围为 1–3 ms，ACK 往返 P95 为 1–2 ms，readWait P95 为 23–29 ms，提交间隔 P95 为 23–29 ms。readWait 的稳态保留窗最大值曾达 67 ms，末窗为 57 ms。重叠保留窗分别描述局部时序，全程 P95 需要独立逐帧样本。

末窗 CPU 绘制均值为 0.336 ms；同一提交计数差窗口的 CPU 均值为 0.322 ms。待发送槽等待与主线程消费较低，后段接收/提交间隔跟随 readWait，下一层诊断应聚焦处理器可读帧的供给节拍。readWait 从 `reader.read()` 发起到返回，覆盖上游供给与 Worker 调度等待，其拆分仍需更细的时间事实。

freshness gate 以 read 完成→主线程收到超过 50 ms 为释放条件，68 ms 的交付尾部已得到释放。已提交帧 read→submit 全程观测最大为 41 ms，末窗 P95 为 1 ms。`rendererSubmitAgeMs` 样本 P95=22 ms、最大 26 ms，描述采样时距上次本地提交的时间；物理显示器 scanout、GPU 完成及完整 glass-to-glass 仍缺少测量。

## 网络、恢复与呈现证据边界

网络：终值 packetsLost=17、packetsReceived=71024，按 lost/(received+lost) 计算约 0.0239%；RTT P95/最大 18 ms，jitter P95=7 ms/最大 9 ms，实际码率均值 18.64 Mbps。当前 BWE 配置为 fixed-remb=50 Mbps。完整播放窗接收/解码均为 1980 帧，平均 decode=4.35 ms、jitterBuffer dwell=3.41 ms，RTC dropped/freeze/pause 均为 0，250 ms 级接收零帧窗口为 0。

恢复：首个有效 inbound 已解码 3 个关键帧、PLI 累计 2；完整播放窗新增关键帧 1、PLI/FIR 增量 0，NACK 增量 84、重传包增量 85，持续解码，renderCause 33 次均为 renderStable。Browser ReferenceChain、cleanAnchorCommitted、DisplayStable 以及逐次 NACK outcome 的观测能力受限；request suppression、响应可用性、建链成功率、NACK effectiveRate、repairability 持久化与综合恢复评分均记为缺证。聚合关键帧/重传计数支持媒体持续性的结论。

呈现：trackingSource 全部为 videoFrameCallback；sourceFpsEstimate 因 mediaTimeDeltaTooSmall 缺值，源节拍由 RTC delta 复算。RVFC 约 53.92 Hz，presentedFrames 推进代理约 59.96 fps，190 次多帧推进合并，callbackGapCount=0、最大 callback 间隔 88 ms；Worker 提交独立保持约 60 fps。presentedFrames 描述 video 呈现推进代理；WebGL canvas 的物理上屏长尾保留为缺证。GPU timer 状态 unavailable，queryCount=0。

时钟：seq918 invalidClockCount=15/receivedMessages1956，约 0.77%；15 帧仍正常交付，其 transfer/read→submit 耗时记为未知。ACK 往返有效记录 1953，ACK 单向交付有效记录 789，相对 workerAckCount1955 的覆盖约 40.36%。代码将负差或非有限值过滤为 unknown；WKWebView 跨线程时钟量化/偏移属于待验证假设，现有分布缺少无效原因和原始时间元组。

稳定 gate：`browserWebRtcBehaviorGate=FAIL`，`browserRuntimeStability=MISSING`，唯一 missingEvidence=`sharedSamples`。脚本要求 panel 与 supervision 读取同一 run/session/client/peer/sample identity；本次 supervision=34、panel=15，其 sampleId 没有交集，sharedSamples=0。该项描述双消费者同样本的证据覆盖；供给结论由 RTC/renderer 的 sampleId 配对与 live 分层计数核验。已有 collectionEvents133、freshObservationEvents48、frameChainSamples47、closedClients1，稳定性内部 failures=[]。

收尾：seq938 clientClosed 后 current/pending renderer、reader、retained frame、frame request、native/animation callback、video/audio element、ready timer 等资源均为 0。前端生产者 dropped、droppedEssential、queueFull、ipcFailed、ipcTimedOut、rowOversized、ackMissing 均为 0。

## 后续优先级

1. 查清 Home 远端对 1080p 档位的采纳规则与服务端响应，建立请求尺寸→answer→实际 inbound 的一致性验证。
2. 补充跨线程时钟校准、invalid 原因分类及限量原始时间元组，并补齐双消费者同样本证据；继续保留单帧在途、latest-only、50 ms 陈旧交付释放。
3. 获取真实 1080→1440 SR 和 USM 切换的长测，分别验证 readWait/receiveInterval 空窗、read→submit 新鲜度及可获得的 canvas 呈现长尾。
