# 视频纹理导入优化后新日志验收

2026-10-30 15:26新构建。视频纹理上传调用加权均值从14.665ms降至0.436ms，下降97.03%；同步处理总耗时从14.821ms降至0.465ms。前台SR完整3秒窗口仍仅提交47.41/52.26fps，下一阶段重点进入共享帧源跨计数抵扣和本地限帧调度。最终video恢复后27个完整15秒窗的输出推进中位59.91fps；保留SR效果的60fps目标继续归属S4。

## 范围与采样边界

- [日志](/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793345174456-1.jsonl)：15:26:14.457–15:33:44.171，449.714秒、2991行、schema3/dev。浏览器媒体统计459份，覆盖15:26:56.570–15:30:08.231、191.662秒；render telemetry 89份。
- macOS/home/webrtc-direct、H264，2560×1440源与输出、60fps预算；SR使用等尺寸passthrough、单RCAS、stops0.88。构建源码fingerprint `488c609246f7efe401849577462ce64af9f6128e1b5c2ec159e5c7395a050d1b` 与736个当前源码输入完全匹配。SR/g1和g3资源均记录 `videoUploadMethod=texImage2D`。
- seq141–573为隐藏段，targetFps=0；seq184–573期间媒体仍约60fps推进，renderer保持pending。seq573恢复可见，seq576首个SR提交；随后seq590–593再次隐藏551ms。首个SR全资源跨度包含这次隐藏，前台结论优先使用最后一次恢复后的完整窗口及同代际即时增量。
- 首个SR从pending到committed约106.9秒，rendererFailed为0；期内后台预算为0，首个GPU提交发生在可见恢复后。首帧浏览器video里程碑与SR首个提交分别记录。连接→浏览器首次推进773ms，首次getStats包/解码里程碑998ms，后者受异步采样边界影响。
- 复算数据、时间窗、源码指纹、网络冻结锚点保存在[metrics](2026-10-30-video-texture-import-fresh-trace-validation.metrics.json)，入口为[分析脚本](2026-10-30-video-texture-import-fresh-trace-validation.py)。各窗口按同renderer代际和完整覆盖取证；共享video的身份环按renderer提交时间过滤前代际残留。

## 上传优化的实机收益

以下为同代际原始资源累计差，分别排除两次renderer首次提交：基线496次、新日志551次。两会话源尺寸、SR模式与预算一致，画面、码率和前后台历史有差异，帧率收益采用各会话同窗事实解释。

| 每次成功处理的CPU调用计时 | 14:39基线ms | 本次ms | 降幅 |
| --- | ---: | ---: | ---: |
| 视频纹理上传 | 14.665 | 0.436 | 97.03% |
| GL提交及其后处理 | 0.149 | 0.020 | 86.62% |
| 同步处理总耗时 | 14.821 | 0.465 | 96.87% |

SR/g1的348次后续处理上传均值0.460ms，SR/g3的203次后续处理0.394ms。首次上传仍为24/22ms，对应处理总耗时26/22ms；冷启动成本和稳态均值分开解释。新日志getStats采集全媒体中位1ms/P952ms，旧SR期间为22ms/P9524ms，主线程异步等待压力随上传调用成本下降。

应用调用入口与耗时下降均有设备证据；实际WebKit GPU快速拷贝命中、CPU内存拷贝次数、GPU转换/执行时间及零拷贝状态继续unavailable。`texImage2D`符合公开WebKit快速尝试条件，GPU内部存储和转换仍需后端证据。

## 前台供给与提交缺口

以下为完整3秒监督窗口中位数，窗口相互重叠。首个SR四窗、首次video十五窗、SR试探一窗、最终video五十六窗。

| 管线/代际 | 接收fps | 解码fps | video推进fps | 候选fps | 当前路径输出fps |
| --- | ---: | ---: | ---: | ---: | ---: |
| SR/g1，seq608–735 | 59.98 | 59.82 | 59.83 | 51.02 | 47.41 |
| video/g2，seq761–1138 | 60.07 | 60.19 | 60.18 | 44.44 | 60.18 |
| SR/g3，seq1161–1235 | 59.86 | 59.86 | 59.86 | 52.26 | 52.26 |
| video/g4，seq1262–2528 | 59.90 | 59.87 | 59.86 | 52.18 | 59.86 |

