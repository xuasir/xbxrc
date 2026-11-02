# 最新 browser-direct 日志：网络、渲染与像素取证

网络和解码维持约60fps，SR canvas的候选供给仍存在帧率缺口。两段SR完整3秒窗口提交中位数为51.50/54.25fps；最终原生video的56个完整15秒窗口推进中位数为59.94fps。像素探测首读24ms后触发8ms停止阈值，当前有效配对为0；本次同尺寸SR重建34ms，源尺寸切换继续待验。

采集日期为2026-10-30，分析收口日期为2026-11-02。已核对macOS应用日志目录、仓库`runtime-logs`及临时目录回退位置，最新可用文件为[runtime-trace-1793353382323-1.jsonl](/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793353382323-1.jsonl)。本轮接续关联聊天尚未收口的17:43样本。

日志范围17:43:02.323–17:46:26.438，共204.115秒、2829行；实际WebRTC统计范围17:43:43.226–17:45:59.512，共136.286秒、481个样本。SHA256为`03d8b97c9ebe55a87b5accfe23c16c9551f7563f04c7445e0a777d90546249d2`。737个源码输入指纹与日志构建一致：`c4398f8831ce4df5211588847608f4369c53cc338ef93b9a1851b21325f3714c`，构建commit为`85177066`。

全部行为统计采用browser-direct媒体事实。203个Rust单例`statsSnapshot`均为`session_phase=connecting`、`host_frame_present_epoch=0`；其帧率、恢复和上屏计数对应闲置原生管线。源与输出固定2560×1440，SR模式为`passthrough`、锐化为0；本轮覆盖这一canvas路径的供给与处理成本。

网络统计的首末inbound和SSRC身份各保持一个。窗口新增接收333289包、丢失133包，正丢失率0.039889%；接收59.940fps、解码59.947fps，平均视频码率21.434Mbps。481个连接样本均为`connected`，冻结、解码丢帧和暂停增量均为0。

| 网络/解码观测 | 中位数 | P95 | 最大值 |
| --- | ---: | ---: | ---: |
| RTT | 12ms | 17ms | 29ms |
| RTP jitter | 5ms | 7ms | 11ms |
| jitter buffer每采样窗平均延迟 | 5.207ms | 9.101ms | 47.799ms |
| 每采样窗平均解码时间 | 4.668ms | 5.027ms | 7.209ms |
| 每采样窗平均处理时间 | 9.924ms | 13.789ms | 54.513ms |

按帧数加权的buffer、解码、处理时间分别为5.143/4.558/9.724ms。表中分位数描述采样窗口均值；各延迟字段保留各自统计定义。前台完整区间129.005秒，接收59.943fps、解码59.967fps；后台完整区间5.039秒，接收59.933fps、解码59.734fps。前后台区间均保持0冻结，跨政策边界的区间单独排除。

相较[16:52样本](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-10-30-browser-frame-counter-pairing-fresh-trace-validation.md)，会话丢失率由0.318703%降至0.039889%，冻结由5次/2.516秒变为0。两份会话的网络状态与SR阶段长度分别记录，性能改善归因继续依赖同条件对照。

帧供给按renderer代际、前后台和完整覆盖窗口归一化。3秒/15秒窗要求接收、解码、video推进、候选与renderer五类覆盖均完整，统计窗口允许重叠。

| 阶段 | 完整窗口 | 解码fps中位数 | 候选fps中位数 | 输出fps中位数 | 同窗解码→输出差值中位数 |
| --- | --- | ---: | ---: | ---: | ---: |
| g1 SR，前台 | 4个×3秒 | 60.231 | 51.830 | 51.496 | 9.044fps |
| g2 video，前台 | 11个×15秒 | 59.924 | 56.558 | 59.932 | 0.028fps |
| g3 SR试探，前台 | 1个×3秒 | 60.492 | 54.254 | 54.254 | 6.238fps |
| g4 video，恢复前台 | 56个×15秒 | 59.944 | 53.861 | 59.942 | 0fps |

差值先在每个同窗样本计算，再取中位数。video输出定义为浏览器呈现计数推进；SR输出定义为成功本地处理。物理屏幕呈现及每张输出图像的唯一性继续独立取证。两段SR的完整15秒窗口均为0；g3整个试探仅3.982秒，当前结果覆盖短时供给。

