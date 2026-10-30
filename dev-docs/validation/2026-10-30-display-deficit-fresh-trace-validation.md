# 显示帧差修复后新日志验收

2026-10-30 14:39新构建。原生video回退、效果验证和一次增强试探闭环已经在实机日志中成立；原生video稳态推进计数约59.94fps，SR两次实际提交均约44.6fps。视频纹理上传调用均值14.67ms，占同步处理约99%，成为后续性能优化的首要对象。保留SR效果并达到60fps的性能目标继续验收。

## 范围与源码身份

- [日志](/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793342364772-1.jsonl)：北京时间14:39:24.772–14:42:08.693，163.921秒、2269行，schema3/dev。浏览器媒体统计359次，覆盖14:40:28.658–14:41:59.472、90.814秒；render telemetry 91份。
- macOS/home/webrtc-direct、H264、2560×1440源与输出、60fps预算，SR为等尺寸passthrough、单RCAS pass、stops0.88。应用媒体区800×568 CSS像素，SR canvas800×450、backing2560×1440；源/输出尺寸与显示状态均有结构化证据。
- 构建fingerprint `68f18dd5d500af8fb25601b90fca337679c808b10136287896fa96c94e30b1ab`，按 `src-tauri/build.rs` 算法复算736个源码输入，完整匹配当前工作树。日志SHA256、窗口分母、计数差和原始锚点保存在[metrics](2026-10-30-display-deficit-fresh-trace-validation.metrics.json)，由[脚本](2026-10-30-display-deficit-fresh-trace-validation.py)复算。
- 本次源码和会话包含前轮显示修复及公共启动修复；分析采用本次会话内同源、同尺寸的SR↔video对照。上午日志码率约22.14Mbps，本次约10.56Mbps，跨会话改善幅度按其各自场景保留边界。

## 同窗供给

下表为完整3秒监督窗口的中位数，窗口重叠；SR首次4窗、原生video首次15窗、SR试探1窗、最终video58窗。

| 管线/代际 | 接收fps | 解码fps | video推进fps | 候选fps | 当前路径输出fps |
| --- | ---: | ---: | ---: | ---: | ---: |
| 首次SR/g1，seq264–391 | 59.90 | 59.98 | 59.65 | 44.67 | 44.67 |
| 原生video/g2，seq411–781 | 59.89 | 59.88 | 59.89 | 57.13 | 59.89 |
| SR试探/g3，seq799–869 | 59.95 | 60.06 | 59.39 | 45.29 | 45.29 |
| 最终video/g4，seq893–2225 | 59.88 | 59.86 | 59.88 | 56.47 | 59.88 |

最终video有46个完整15秒窗：接收中位59.95fps、解码59.94fps、video/输出推进59.94fps，输出区间59.83–60.03fps；候选回调中位54.86fps。供给验收使用完整窗口，约250ms短采样内55/63fps等瞬时波动由采样边界和合批推进共同形成。

SR原始同代际资源增量另行结算：首次7.119秒、317次实时处理，44.53fps；试探4.010秒、179次实时处理，44.64fps。即时诊断对照分别271个新候选→271次处理、137个新候选→137次处理，候选交付后的持续额外吞帧证据为0。完整窗口的缺口集中在约60fps解码→约45fps候选，约25%的可用源供给受到影响。

原生video输出来自浏览器/帧源推进计数，SR输出来自成功实时处理计数。物理scanout、GPU执行与submit-to-present继续unavailable。

## 回退闭环与首帧修复

| 时间/seq | 实机证据 |
| --- | --- |
| 14:40:19.157，seq186 | SR候选创建，尚无媒体，readyTimers=0。 |
| 14:40:28.834，seq240 | 首帧后提交SR。候选到提交的单调时钟间隔9723ms，rendererFailed为0；等待媒体期间的首帧期限修复得到验证。 |
| 14:40:35.953，seq384/387/390 | 完整3秒供给覆盖、持续3044ms欠帧触发 `sustainedFrameDeliveryDeficit`，切换并提交video/g2。 |
| 14:40:39.970，seq477 | 新代际有完整3000ms效果窗口，回退结果 `improved`。 |
| 14:40:54.026，seq775/778/782 | 健康持续15059ms、覆盖15000ms，执行唯一一次 `healthyEnhancementTrial`，提交SR/g3。 |
| 14:40:58.078，seq862/863/868 | SR新代际完整3秒输出仍约45fps，`enhancementTrialFailed`，恢复video/g4；效果记录为neutral。 |
| 14:41:02.094，seq959 | 再次验证回退效果improved，至退出保持video。 |
| 14:41:59，seq2232/2239/2240/2249 | 菜单正常退出，clientClosed资源归零，runtimeStopCompleted，远端会话closed=true。 |

软动作共3次，两次回退与一次试探均有完整证据；试探失败后的保真回退即时执行。超分用户意图保留，回退期间实际增强暂停。此次回退效果和有限试探已经有设备证据，增强60fps吞吐继续作为性能缺口。

## 上传成本与提交年龄

资源差统计覆盖496次实时处理，分段计数完整：