SR/g1同代际即时计数差：6.024秒内300个候选、280次成功处理，候选49.80fps→提交46.48fps，额外损失20/300=6.67%；四个完整窗口的输出/可用源预算约79.3%。pacingWait累计增长、latestOverwrite累计达到22，duplicateDraw与submitFailure均0，供给进入renderer后存在latest覆盖丢失。`pacingWaitCount`表示等待尝试次数，独立丢帧数量继续使用候选与成功处理差。

SR/g3即时计数差：3.012秒内157个候选→157次处理，52.12fps；完整窗口输出/可用源预算约87.3%。试探累计pacingWait58、latestOverwrite2，完整即时区间候选交付完整。两段SR的网络解码均约60fps、freeze增量0；持续欠帧同时包含浏览器候选供给不足，以及首段本地调度覆盖损失。

最终video完整15秒窗口共43份，其中早期窗口包含1.114秒冻结，输出最低55.31fps。冻结后从下一份恢复解码统计191137ms重新建立完整15秒覆盖，得到27窗：接收中位59.924fps、解码59.910fps、video/输出59.910fps，输出范围59.837–60.076fps；候选中位50.18fps。video输出统计使用浏览器推进，SR输出使用成功处理计数，二者按各自定义读取。

## 跨计数抵扣余额：已复现的下一处瓶颈

本次诊断累计4477次native回调全部接受；5317次rAF观察的候选接受数为0。原始metadata的mediaTime固定0，sourceFpsEstimate全部缺证，reason=`mediaTimeDeltaTooSmall`；source供给以完整接收/解码计数取证。

共享帧源的native `presentedFrames` 与 `playbackQuality` 计数长期相差约6100–6200帧，`uncountedNativeFrames`一直保留相近规模的抵扣余额。关键seq608环内143701ms样本：native=6164、playback=33，播放计数增量14全部抵扣、counterAdvance=0，余额从6142变成6129；144701ms样本native=6225、playback=48。随后seq628/648等继续出现数千帧余额。metrics保存counterDelta、creditedNativeFrames及counterAdvance锚点。前后台计数分叉与首次约102秒媒体后台推进相邻，余额形成的高频全过程继续缺证。

`video-frame-source.ts`将播放计数增量优先抵扣 `uncountedNativeFrames`；native交付又持续补充余额。计数基准分叉后，旧资格会跨越后续窗口维持，rAF补帧机会受到阻断。原生RVFC形成新候选仍被完整接受，上一轮native额度修复的效果维持。

[离线复现](2026-10-30-video-texture-import-fresh-trace-replay.cjs)直接加载当前生产帧源及SR，保持600个源帧/10秒、逐帧播放计数、60Hz刷新，分别提供45/50Hz native回调；用后台native推进6120而播放计数冻结构造相同的余额状态：

| 受控输入 | native候选 | 轮询补帧 | SR提交/10秒 |
| --- | ---: | ---: | ---: |
| 计数对齐，RVFC45Hz | 450 | 150 | 600 |
| native领先6120，RVFC45Hz | 450 | 0 | 450 |
| 计数对齐，RVFC50Hz | 500 | 100 | 600 |
| native领先6120，RVFC50Hz | 500 | 0 | 500 |

四组断言通过，领先组终点余额6119，均无提交失败、pacingWait或latestOverwrite。该复现独立证明陈旧抵扣可压低补帧供给；实机播放计数的更新节拍、计数对应同帧的精度与剩余renderer限帧损失需要继续验证。简单清空余额后的设备60fps效果仍待新构建证据，修正必须保留正确晚到批次的同帧去重。

## 回退、冻结与长尾