同代际实时诊断增量进一步确认：g1末段5.017秒候选264→处理263，分别为52.621/52.422fps；g3的3.012秒候选165→处理165。g1资源累计`latestOverwriteCount=1`、`pacingWaitCount=1`，提交失败与重复绘制均为0；g3首提交前有一次latest覆盖，后续实时增量完整交付。持续缺口主要出现在解码→候选边界，steady候选→提交链保持接近完整。

首段存在一个需要后续对照的采集边界。seq184→seq243，首提交40,550ms至实时诊断55,108ms共14.558秒，候选与提交均新增436，输出29.949fps；对应网络统计seq188→seq238解码约59.982fps。首个telemetry累计437次RVFC、875帧推进、391次多帧跳跃；政策目标持续60fps，transport warmup记录为2200ms。诊断启动后的同代际增量升至约52.6fps候选，且seq250记录刷新率估计变化。

诊断启动前后的30/53fps属于已观测到的时间关联。窗口状态、DOM/compositor调度及采样启停均需固定条件验证。常规rAF在共享帧源中始终沿既有调度运行，当前代码的dev分支增加身份/像素诊断；设备上的刷新节拍影响继续由对照日志判断。完整g1资源区间35.709fps同时包含前述两个节拍阶段。

身份计数的466组有界配对中，播放质量计数有效466组，WebKit备用计数有效0组。累计471次主计数更新中462次在RVFC、9次在rAF；6922次rAF观察新增候选为0。g2/g3/g4恢复前台的同代际计数更新平均间隔约258.23/260.55/258.63ms。首个累计计数区间14,594ms跨越诊断启动，该区间采用左侧缺证解释。

`trackingSource`全部为`videoFrameCallback`。117个telemetry的`sourceFpsEstimate`均因`mediaTimeDeltaTooSmall`缺证，源节拍采用RTC接收/解码事实评估。有界native样本的`mediaTime`保持不变，RTP/native计数继续推进。SR g1/g3最大回调间隔112/49ms，callback gap累计1/0，多帧推进跳跃累计422/19，dropped-like累计422/19。video两段最大回调间隔161/151ms，也存在多帧推进；全会话RTC dropped与freeze增量均为0。dropped-like用于回调/计数合批诊断，实际GPU失败以提交与错误字段单独检验。

SR有界样本的`expectedDisplayLeadMs`中位数为−5/−13ms，表示回调相对浏览器预计显示点晚到；g1单次`rawNativeDelta`最大4帧，g3最大1帧。`sourceFrameIntervalMs`、`maxPresentedFramesDeltaSinceLastSample`及连续dropped-like streak在当前telemetry中缺证。独立`renderFrameDropped`、`renderBackpressureChanged`与`renderCauseClassified`事件族样本为0，当前归因采用`renderTelemetryObserved.renderCause`及累计候选/处理结果。

全部telemetry归因为`frameDeliveryLimited=110`、`renderStable=7`；当前归因计数主要反映callback与推进计数的差异。rAF最大间隔6112ms与6.090秒后台政策段相邻，前台指标按可见区间结算。

919次首提交后的成功处理，CPU调用平均总耗时0.491ms，其中上传0.457ms、提交0.029ms；上传继续使用`texImage2D(video)`。g1/g3稳态上传分别为0.443/0.500ms。首处理单独记录：g1总19ms、上传17ms；g3总22ms、上传22ms。这些数值覆盖同步API调用和本地处理，GPU执行、零拷贝及合成器排队继续缺证。

10个SR最近成功提交年龄样本：P50=11.5ms、P95=28.15ms、P99=32.83ms、最大34ms，超过50ms的样本为0。该字段描述采样时最新提交的年龄；submit→present、glass-to-glass及物理屏幕P95继续缺证。

像素探测在seq266首次观测到一次读取，16×9缩略图、576字节，耗时24ms。`disabledReason=readbackBudgetExceeded`，预算8ms；`baselineCount=pairCount=changedPairCount=0`，此后canvas与pending pair均为0，整个会话保持`readCount=1`。当前读取把canvas/context创建、`drawImage`与`getImageData`纳入同一计时区间，24ms的内部成本分项继续缺证。有效配对为0支持“观测不足”；RVFC缺席时的纹理提前更新机会继续待验。

新增准备计时已提供可复算的真实分段。以下耗时均来自同一renderer的单调时钟；旧资源释放采用当前毫秒记录精度。

