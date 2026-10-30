# 最新日志网络与渲染性能分析

2026-10-30。当前样本证明新 RVFC 身份全部交付，旧跨计数抵扣故障在本次会话中消除。1440p SR 稳态接收、解码约60fps，实际提交约43.5fps；缺口集中在解码后的回调/候选供给。SR 同步上传与命令提交均值14.84ms，进一步挤压回调节奏具有证据支持。网络另有一次集中丢包与冻结，以及分辨率切换附近的短时冻结。

## 范围与证据身份

- [最新日志](/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793329037119-1.jsonl)：北京时间10:57:17.119–11:00:57.899，220.780s、2398行。实际浏览器媒体统计覆盖10:58:00.668–10:59:38.842，98.174s、382次getStats采样、97份render telemetry。
- macOS debug、home、webrtc-direct、H264、60fps预算，schema3/dev。实际候选对为IPv4 UDP、srflx→srflx；有值的候选对ID保持同一个。6份统计缺少候选对，连接状态382份均为connected。
- 构建源码指纹`b5abf1b3723a4adbfaa3180e9ddbaf97282c30d995c00395ced1b1ee80537c87`与当前732个源码输入完全匹配，按`src-tauri/build.rs::source_fingerprint`算法复算。
- [统计数据](2026-10-30-network-render-trace-audit.metrics.json)与[分析脚本](2026-10-30-network-render-trace-audit.py)保存窗口、分母、原始字段和关键seq。本轮操作为日志分析与源码核对，应用操作保持暂停。
- 同日上午10:23日志包含启动/隐藏与关闭过程，媒体性能telemetry为0。前一日cloud日志用于核对旧抵扣故障；设备模式、网络、帧率与分辨率同时变化，优化收益以本次会话内的连续同尺寸窗口评估。

## 同窗供给与稳态吞吐

按renderer代际和实际源尺寸切开连续窗口，保留完整覆盖的3s/15s统计。下表取15s窗口间的中位数；原生1440p段15窗，SR1440p段20窗。窗口有重叠，数量表达观测覆盖。

| 指标 | 原生video，seq881–1499 | SR，seq1517–2215 |
| --- | ---: | ---: |
| 接收fps | 59.96 | 59.93 |
| 解码fps | 59.93 | 59.94 |
| video呈现推进计数fps | 59.96 | 59.90 |
| 接受回调/候选fps | 51.85 | 43.90 |
| 当前路径输出计数fps | 59.96 | 43.90 |
| 同步上传/命令提交均值ms | unavailable | 14.88 |
| 同窗最近提交年龄均值ms | 11.03 | 7.52 |

原生video的输出fps由帧源呈现推进计数投影，SR输出fps由实时draw计数形成。video计数表达浏览器/帧源进度；SR实际GPU命令提交与屏幕合成是两个阶段，真实屏幕帧率继续unavailable。

缓存统计使用sampledAtMs，即时mediaDiagnostics使用observedAtMs。seq1499的统计仍属video代际2，即时诊断已属SR代际3；脚本将其从video即时增量结算中剔除，完整策略窗口继续按其采样时间归属。

SR诊断按其自身单调时钟106528–141003ms对齐，34.475s内：原始RVFC1498、接受RVFC1498、轮询补帧0、marker1498、成功实时处理1498，全部对应43.4518fps。资源快照独立对照seq1522–2209：34.2s、1486次实时处理、43.4503fps。完整3s窗口提交为38.43–45.61fps；15s窗口为42.64–44.25fps，持续供给差值成立。

SR相对约59.94fps解码供给，实际处理覆盖约72.5%，缺口约16.5fps；15s窗的候选与提交一致，交付后的额外持续吞帧证据为0。原生1440p段候选中位51.85fps，切换SR后43.90fps，回调吞吐进一步下降约15.3%。

诊断累计4508次原始RVFC全部接受，`pollAlreadyDelivered=0`，接受轮询0，拒绝轮询5326。381条去重身份样本覆盖全部sampledCount；189条native样本均为advanced，188条RTP为advanced、1条为initial，mediaTime均为0。当前native去重资格有完整新身份进度，轮询观察到的播放计数在RVFC入口形成合批推进，多帧推进仍对应一次当前候选交付。

