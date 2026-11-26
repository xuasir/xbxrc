# PC Worker 首轮实机日志回归

视频 `worker-direct` 与输入 `input-worker` 已实机启用，前台稳态提交59.88FPS，上传和绘制P95/P99均1ms。整段健康验收为FAIL，失败集中在媒体供给停顿期间的submit/present年龄。M0–M4工程保持已交付，完整实机成本与功能验收继续为Validation Pending。本轮执行日志分析与证据归档。

## 范围与证据

- 日志：`runtime-trace-1795701183517-1.jsonl`，2026-11-26 21:53:03.517–21:55:27.459（Asia/Shanghai），3175行、8,430,352字节、143.942秒，串流观察109.090秒；dev profile，完整行冻结。
- 运行版本：Browser debug、git `37e4fb43`、dirty tree，源码指纹`39b5f808166db4b2bc55004e107f270d1c0a16f8e352f1fb5d1bd2bb80597e46`。
- 配置：macOS Home、2560×1440/60FPS、USM quality锐化3、SR关闭、颜色100，输出2994×1684，preventResolutionDrops开启、cap100000kbps。
- [facts.json](facts.json)保存指标、明确作用域、对照窗口与资格；[evidence-anchors.json](evidence-anchors.json)保存原始seq；[production-health.json](production-health.json)、[browser-behavior-summary.json](browser-behavior-summary.json)、[trace-summary.json](trace-summary.json)保存报告。
- [manifest.json](manifest.json)、[checks.json](checks.json)记录冻结、命令、冒烟与清理。[完整压缩日志](runtime-trace-1795701183517-1.jsonl.gz)的解压SHA256为`b798d2c33ee73acf83cbe50abc49d97407ad63e6cf22bc810226e2d88a956497`。
- 开始分析时原件存在；归档时原应用日志目录已不存在。本轮完整冻结压缩副本保留原始证据。

## 同档前台比较

新窗口为21:54:10.838–21:55:14.882，64.044秒、64个健康桶；[旧基线](../2026-11-26-pc-latest-log-1795692428673/README.md)窗口为19:30:14.924–19:31:18.955，64.031秒、64个桶。两段使用相同画质、源/输出尺寸与前台状态。按各counter覆盖毫秒累加非重叠健康桶delta；上传均值取各窗口内两个资源快照的累计差。时段、画面与操作不同，繁忙注入收益保留缺证资格。

| 指标 | 基线 | 本轮 |
| --- | ---: | ---: |
| received/decoded FPS | 59.674 / 59.674 | 59.896 / 59.896 |
| renderer submit FPS | 59.674 | 59.881 |
| submitted/decoded | 100% | 99.974% |
| Video present / rVFC callback FPS | 59.659 / 54.224 | 59.912 / 55.743 |
| 上传均值（累计差） | 0.471ms | 0.606ms |
| 上传/绘制窗口P95与P99的中位数 | 1 / 1ms | 1 / 1ms |
| read→submit窗口P95/P99的中位数 | 2 / 4ms | 2 / 3ms |
| 输入observation→send窗口P95/P99的中位数 | 1 / 1ms | 1 / 1ms |
| 输入drain窗口P95/P99的中位数 | 1 / 3ms | 0 / 1ms |
| 反馈发送年龄窗口P95/P99的中位数 | 9 / 10ms | 11 / 11ms |
| 网络RTT/jitter/视频缓冲中位数 | 4 / 5 / 5.71ms | 5 / 6 / 6.98ms |
| 音频缓冲中位数 | 38.46ms | 60ms |

保留窗有重叠，表内分位数代表每个窗口所报告分位数的中位数。新窗口read→submit所报P95最大3ms、P99最大5ms；基线分别2.5ms、8ms。新窗口的3s/15s末端提交59.72/59.63FPS，两个窗口的freeze、实际framesDropped和submit failure delta均0。rVFC多帧合批推进分别263/346次，实际videoPresent和renderer保持近60FPS；trackingSource为videoFrameCallback，mediaTimeDeltaTooSmall使源FPS直接估计缺证，源节拍通过received/decoded事实确认。

## 导入、端口与输入成本

- seq3108资源快照：6048次完整上传、0次子上传；上传累计3622ms，均值0.599ms，生命周期上传/绘制max6ms。截图draw和同帧texture reuse计数均0，当前回归覆盖实时完整导入。
- Reader→Render单向交接最后保留窗mean0.805ms、P951.5ms、P994.5ms；ACK往返mean1.395ms、P953ms、P996ms。read→submit最后保留窗2.5/5.5ms。该末窗含最后一次供给异常，P99较规划基线3ms增加2.5ms；时钟校准uncertainty为2.5ms，按同窗供给及计时资格继续审查成本门。
- Reader连续读取、每帧一次VideoFrame转移和一次CPU处理ACK。观察中held frame最大1；应用合同上界3，帧待处理队列上界1。invalid/unknown clock、stale drop与rejected frame均0。
- 最后资源快照：帧端口收/发6072/6072条、元数据估算5,427,970/2,974,988字节；控制收/发1066/1061条、4,512,616/230,778字节。100ms快照1034次、1Hz聚合104次，聚合计时累计97ms、max2ms。字节为应用JSON UTF-16估算；VideoFrame原生引用与驱动成本由交接/导入计时分别观察。
- 输入seq3083：交接max2ms，pendingHandoff/handoffInFlight/pendingInputs/stagedOverflows/traceOverflows均0；反馈5565帧，超时、过期和溢出均0，末窗发送年龄P95/P9911/12ms，16ms预算。
- 输入seq3091末窗observation→send P95/P995/5ms，physicalSampleAge为86/101ms。Worker心跳沿用最近设备帧的sampledAt、更新observedAt，物理年龄统计包含重复状态；后续分开统计新采样与重复心跳。实机远端短按识别继续由用户观察确认。
- 两次gamepadInputBackpressure均为owner-changed屏障，各取消1个旧源状态；epoch与neutral交接记录保留。seq3139 neutralAccepted=true，pendingInputs/pendingFeedback均0。

