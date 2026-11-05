# 调度简化后的新设备日志

新日志确认固定时隙限帧引入了明显的候选→提交损失。首段SR累计352个候选，提交268个、限帧跳过84个，占23.86%；同代际即时区间候选53.14fps、提交40.19fps。网络与解码约59.84fps，video完整15秒窗口约60fps，输入发送与会话清理正常。

## 样本与身份

- [设备日志](/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793860966510-1.jsonl)：2026-11-05 14:42:46.510–14:45:27.074，160.564秒，1773行，4215594字节。
- SHA-256：`c83a7568a8b8e520f962943205ec030a2ee5922e39e5e6473813c5c5e4b0f038`，完整换行、seq 1–1773连续，分析前后字节身份一致。
- Browser/debug、main基础提交`82e1fed2`，构建源码指纹`7c957f1437f6e863370d5720c914702049927d12945c60f42c865a28bf81ba5b`与当前源码按build.rs完整算法重算一致。早期BASE脚本的历史输入范围较窄，源码身份以本次完整重算为准。
- compiledPlan确认`targetType=home`，Browser物理手柄、配置250Hz，初始渲染预算60fps。全程只有一次初始预算设置，四个renderer阶段均记录visible=true、target60。
- SR源和输出均为2560×1440，FSR1模式passthrough，覆盖同尺寸增强绘制。普通锐化、云游戏及实际放大SR沿独立设备样本验收。当前日志缺少逐段运动画面与窗口遮挡的用户确认，前轮测试场景沿原报告保留。
- [指标](2026-11-05-latest-frame-scheduling-fresh-trace-analysis.metrics.json)与[复算脚本](2026-11-05-latest-frame-scheduling-fresh-trace-analysis.py)保存同代际增量、完整窗口、资源配平与生产FramePacer回放。

## 渲染供给与稳态窗口

getStats有效区间为单调时间74420–147568ms，共73.148秒。下表即时帧率按mediaDiagnostics即时观测时间计算；3秒/15秒窗口按同一renderer代次完整覆盖计算。

| 阶段 | 阶段时长 / 即时采样跨度 | 即时候选→提交 | 完整3秒窗输出fps中位值 | 完整15秒窗输出fps中位值 |
| --- | ---: | --- | ---: | ---: |
| SR g1 | 6.717s / 6.022s，seq259→393 | 320→242，53.14→40.19fps | 41.16，4窗 | 阶段不足15秒 |
| video g2 | 18.101s / 17.069s | 880候选回调，输出沿video呈现计数 | 59.89，15窗 | 59.99，3窗 |
| SR g3 | 3.986s / 3.013s，seq785→852 | 167→167，55.43→55.43fps | 55.32，1窗 | 阶段不足15秒 |
| video g4 | 44.195s / 43.172s | 2414候选回调，输出沿video呈现计数 | 59.88，41窗 | 59.96，29窗 |

g1完整3秒窗的video→renderer供给delta中位18.99fps，其中candidate→submit中位12.76fps。g3对应delta为4.88fps，candidate→submit为0。两个SR阶段均短于15秒，增强稳态满帧验收继续待完成；video的15秒滚动窗提供稳态输出证据，窗口之间存在重叠。

### 限帧损失的精确配平

seq389、单调时间81413ms的资源快照记录g1：源native回调累计352、轮询新增候选0、最后提交序号352、realtimeDrawCount268、rateLimitDropCount84、提交失败0、重复绘制0、renderer动画等待0。seq393的即时身份在同一观测时间确认localSequence352，初始候选序号1：

```text
352个候选 = 268次提交 + 84次限帧跳过
限帧跳过比例 = 84 / 352 = 23.86%
```

seq847、单调时间103502ms的g3资源快照与seq852即时身份同刻对齐：初始缓存候选序号1280、最终1500，共221个候选，219次提交、2次限帧跳过。g3的即时seq785→852区间限帧计数保持2，167个候选全部提交。

顶层周期统计与mediaDiagnostics即时读取存在时差，例如seq259采样时间75212ms、即时身份75391ms。即时320→242的78帧缺口，与两个顶层rateLimitDropCount的差值分别按各自采样边界解释；累计352=268+84使用对齐资源时刻完成归因。

### 固定时隙的相位冲突