| CPU调用计时 | 首次SR/g1 | SR试探/g3 | 合并加权 |
| --- | ---: | ---: | ---: |
| 处理总耗时ms | 14.84 | 14.79 | 14.82 |
| 视频纹理上传ms | 14.67 | 14.66 | 14.67 |
| GL命令提交及其后处理ms | 0.164 | 0.123 | 0.149 |

上传占同步处理98.95%，单次历史最大首次26ms、试探32ms。当前SR上传入口为 `SuperResolutionWebGL2Renderer.ts:869` 的 `uploadVideoSource()`，等尺寸逐帧调用 `texSubImage2D(... RGB ..., HTMLVideoElement)`；上下文使用preserveDrawingBuffer=false、high-performance。后续优化应针对视频纹理导入、格式转换、拷贝和同步等待。调用耗时可以包含驱动/GPU等待，GL提交CPU计时表达调用侧成本；GPU shader执行耗时继续缺证。

SR期间getStats异步采集耗时中位22ms/P9524ms，video期间中位1ms/P952ms；它包含主线程调度与异步等待，支持上传路径压力的关联。逐帧CPU耗时P95/P99继续缺证，累计最大值与采样窗均值保留各自口径。

SR telemetry实时提交年龄11份：中位7ms/P958ms/max8ms；新增年龄观测时间与实时资源同代际对齐。上午缓存年龄中位161ms的虚高口径已经修正。此年龄描述最近成功处理的新鲜度，真实提交→呈现长尾继续unavailable。

## 网络、恢复和回调证据

- 90.814秒接收111023包、正丢失43包，丢失率 `43/(111023+43)=0.03872%`。接收/解码均推进5439帧，59.89fps；平均视频接收码率10.56Mbps。359份连接状态均connected，有值候选对ID保持同一个，UDP/prflx→srflx；2份候选对缺证。
- RTT中位11ms/P9515ms/max17ms；jitter中位2ms/P955ms/max11ms。按帧加权jitter buffer2.04ms、解码4.09ms、processing6.15ms、assembly1.62ms；各阶段部分重叠，采样窗均值的分位数与逐帧长尾分别取证。
- freeze增量1次/0.212秒，seq532、14:40:42.104：接收+20帧、解码+16帧、丢包+1、NACK+1、RTX+1，RTT15ms/jitter11ms，jitter buffer窗均值48.58ms。相邻seq522帧供给+10，seq536解码+19；局部供给停顿、批量到达和缓冲积累相邻，逐包修复配对继续缺证。SR两段freeze增量均0，全程framesDropped增量0。
- PLI累计1且起始媒体统计已含该请求，统计窗口PLI增量0、FIR0、NACK+37、解码关键帧+6、RTX包+58。pictureRecoveryTransition、pictureRecoveryBlockerObserved、videoIngressTermination、firstFrameLatencyObserved及H264逐包事件均0；浏览器内部请求合并/抑制、响应与decode配对、ReferenceChain、cleanAnchorCommitted、DisplayStable、NACK及时性与有效率、建链成功率、repairability连续性和恢复综合评分继续unavailable。
- trackingSource全部videoFrameCallback，sourceFpsEstimate均缺证、reason=mediaTimeDeltaTooSmall；源节拍采用完整接收/解码增量取证。采样身份180条native全部advanced，RTP179条advanced/1条initial；诊断累计4900次原始RVFC全部接受，轮询候选接受0，诊断采集开始前已有2个localSequence。
- 首次SR采样callback gap0、jump33；SR试探gap0、jump17。原生video/g2 gap2、最大回调间隔216ms，邻近上述freeze；最终video/g4 gap0、最大间隔84ms、jump283。SR最大回调间隔分别50ms/46ms。dropped-like按gap/jump并集统计，连续streak和单次最大presentedFrames跨度缺少telemetry字段；屏幕实际丢帧继续独立验收。
- SR试探前两份renderStarvation分别只有525ms/1584ms renderer覆盖，候选窗口仍含前代际video回调；完整3秒窗口分类为frameDeliveryLimited。原生video的该标签描述候选回调供给，健康验收采用video推进/源预算。窗口覆盖与实际输出应共同解释诊断标签。

browserWebRtcBehaviorGate与browserRuntimeStability均PASS，sharedSamples8、3次软动作完整配对、退出资源归零。receiveFeedbackGate返回PASS，同时关键恢复样本为0，内部恢复闭环验收为unavailable。trace无budget notice，telemetry丢失0，排队最大3、在途最大4；本次证据完整性通过。

## 后续优先级

1. 优先优化现有HTMLVideoElement→WebGL纹理上传路径，采用同机、同1440p源、相同画面与60fps预算比较导入格式/调用方式，记录上传成本、候选→处理交付与是否发生回退。
2. 补可用时的异步GPU timer，以及有界逐帧上传/回调/提交间隔P95/P99，区分导入转换、拷贝和等待占比；使用当前相同renderer主线实施。
3. 完成保持SR效果的60fps实机验收，并继续ROG1440p、真实上屏和2h/8h稳定性测试；自动实机操作保持暂停。

复算：`python3 -B dev-docs/validation/2026-10-30-display-deficit-fresh-trace-validation.py <trace.jsonl> <output.metrics.json>`。