- seq728/731/734：完整3000ms窗口、持续3011ms压力触发 `sustainedRendererPressure`，提交video/g2；seq825以新代际完整3秒验证improved。
- seq1133/1135/1139：健康持续15060ms后唯一一次增强试探，提交SR/g3；seq1227完整3秒输出52.26fps低于98%源预算，seq1228/1229触发 `enhancementTrialFailed`，提交video/g4。增强效果记录neutral；seq1323再次验证回退improved。用户SR意图保留，回退区间实际增强暂停。
- 第一处freeze，seq966/161360ms：0.205秒，video/g2。之前RTT达到107ms、jitter达到17ms，jitter buffer窗口均值最高59.09ms；seq959接收12帧、解码2帧、buf53.77ms，seq966接收19帧、解码30帧后恢复。接收缓冲及解码供给长尾相邻，shader路径已回退。
- 第二处freeze，seq1604/190885ms：1.114秒，video/g4。seq1583/1588/1595/1601四份连续统计共1010ms收包/字节/接收帧/解码帧增量均0；连接仍connected，部分候选对字段缺证。随后接收79帧、解码15帧、framesDropped+63、丢包+15、关键帧+1。该次接收观测停顿与恢复后积压丢弃有完整时间线；远端、链路或浏览器内部停顿的细分因果继续缺证。
- SR当前同代际telemetry提交年龄11份，中位10ms/P9525.5ms/max32ms；表示最近成功提交的新鲜度。原生video/g4最大回调间隔1100ms对应上述冻结。GPU执行、物理scanout、真实submit-to-present、glass-to-glass及逐帧CPU P95/P99继续unavailable。
- trackingSource均为videoFrameCallback。SR/g1与g3最大回调间隔96/51ms，采样callbackGap计数1/0、presentedFramesJump88/35；video/g2最大200ms，gap6、jump263；video/g4最大1100ms，gap3、jump389。全首个恢复样本包含后台累计，按前台分段解释。dropped-like为gap/jump并集；streak和单次最大presentedFramesDelta字段继续缺证，采样jump表达多帧推进。

## 网络与恢复证据

全媒体191.662秒接收447688包、正丢失228包，丢失率0.05090%，平均视频码率20.46Mbps；接收59.94fps、解码59.61fps。RTT中位14ms/P9564ms/max107ms，jitter中位5ms/P9512ms/max17ms。按帧加权jitter buffer5.97ms、decode4.79ms、processing10.69ms、assembly4.18ms；统计窗均值分位数与逐帧长尾分别解释。

仅取完全落在可见政策区间内的统计增量，前台覆盖88.496秒、接收139455包、丢失128包，丢失率0.09170%；接收59.96fps、解码59.22fps，包含两次freeze/1.319秒及63个解码丢弃。前台RTT中位14ms/P9588ms/max107ms；后台样本稀释了全程网络长尾，前台体验优先读取分段统计。候选对有值时身份保持同一个，459份连接状态均connected。

全媒体PLI/FIR增量0/0，起始统计已含PLI累计2；NACK+132、RTX包+612、关键帧decode+6。pictureRecoveryTransition、pictureRecoveryBlockerObserved、videoIngressTermination、firstFrameLatencyObserved及H264逐包事件均0。浏览器内部请求抑制/合并、响应→decode配对、ReferenceChain、cleanAnchorCommitted、DisplayStable、NACK及时性与有效率、建链成功率、repairability连续性及恢复综合评分继续unavailable。

browserWebRtcBehaviorGate、browserRuntimeStability均PASS，sharedSamples5、三次软动作证据完整、退出资源收敛。receiveFeedbackGate返回PASS且关键恢复样本为0，内部恢复闭环验收仍unavailable。447份Rust statsSnapshot来自浏览器媒体之外的空闲singleton，原生midsegment gate在该会话采用inapplicable，浏览器媒体统计为性能事实权威。

trace无budget notice，dropped/droppedEssential均0，pending最大3、inFlight最大4。seq2546菜单退出、seq2554 runtimeStopCompleted、seq2561远端closed=true。退出后的singleton日志与媒体性能窗口分别取证。

## 下一步

1. 优先修正共享帧源native抵扣资格的有效窗口与前后台计数分叉，覆盖领先/落后计数、批量更新、native/rAF先后、重复身份、暂停恢复；保持同帧去重与latest-only，使用上述生产复现和新前台日志验收。
2. 校准现有FramePacer的累计deadline与回调相位，补有界等待→覆盖→提交证据；以约60fps解码、候选→处理98%和完整3s/15s覆盖验证，保持当前renderer与媒体边界。
3. 同机1440p/60fps持续前台验证保留SR效果的输出，并观察网络冻结；零拷贝、GPU计时、像素与实际呈现、Windows/ROG及2h/8h验收继续归属S4。自动实机操作保持暂停。

复算：`python3 -B dev-docs/validation/2026-10-30-video-texture-import-fresh-trace-validation.py <trace.jsonl> <output.metrics.json>`。本轮交付分析、四组生产模块离线复现和验收文档。