生产规则以首绘时间作为固定起点，计算`Math.round((atMs - startedAtMs + 1) * targetFps / 1000)`，同槽后续候选直接跳过。g1首绘起点74675ms，抽样native证据中的79416→79432ms与79916→79932ms均相隔16ms。将这两组入口时间交给生产FramePacer，分别落在285/285和315/315槽，后一候选被跳过。

| 生产类离线回放 | 候选数 | 提交数 | 限帧跳过 | 证据用途 |
| --- | ---: | ---: | ---: | --- |
| 模拟稳定60fps、毫秒量化时钟 | 600 | 600 | 0 | 稳定相位基线 |
| 模拟60fps、运行后相位移动7.33ms、交替±1ms抖动 | 600 | 331 | 269 | 验证固定相位遇到轻微交替抖动会损失可用候选 |
| 模拟53fps、相位移动7ms、交替±4ms抖动 | 530 | 453 | 77 | 验证低于60fps上限的供给仍会被抽掉 |

模拟回放的比例描述该组构造时序；设备实际损失由资源计数给出23.86%。原始trace记录抽样入口时间，admit调用时间与逐帧决策仍缺证，因此具体84帧各自落在哪个槽保留观测边界。设备计数证明限帧规则直接造成损失，生产回放证明该规则存在相位敏感性，g1/g3的明显差异与这一机制一致。

前轮±3ms稳定相位、突发和显式30fps回放已通过。新回放补出了运行中相位移动与交替抖动的盲区，自动合同验证和设备供给验收沿独立结果记录。

### 耗时、回调与回退

g1首绘CPU24ms，其中纹理上传22ms；后续平均绘制0.438ms、上传0.386ms、提交0.041ms。g3首绘23ms；后续平均绘制0.569ms、上传0.532ms、提交0.032ms。两个阶段结束快照的最近绘制P95/P99均1ms。GPU timer扩展为unavailable，GPU实际执行和物理submit-to-present仍待测量。

g1最近提交间隔P95/P99为49/67ms，最大110ms；g3为33/34ms，最大50ms。已采到的新候选可以立即提交，固定槽拒绝会扩大提交间隔。

trackingSource全程videoFrameCallback。g1即时窗口native观察320次、接受320次，rAF观察360次、接受0次；g3对应167/167和181/0。g1的320次回调伴随presentedFrames推进366帧、46次多帧推进；g2的880次回调伴随推进1023帧、143次多帧推进；g4的2414次回调伴随推进2588帧、174次多帧推进。浏览器呈现推进约60fps，JS候选回调约52–56fps，两层计数共同解释video→candidate缺口。dropped-like计数与多帧推进相关，真实GPU绘制失败由submitFailureCount单独取证。

g1即时native最大间隔84ms，顶层阶段采样最大间隔101ms；g3采样最大50ms。sourceFpsEstimate/rawSourceFpsEstimate全程为空，原因mediaTimeDeltaTooSmall；有效节拍以getStats、呈现推进和回调实测为准，RTP身份仍持续推进。

seq386以sustainedRendererPressure计划回退，seq388切换video；seq755在健康15秒后计划增强试跑，seq757切入g3；seq845以enhancementTrialFailed回退，seq846切换video。三次动作均具有完整覆盖证据，video输出恢复约60fps。g1回退前主要存在限帧准入损失，g3试跑失败前主要存在候选供给缺口；对两段使用分层原因更有诊断价值。

## 像素诊断

像素准备在75402–75407ms完成一次，共5ms，发生在g1首绘之后；初始off采样的读取和准备计数均0。像素导入已退出renderer初始化和候选交付链。

最后累计读取/完成141次、有效配对12组、变化0、补交付0，运行模式diagnostic。按首读到最后即时观测计算约1.95次读取/秒；首次冷CPU读取9ms，warm最大3ms，整体CPU读取P95/P99为2/3ms、GPU等待P95/P99为3/3ms。当前配对窗口观测到相同像素，逐段运动标记缺证，持续运动场景的drawable更新继续单独验收。

## 手柄与输入

媒体有效区间完整采样窗覆盖72.699秒，实际轮询184.75Hz、快照发布9.77Hz；留出首次telemetry后3秒的区间为184.79Hz。采样间隔P95快照均9ms，后段P99快照为9–10ms，最近窗口最大33ms；全媒体区间最大43ms。串流有效区间的完整采样窗均高于100Hz。1100ms生命周期峰值在串流前seq87出现，关闭后尾部约1Hz采样沿应用阶段单列，当前测试的遮挡原因缺少用户确认。