| renderer切换 | 请求→提交 | setup | setup→首候选 | 候选→首处理 | 首处理 | 旧资源释放 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| g1，启动SR | 4692ms | 59ms | 4604ms | 0ms | 19ms | 启动 |
| g2，SR→video | 1ms | 原生video | 原生video | 原生video | 原生video | 1ms |
| g3，video→SR试探 | 34ms | 8ms | 1ms | 2ms | 22ms | 0ms |
| g4，SR→video | 1ms | 原生video | 原生video | 原生video | 原生video | 0ms |

启动setup于35,927ms结束，首候选于40,531ms到达；4604ms区间涵盖后续ICE/媒体准备，传输connected后的首帧通知约718ms。g3首处理22ms占同尺寸重建的主要时间。本次源尺寸始终2560×1440，此前467ms尺寸变化的准备/等待案例继续由尺寸切换日志验收。

seq185–187的首包/解码milestone在首个RTC采样中同时产生，`elapsedSinceConnectedMs`均为0；事件单调时钟40,558ms比连接通知约晚0.74秒。零值描述当前埋点字段，首样本检测时间和实际首包时间分别解释；startup复算采用原始事件时钟与首帧通知字段。

关键策略锚点保留原始seq和时钟：seq346（`tsMs=1793353442790`）持续供给缺口回退，seq435评估`improved`；seq931（`tsMs=1793353468899`）健康增强试探，seq1023（`tsMs=1793353472914`）试探失败再回退，seq1113评估`improved`。seq1233/1268分别在单调99,423/105,513ms切换后台/前台。seq2765在176,938ms正常关闭，video、renderer、回调、timer、像素探测与音频资源全部归零。

恢复证据采用浏览器可见计数与内部闭环分开结算。首个RTC样本已有PLI=2、关键帧解码=3、RTX到达157包；136.286秒媒体窗口再新增PLI=0、FIR=0、NACK=455、关键帧解码=4、RTX到达472包。NACK与RTX计数的粒度和配对关系分别保留，逐请求有效、迟到、跳过、过期结果均为unavailable。

按恢复证据顺序审计，`pictureRecoveryTransition`、`pictureRecoveryBlockerObserved`、`videoIngressTermination`、`firstFrameLatencyObserved`及H264内部检查事件均为0；`ReferenceChain`、`cleanAnchorCommitted`和`DisplayStable`也缺少浏览器内部事实。请求抑制/合并、usable response、建链成功率、NACK有效率、repairability连续性及综合恢复评分均按unavailable解释。summarizer的零样本派生零分保留为观测缺口。

默认receive报告为PASS，反馈决策样本为0。严格`--require-media-recovered --require-display-stable`退出2，两项必需闭环事实缺证。native midsegment脚本退出0，`MEDIA_SUPPLY_GATE`与`STEADY_SUPPLY_GATE`均为SKIPPED，年龄为n/a；`GLOBAL_LATENCY_GATE=PASS`对应闲置原生统计的无触发结果。实际稳态供给使用本报告的browser同窗结果检验。

browser行为与runtime稳定性门禁均PASS，24组共享样本、149个frame-chain样本及正常释放满足结构化验收；SR目标60fps继续由实际供给delta检验。schema v3全部为dev，essential/key/debug分别74/1755/1000；`traceBudgetNotice=0`，前端trace dropped与droppedEssential均为0。常规RTC采样CPU耗时P95=2ms、最大7ms；像素首读24ms作为独立采样成本保留。

复算结果保存于[metrics](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-02-network-render-fresh-trace-analysis.metrics.json)。[脚本](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-02-network-render-fresh-trace-analysis.py)沿用既有窗口算法，并独立核对170个完整窗口实例、2829个唯一seq、实时帧增量、成本加权、生命周期顺序及日志SHA，全部通过。

```sh
python3 -B dev-docs/validation/2026-11-02-network-render-fresh-trace-analysis.py /Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793353382323-1.jsonl dev-docs/validation/2026-11-02-network-render-fresh-trace-analysis.metrics.json
```

后续工作按以下顺序推进：

1. 拆分首次像素读取的canvas/context初始化、图像转换与回读耗时，保留8ms预算与已有退出机制，取得低成本有效配对。
2. 在同源、持续前台条件下固定采样预算，记录诊断/窗口状态切换，验证30→53fps调度关联及RVFC缺席时的drawable像素推进。
3. 取得60–120秒有效SR阶段、一次源尺寸切换及真实呈现证据，验收SR足帧、cold upload、submit→present和长期稳定性。当前流畅播放优先使用原生video；S4保持In Progress，实机采集由用户手动完成。
