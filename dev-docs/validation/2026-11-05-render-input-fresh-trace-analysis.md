# 新日志：渲染供帧与手柄输入

新日志确认输入发送和会话资源清理正常。增强渲染的主要缺口位于候选帧形成：健康网络下SR解码约60fps，候选47.79fps、提交47.46fps，自动试跑在约4秒后回退到video。GPU像素探测冷准备成功，233组有效配对取得相同像素，补交付0帧。

## 样本与身份

- [设备日志](/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793855239260-1.jsonl)：2026-11-05 13:07:19.260–13:15:26.553，487.293秒，2900行，7032656字节。
- SHA-256：`2dfb6598e7d817b598f2bf31af664ac69803163208298b43fb991c33b2964e5a`，解析期间字节身份稳定、完整换行、seq连续。
- Browser/debug、main基础提交`82e1fed2`，构建源码指纹`31ccfc435c6caab34d12f128b3c021f4fb4285dd421c6a5f7157d43e78510e96`与当前源码按build.rs完整算法重算一致。
- 用户补充测试包含菜单及有运动画面的游戏库场景，并确认串流早期曾切换或遮挡应用窗口。逐段运动状态尚无日志标记。
- [可复算指标](2026-11-05-render-input-fresh-trace-analysis.metrics.json)与[分析脚本](2026-11-05-render-input-fresh-trace-analysis.py)独立保存；历史设备样本和自动回放保持原身份。

本次会话getStats有效区间为13:12:34.455–13:15:01.728，共147.277秒。报告将早期遮挡段、各renderer代次及完整3秒/15秒窗口分别计算。

## 渲染阶段

| 阶段 | 同代际即时采样跨度 | 候选→提交 | 即时提交fps | 完整3秒窗提交fps中位值 | 结果与背景 |
| --- | ---: | ---: | ---: | ---: | --- |
| 普通锐化g1 | 13.051s | 739→734 | 56.24 | 57.20，10窗 | 首阶段总长45.875s，早期包含窗口遮挡与3次freeze；后段回退video |
| video g2 | 27.109s | 沿video呈现计数 | 约60 | 59.95 | 完整15秒窗60.00fps |
| 普通锐化g3 | 14.059s | 814→808 | 57.47 | 58.23，12窗 | 解码约60fps，持续缺口后回退video |
| video g4 | 2.009s | 沿video呈现计数 | 约60 | 阶段不足完整采样 | 用户切换SR |
| SR g5 | 6.025s | 244→243 | 40.33 | 30.70，4窗 | 同阶段接收/解码约45fps，包含丢包突增与1.691s freeze |
| video g6 | 17.069s | 沿video呈现计数 | 约60 | 59.88 | 完整15秒窗59.94fps |
| SR g7 | 3.013s | 144→143 | 47.46 | 46.98，1窗 | 健康网络、解码约60fps；约4秒试跑后回退video |
| video g8 | 25.100s | 沿video呈现计数 | 约60 | 59.86 | 完整15秒窗59.96fps |

窗口fps来自同代次的Browser事实，完整覆盖接收、解码、video、callback与renderer。SR阶段g5/g7均短于15秒，长期稳态验收继续需要独立长段。SR实际源和输出均为2560×1440，FSR1记录为passthrough，当前结果覆盖同尺寸锐化路径。

关键健康对照是g7，seq2242→2308：解码59.76fps、候选144帧、提交143帧，候选交付率99.31%。完整3秒窗的接收/解码/video/候选/提交依次为60.19/59.86/59.86/47.32/46.98fps，主要delta在video→candidate。g3候选814→提交808，交付率99.26%，缺口同样主要在候选之前。

同阶段CPU绘制、纹理上传P95/P99约1ms，g3平均绘制0.277ms、上传0.264ms；g7平均绘制0.274ms、上传0.247ms。所有canvas renderer的submitFailureCount和duplicateDrawCount为0。GPU timer扩展状态为unavailable，因此GPU执行时间和物理submit-to-present保留设备观测边界。

提交间隔有长尾：g3最近窗口P95/P99为33/35ms，最大51ms；g7为34/50ms，最大51ms。g5最大1701ms与网络freeze对应。g1生命周期最大绘制637ms、提交间隔1835ms，位于包含遮挡和恢复的首阶段；其结束快照的最近256项绘制P95为1ms。

## 像素补供给与自动回退

探测前置准备共5次，最大20ms、累计42ms。首次cold读取CPU13ms、GPU等待4ms，cold最大13ms低于50ms预算，warm最大6ms低于8ms预算，运行状态ready且disabledReason为空。先前设备日志的冷读取超预算停用现象在本次有效探测区间得到修复验证。

全会话readCount2050、completedReadCount2049、pairCount233、changedPairCount0、deliveredFrameCount0。g3即时区间读取537次、配对21组、健康跳过302次；g7读取179次、配对35组、补交付0。累计健康跳过414次，降频机制有执行证据，新增drawable确认在本次配对窗口内保持0。

配对样本RGB差值、changedPixelCount与maxChannelDelta均为0。这支持“这些异步采样窗口取到了同一可绘制图像”的事实。菜单与运动场景混合、缺少逐段运动标记，对WebKit视频可绘制图像更新时机和配对资格的因果解释继续作为待验证假设。

seq738/1643/1853分别在约3秒持续缺口后计划回退；seq2218计划健康增强试跑，seq2300在完整3秒覆盖后以enhancementTrialFailed回退。相关动作均带sampleId、rendererGeneration及覆盖证据。试跑g7结束后，g8完整15秒输出约59.96fps，确认video路径保住了帧供给。