## 六维健康与异常时间线

有效production健康段为observationEpoch=1，104.060秒，publication覆盖99.04%、media valid覆盖98.84%。networkHealth、Browser公开统计recoveryChainHealth、mediaSupply、steadySupply、frameSupplyDelta均PASS，submitPresentTail为FAIL。整段received/decoded6078/6078，read/delivered/submit6074/6072/6072，Reader overwrite2，stale/rate-limit/duplicate/submit failure与实际framesDropped均0；窗口端点异步采样保留余量。

| 窗口与锚点 | 事实 | 判断 |
| --- | --- | --- |
| 21:53:54.208起，episode1，seq657–837 | received与decoded同步降到0；seq711/715/720累计包、字节与帧均停止推进。Reader seq889等待1033ms，随后转移约1ms、消费约2ms。submit/present年龄最高844/925ms，上传/绘制仍低于1ms | 媒体供给停顿；episode稳定确认延续5.29秒，前三次实际freeze合计1.728秒 |
| 21:54:06.051起，episode2，seq1031–1059 | tick发布samplingUnavailable时上一样本年龄251ms；seq1033在约1ms后完成新getStats。邻近健康桶接收/解码/提交维持近60FPS、freeze delta0 | 采样节拍与健康发布交错产生的可用性告警，单独保留观察缺口 |
| 21:55:19.169起，episode3，seq2973–3026 | 一次255ms窗口仅received/decoded3帧；下一窗口视频packetsLost增440、NACK增1，1秒健康桶丢包15.86%、RTT4→27ms。Reader随后103ms等待，转移和消费各约1ms | 接收供给扰动，实际freeze213ms；恢复后提交重回约60FPS |

四次实际freeze合计1.941秒；三次episode累计7.302秒包含稳定确认与采样可用性窗口。全局累计视频丢包0.1483%，网络PASS反映整体状态，瞬时扰动单独保留。PLI2出现在启动期；NACK1后有关键帧与解码推进。Browser原生ReferenceChain、cleanAnchorCommitted/DisplayStable逐请求配对、NACK逐请求有效性与物理scanout继续按能力/缺证资格记录。

首次供给停顿伴随音频concealment，音频jitter buffer从约47ms升到280ms；21:54:27.976首次回到80ms以内，距1秒断供后视频重新推进约32秒，退出末窗56.67ms。Opus48k双声道direct route，concealed ratio0.295%、concealment5次，audio state ready、recoveryAttempts0。该值为WebRTC缓冲统计，实际播放/听感继续按实机观察确认。

## GPU与生命周期资格

稳态窗口各65个去重fence样本，submit→完成观察upper bound P95/P99由6/8ms到9/12ms。拆分后，submit→fence插入的中位数由0ms到3ms，插入后→完成观察P95/P99双方均6/8ms。增加主要落在fence插入时点；timer query扩展unavailable、lower bound均0，完成代理与物理上屏分别保留各自资格。全部100次完成、0timeout。

seq3138 Main资源归零：current/pending renderer、Reader、retained frame、frame/capture request、video/audio element、audio context/node均0。seq3139输入neutral接受；seq3153菜单退出清理409ms且flushed，pageUnmount清理28ms。`renderExecutionStopped`事件计数0：`browser-runtime.ts`的destroyClient先clearClientSubscriptions再close，`render-worker-host.ts`的异步retireWorker回调失去订阅。Main释放与输入关闭已有证据，Render Worker独立最终GPU/frame计数保留缺证。

自动报告同时消费135个dev interval，部分缺observationEpoch/配置，与production桶重叠；顶层205154ms仅作报告内部累计，实际串流109090ms。六维判断采用具备配置的epoch1连续production段，同时保留整段submit/present失败。trace budget notice、producer/backend丢失、malformed/duplicate row均0，ending PASS。

## 收口与建议

保留texImage2D与Reader→Render直连；优先修正采样竞态、同帧fence时点、输入心跳年龄口径及Render停机观察，再处理媒体断供后的音频缓冲回落长尾。受控繁忙收益、真实低源分辨率SR、用户截图、串流前后台与远端短按识别继续为待验资格。

三份日志报告已运行；一次证据冒烟核验完整gzip/SHA256、原始seq、指标与健康段一致性，结果记入checks.json。本轮专属临时目录按清单清理，原始冻结压缩日志与最终分析证据保留。

本轮临时目录已清理：10个文件、8.44MiB，目录存在性与磁盘空间已核验；所有分析命令已结束。保留完整冻结压缩日志和最终证据，暂存区保持原样。