trackingSource全部为videoFrameCallback；sourceFpsEstimate全部unavailable，原因mediaTimeDeltaTooSmall，源节拍以完整接收/解码增量约60fps取证。SR35份采样累计callback gap4次、presentedFrames jump173次、dropped-like175次，样本边界包含切换瞬态。实际浏览器framesDropped全会话增量0，SR稳态freeze增量0。dropped-like主要记录多帧推进合批与少量回调间隔跨阈值；连续性指标droppedLikeStreak和单次最大呈现跨度缺少输出字段。

## 网络性能与异常时间线

98.174s全媒体窗口接收247718包、正丢失642包，按`lost/(received+lost)`归一为0.2585%。接收、解码均推进5773帧，平均58.80fps；平均视频RTP接收码率22.14Mbps。采样RTT中位17ms/P9538ms/max230ms，jitter中位5ms/P9512ms/max40ms。

网络常态与异常集中段分别取证：

| 时段/seq | 证据 | 解释 |
| --- | --- | --- |
| 10:58:18，seq555/566/576 | 丢失分别290/34/263包；RTT113ms；seq566接收/解码帧增量0，seq576仅推进1帧 | 集中丢包伴随帧供给冻结，三窗587包占全窗丢失91.4% |
| seq576→595 | freeze分别增加0.717s与1.010s，后者源尺寸1440p→720p、解码关键帧+2 | 网络异常后恢复与源尺寸变化相邻，恢复请求逐项配对继续缺证 |
| seq788/870 | 720p→1080p→1440p附近，freeze增加0.241s/0.232s，关键帧分别+2 | 分辨率切换与短时供给停顿相关 |
| seq1290 | freeze+0.215s，当前丢包增量0，接收+19帧、解码+14帧 | 局部供给/处理停顿继续独立取证 |
| SR稳态seq1509–2212 | 34.654s接收/解码2077帧，59.935fps；丢失25包/0.02422%；RTT中位13ms/P9538ms，jitter中位5ms/P959ms | SR持续43fps的同时，上游媒体保持约60fps |

全会话freeze累计5次/2.415s。网络突发支持解释局部冻结；SR稳态低提交率由其同窗候选和实际处理计数定位到解码后阶段。

SR稳态按帧加权的jitter buffer延迟5.92ms、解码4.27ms、processing10.21ms、assembly4.79ms。各约250ms统计窗平均解码耗时P954.44ms/max4.81ms；平均jitter buffer延迟P9511.41ms/max41.05ms。全会话最差seq576的jitter buffer/assembly均值约746ms，与丢包冻结窗口一致。上述阶段部分重叠；这些统计分位数描述采样窗均值，逐帧P95/P99和glass-to-glass继续unavailable。

## 渲染成本、年龄口径与启动回退

SR运行1440p源→1440p输出、passthrough、RCAS stops0.88；当前等尺寸路径执行video纹理更新与一个RCAS pass。资源快照记录3个GPU对象、0次截图、0次重复处理、0次pacing wait、0次提交失败，latest overwrite累计1次。同步上传与GL命令提交按1486次处理加权为14.8398ms，历史单次最大28ms；约占16.67ms帧预算的89%。逐帧CPU成本分位数、GPU执行时间、compositor呈现继续unavailable。

同尺寸原生段getStats异步采集耗时中位1ms，SR段中位22ms、最大26ms；切换瞬态seq1498达104ms，采样间隔734ms。该计时包含异步等待和主线程调度，支持SR期间调度压力增加的关联；纹理上传、GL提交和GPU等待的因果占比需要分段计时。

提交年龄存在明确的缓存老化口径：SR telemetry年龄中位161ms/P95253.6ms/max255ms；同窗统计的均值约7.5ms，18份当前资源快照的真实最近draw年龄中位8ms/max16ms。seq1763在118715ms读取118467ms统计快照，旧draw计数566、当前实时计数577，旧提交年龄被增长到255ms。`StatsService.snapshot()`将最新采样时的提交年龄加上采样年龄，采样间新增提交继续由下一次采样更新。

因此，百毫秒telemetry年龄表达缓存draw时间的老化估计；真实submit-to-present继续unavailable。性能判定优先使用同窗提交增量、采样时年龄与当前资源快照，并为缓存采样年龄和实时提交年龄建立独立字段。

另有启动回退：seq146在34089ms创建webgl2候选，seq170在39090ms记录rendererFailed，seq171实际提交video；seq189在42723ms才连接，首媒体在43s附近到达。5001ms与`PlaybackService.armReadyDeadline()`的5000ms挂载首帧期限吻合，支持媒体到达前首帧期限触发回退的解释。rendererFailed原始reason字段缺证，具体错误分类保留这一边界。初始规划pipeline仍为webgl2，实际管线以资源快照中的video为准。