优先检查共享帧源候选资格、HTMLVideoElement→纹理导入的图像更新时机和像素配对收益，再复核增强试跑/回退依据对网络freeze及静止场景的归属。

## 手柄采样与输入交付

早期手柄轮询约1Hz、全生命周期最大采样间隔1586ms，seq394。用户确认曾切换或遮挡窗口，该异常与遮挡测试阶段一起记录。全媒体区间平均146.76Hz混合了这一段与后续稳定段。

从首次渲染telemetry之后留出3秒，让遮挡阶段间隔退出256项滚动缓存，后续111.606秒、seq564→2840的累计轮询为184.58Hz、快照发布9.77Hz。采样间隔P95快照均为9ms，P99快照中位9ms、最大20ms，最近窗口最大间隔33ms。4ms配置由rAF与setInterval共同执行，实际getGamepads调用频率由本平台调度结果记录；硬件报告频率单独测量。

| 输入指标 | 本轮结果 | 口径 |
| --- | --- | --- |
| queueWait / observationToSend | 最后快照P95/P99均1ms，生命周期最大9ms | 最近256项分位数及服务生命周期最大值分别保存 |
| 浏览器排空年龄 | 最后P95/P99为2/3ms，最大33ms | 浏览器接受字节的排空观测 |
| 当前bufferedBytes | 中位99B，观测最大543B | 136份周期快照 |
| 最旧缓冲年龄 | 观测最大1ms | 明显低于50ms年龄预算 |
| pendingInputs | 全部快照为0 | 当前待发状态 |
| physicalSampleAge | 最后P95/P99为85/103ms，最大104ms | 稳定状态100ms心跳影响最新快照年龄 |

gamepadFrameSent1049条。输入发送与排空保持通畅，采样调度节奏及稳定状态心跳年龄分别解释；端到端按下到游戏画面响应接续外部测量。

## 网络、恢复与首帧

147.277秒统计区间接收58.66fps、解码58.26fps、平均10.67Mbps。丢失530包，占0.2914%；RTT中位13ms、P95/P99为28/80ms，jitter中位2ms、P95约10.5ms、最大62ms。491份getStats的connectionState均为connected。

会话记录4次freeze共5.367秒：seq357为0.664s、seq397为1.834s、seq419为1.178s，位于早期遮挡阶段；seq1823为1.691s，属于SR g5。g5的seq1807出现单次360包丢失，framesReceived/Decoded增量均0，jitter48ms；seq1823随后关键帧解码增量1、解码16帧，媒体供给恢复。

PLI增量2、FIR0、NACK85、关键帧解码11、重传包792。Browser记录支持反馈和恢复结果观察，ReferenceChain、RecoveryLedger、cleanAnchorCommitted、DisplayStable及NACK逐包有效率由WebRTC内部观测边界限定。空native receive-feedback门禁保持适用性说明。

启动主要等待远端sessionReady约17.0秒，seq234→267；seq319连接，seq326/327约390ms后记录解码与呈现milestone。H264选择constrained-baseline。周期getStats首包/首解码约1002ms属于采样首次发现，首帧milestone提供更细时间锚点。

## 日志完整性与资源清理

schema v3/dev全部2900行，essential/key/debug为94/1808/998。fileOpened记录writerThread=`runtime-trace-writer`、8192行/8MiB pending预算和64KiB行预算；traceBudgetNotice为0。前端trace dropped/droppedEssential均0，pending最大3、inFlight最大4，前端IPC队列与Rust pending预算分别记录。

seq2859 clientClosed后，renderer、frameRequests、native/rAF回调、像素canvas/read/timer、video元素与audioContexts均为0。输入释放3ms、runtime停止18ms、remoteCleanup约1020ms。日志尾部继续有应用级手柄采样，当前关闭证据覆盖串流会话生命周期。Pointer多帧编码与setup失败退出码接续已有定向回归证据。

Browser基础行为门禁PASS，严格稳定性门禁为MISSING，原因是sharedSamples：169条browserObservationConsumed均属于supervision，门禁要求同一身份同时有panel和supervision消费。完整3秒/15秒窗口与资源证据可独立分析；共享消费验证应补齐panel取样身份。

默认native midsegment窗口+79–150秒早于本次串流开始，且当前Browser日志缺少native statsSnapshot/host epoch，MEDIA_SUPPLY与STEADY_SUPPLY为SKIPPED。本报告使用上述同renderer代次的Browser窗口覆盖media/steady supply，并将物理present证据单独保留。

## 复算与下一步

```sh
python3 -B dev-docs/validation/2026-11-05-render-input-fresh-trace-analysis.py /Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793855239260-1.jsonl dev-docs/validation/2026-11-05-render-input-fresh-trace-analysis.metrics.json
```

源码匹配、日志SHA/seq、同代次候选→提交增量、完整窗口及结束资源检查通过。analysisConsistencyGate表示复算一致性；增强渲染供帧质量沿本报告的缺口与窗口证据判断。

后续优先顺序：候选/像素配对的实际drawable证据与收益；增强回退对供帧缺口、网络freeze和静止画面的归属；panel/supervision共享采样身份。持续运动场景分别固定video、普通锐化、SR各60秒，按同一网络和尺寸比较接收→解码→video→candidate→submit，并补充物理present与端到端手柄响应。