71份输入交付快照记录queueWait/observationToSend最后P95/P99均1ms，生命周期最大值也均1ms。bufferedBytes观测最大127B，最旧缓冲年龄全部0、pendingInputs全部0；最后排空年龄P95/P99为2/3ms，生命周期最大19ms。gamepadFrameSent共641条，输入发送与排空保持畅通。

physicalSampleAge最后P95/P99为82/95ms、最大106ms，包含稳定输入100ms心跳时的源快照年龄。该指标与实际新输入的发送等待分别解释；浏览器API调用频率、硬件报告频率及按下到游戏画面响应沿不同测量口径验收。

## 网络、恢复与首帧

接收和解码均59.8376fps，各4377帧，framesDropped0、freeze0、pause0。平均14.995Mbps，丢失382包，占0.30275%；RTT中位11ms、P95/P99为20/24ms，jitter中位2ms、P95/P99为7/9.1ms、最大15ms。291份connectionState均connected。

加权平均jitter buffer3.163ms、解码4.179ms、processing7.365ms、assembly2.627ms。接收/解码全程供给正常，首段SR约40fps提交的新增缺口已由候选与限帧计数分层定位。

反馈计数PLI+1、FIR0、NACK48，关键帧解码5、重传542包。Browser日志覆盖请求、重传与解码/冻结观察；ReferenceChain、RecoveryLedger、cleanAnchorCommitted、DisplayStable及NACK逐包处置/有效率由WebRTC内部观测边界限定。native总结中的有效率0和综合评分0来自空事件集，恢复结论以本段实际媒体事实为准。

transportConnected单调74130ms，首个frameDecoded/framePresented均74674ms，相差544ms；周期getStats首次发现解码晚于帧入口，首帧体验沿更细的阶段milestone取证。当前样本成功启动和关闭。

## 日志与清理验证

schema v3/dev共1773行，essential/key/debug分别132/1022/619。writerThread为runtime-trace-writer，pending预算8192行/8MiB、单行64KiB，traceBudgetNotice0；前端pending最大3、inFlight最大4、dropped/droppedEssential均0。

seq1747、单调147698ms的clientClosed快照中，renderer、源/renderer回调、帧准备、像素canvas/GPU读取/定时器、视频、音频、麦克风、截图等会话资源计数全部0。powerOff路径输入释放4ms、runtime停止11ms、remoteCleanup1075ms且localResourceReleased=true；随后pageUnmount再次释放输入20ms、runtime停止1ms，均success。

Browser行为与严格运行态稳定性门禁均PASS：110条共享消费记录中supervision74、panel36，sharedSamples1、freshObservationEvents109，failures和missingEvidence均为空。前轮共享消费缺证在新样本中得到补齐。该门禁覆盖观察身份、生命周期和动作合同；SR供帧问题由上面的设备delta与完整窗口继续判定。

默认native midsegment窗口+79–150秒内statsSnapshot0，MEDIA_SUPPLY/STEADY_SUPPLY为SKIPPED。native空证据PASS沿适用性单独记录，本报告使用Browser完整3秒/15秒窗口评估media/steady supply，submit/present长尾的物理部分待设备测量。

## 复算与建议

```sh
python3 -B dev-docs/validation/2026-11-05-latest-frame-scheduling-fresh-trace-analysis.py /Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793860966510-1.jsonl dev-docs/validation/2026-11-05-latest-frame-scheduling-fresh-trace-analysis.metrics.json
```

源码指纹、日志SHA/seq、同代际候选→提交、资源配平、生产相位回放和结束资源清零复算通过。analysisConsistencyGate表示分析一致性，SR设备帧供给沿本报告的回归结论记录。本轮只新增分析脚本、指标与文档。

优先让常规源帧率下的有效新候选直接放行；显式低FPS上限或可证实的超额供给使用独立轻量限流，并补充相位移动、低于上限的抖动供给与突发回放。随后固定video/普通锐化/SR各60秒，核验同窗received→decode→video→candidate→submit、限帧跳过与提交间隔P95/P99；video→candidate的剩余缺口继续围绕WebView回调/可绘制图像更新取证，并补齐物理present与端到端输入响应。