## 恢复、门禁与结论边界

PLI增量2、FIR0、NACK70、关键帧解码11、RTX包846。有效关键帧已解码，浏览器内部请求抑制/合并、请求与响应配对、NACK及时修复/晚到/跳过/过期、ReferenceChain、cleanAnchorCommitted、DisplayStable、建链成功率、NACK有效率、repairability连续性及综合恢复评分均unavailable。RTX接收计数与NACK请求计数采用不同口径。

结构化pictureRecoveryTransition、pictureRecoveryBlockerObserved、videoIngressTermination、firstFrameLatencyObserved、H264逐包检查均为0，当前媒体由浏览器运行。Rust侧statsSnapshot维持priming/connecting和空媒体计数；该投影对应未承担本次媒体运行的引擎，诊断权威采用browserWebRtcStatsObserved、browserObservationConsumed及render telemetry。

browserWebRtcBehaviorGate和browserRuntimeStability均PASS，资源关闭归零、sharedSamples21。通用receiveFeedbackGate返回PASS，同时receiveFeedbackDecision、关键帧控制链和ReferenceChain样本均0；本次恢复内部闭环验收为unavailable。现有浏览器门禁证明资源、观测共享与配置边界，完整60fps输出验收仍由本报告供给差值单列。

97份telemetry的trace丢失0，排队最大2、在途最大4，traceBudgetNotice为0。网络unknown/pathStable多次切换包含候选对缺证与证据连续性变化；seq1448 RTT基线更新到175.64ms，随后SR段RTT中位13ms。网络基线受短时RTT尖峰的影响值得结合现有窗口准入继续核查。

## 建议执行顺序

1. 优先修正健康判据与采样年龄：持续约60→44fps的输出缺口在本次35份SR采样中仍有31份renderStable，另外4份frameDeliveryLimited。沿既有策略分别评估解码→候选与候选→提交，结合用户60fps预算、video观测基线和完整覆盖窗口；实时提交年龄与统计快照老化分开取证。
2. 在现有renderer补有界上传CPU、GL提交CPU、异步GPU timer及回调/提交间隔P95/P99。当前等尺寸单pass已形成最小SR路径，14.84ms成本需要阶段测量决定下一处优化；使用原生video→等尺寸处理→实际放大的同机同源阶梯验证输出吞吐与长尾。
3. 将候选首帧期限与可用媒体/首候选进度关联，并记录rendererFailed原因与实际回退。网络专项保留10:58:18突发窗口，继续核查RTT基线、短时缺证和尺寸变化期间冻结，按现有浏览器恢复可观测边界验收。

本次优先级为持续帧率缺口与年龄口径，随后是上传/提交阶段成本和启动首帧期限。设备样本对应macOS home，ROG1440p性能改善幅度继续使用其同源日志验收。

## 关闭超分时的共同瓶颈与指标语义

用户补充关闭/开启超分均感到提交率和呈现帧率不足。排查主线扩展为公共解码后显示路径：当前auto策略优先装配普通WebGL，启用SR再装配SR renderer；两种canvas路径共用VideoFrameSource、帧节拍准入和video纹理更新，持续供给瓶颈可以影响两种模式。

本次关闭SR期间，实际renderer已因启动失败回退video，日志覆盖的是video→SR的会话内对照。video段约60fps来自播放计数与统一帧推进量，实际候选交付约52fps；独立屏幕上屏事实继续unavailable。该样本支持定位公共回调供给不足，普通WebGL在关闭SR时的持续提交损失需要其实际管线采样补证。

当前`StatsService.sampleNow()`将canvas路径的`presentFps`赋为`rendererSubmitFps`；面板“呈现FPS”和“渲染提交/秒”在canvas路径共享draw计数来源。video路径的两项均由video推进计数投影。后续指标应明确实际提交、浏览器video进度与屏幕合成各自的观测来源，按接收→解码→候选→提交→实际显示逐段验收。

整改优先处理公共帧源和显示调度的供给完整性，再定位普通WebGL与SR共有的纹理上传成本；SR算法成本作为增强路径的额外分项继续评估。

复算：`python3 -B dev-docs/validation/2026-10-30-network-render-trace-audit.py <trace.jsonl> <output.metrics.json>`。脚本默认绑定本次日志和同名统计输出；后续代码变化会体现在sourceMatchesCurrent字段。
