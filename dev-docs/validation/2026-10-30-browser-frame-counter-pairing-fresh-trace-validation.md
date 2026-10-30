# 播放计数配对后的新日志验收

2026-10-30 16:52日志确认当前WebView的WebKit计数缺乏有效值，主播放计数主要由native入口按约260ms合批消费，rAF补帧仍0。SR正常供给阶段完整3秒窗口约44–50fps，首次试探遭遇明显媒体/网络停顿。后续原生video完整15秒窗推进约59.93–59.99fps。下一步应验证RVFC缺席时的video纹理实际推进，并拆分源尺寸切换附近的资源准备与回调停顿。

## 范围与构建

- [日志](/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793350332368-1.jsonl)：16:52:12.368–16:58:20.768，368.400秒、2842行、10,994,066字节、schema3/dev。浏览器媒体375份，16:56:35.202–16:58:09.936、94.735秒；render telemetry94份。
- 736个源码输入的fingerprint `62536906cc72eb9433808b156885a591845155a8e92f57d5e024e8693f820a65` 与当前源码完全匹配。macOS/home/webrtc-direct/H264，显示预算全程60fps，可见性预算变化事件0。
- 源尺寸先为2560×1440，seq1302/292140ms变成1280×720，seq1533/301968ms恢复2560×1440。SR/g5为720p→1080p upscale、RCAS stops0.72；其余SR为1440p同尺寸passthrough、stops0.88。尺寸变化形成新的renderer阶段，跨阶段窗口各自隔离。
- [分析脚本](2026-10-30-browser-frame-counter-pairing-fresh-trace-validation.py)复用既有同代际/可见窗口分析；[metrics](2026-10-30-browser-frame-counter-pairing-fresh-trace-validation.metrics.json)保存365个唯一配对样本、累计时序差、源尺寸变化、冻结锚点、资源成本与门禁。完整3秒/15秒窗口相互重叠；候选/提交使用同代际即时增量。

## 配对计数结论

365个唯一配对样本的播放质量计数全部有效，WebKit total/dropped/frames有效样本均0。当前WebView未提供有效备用WebKit计数，当前日志支持继续沿播放质量主计数取证。所有配对样本的播放质量dropped为0，getStats的framesDropped增量7；两者分别覆盖播放质量统计与浏览器媒体丢弃统计。

诊断采集93.373秒内，4597个native全部接受；5559次rAF观察接受0。主计数350次更新、总推进5567帧，平均批次15.906帧、平均更新间隔266.429ms；348次在native入口、2次在rAF入口。两次rAF增量已被native资格抵扣，reason=`nativeAlreadyDelivered`，新增候选仍0。计数基准reset0，低频资源抵扣余额最大22帧，累计过期17帧/1次，末资源余额13帧。陈旧余额保持有界。

| SR代际 | 主计数更新次数 | 平均批次 | 平均更新间隔 | 更新入口native/rAF | native实际入口平均间隔 | rAF实际入口平均间隔 |
| --- | ---: | ---: | ---: | --- | ---: | ---: |
| g1 | 23 | 15.478 | 261.609ms | 23/0 | 20.267ms | 16.754ms |
| g3，含停顿 | 6 | 20.167 | 516.833ms | 6/0 | 52.000ms | 16.669ms |
| g5，720p→1080p | 23 | 15.609 | 259.391ms | 22/1 | 19.519ms | 16.671ms |
| g7 | 23 | 15.783 | 261.609ms | 23/0 | 20.748ms | 16.668ms |
| g9 | 12 | 15.500 | 256.917ms | 12/0 | 21.705ms | 16.678ms |

以上采用首末同代际即时诊断累计差，首样本之前的观察各自排除。全采集rAF时间戳到JS入口滞后平均1.512ms/max27ms，native平均0.022ms/max1ms。native参数`atMs`本身接近回调调用时间，这个差值衡量回调入口包装的时间差；浏览器内部排队、合成时序与GPU阻塞继续独立取证。

seq668的rAF样本263790ms：主计数65、delta0、余额5、实际入口263792ms，WebKit各字段null；随后多数样本重复同样的计数合批形态。seq779环内268357ms native计数delta15、距前次推进249ms；seq1262环内289874ms在长停顿后delta10、间隔984ms；seq1309环内292032ms推进59帧、间隔542ms。批量累计提供当前图像的更新事实，历史图像的独立绘制资格继续缺证。

原始metadata mediaTime固定0，sourceFpsEstimate全部缺证，reason=`mediaTimeDeltaTooSmall`。源帧供给以完整接收/解码窗口取证；配对字段的采样率3.909Hz、环上限8，诊断采样预算通过。

## 供给、steady supply与提交差

下表为完整3秒监督窗中位数。

| SR代际/尺寸 | 窗数 | 接收fps | 解码fps | video推进fps | 候选fps | 成功处理fps | 即时候选→处理 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| g1，1440p | 4 | 60.056 | 60.041 | 60.041 | 49.453 | 48.786 | 297→293/6.022s |
| g3，1440p，含停顿 | 1 | 36.903 | 36.544 | 41.211 | 18.903 | 18.903 | 58→58/3.012s |
| g5，720p→1080p | 4 | 59.867 | 60.399 | 60.414 | 50.475 | 50.475 | 310→310/6.026s |
| g7，1440p | 4 | 59.980 | 59.647 | 59.630 | 44.354 | 44.354 | 290→290/6.025s |
| g9，1440p | 1 | 59.780 | 59.780 | 60.113 | 46.557 | 46.557 | 139→139/3.013s |

g1即时额外损失4/297=1.347%，latestOverwrite累计4、pacingWait31；同窗候选→处理差中位0.667fps。其余四个即时区间交付100%，累计覆盖仍有1–2次的阶段边界差，窗口口径独立保存。五段即时总候选1094→处理1090，主供给缺口继续大于本地覆盖差。g1/g5/g7/g9的decode→输出逐窗差中位11.235/9.860/13.923/13.223fps；g3为17.641fps并伴随接收/解码下降。重复处理与提交失败均0。

g7四个完整窗输出依次53.222、44.839、43.869、43.578fps；同窗解码62.000、56.839、59.869、59.425fps。初期源切换/媒体推进批次与后续供给分别解释，完整窗中位44.354fps与整段即时48.133fps各有时钟口径。

原生video/g2三个完整15秒窗推进中位60.089fps，包含0.210秒冻结，冻结后纯15秒窗为0。video/g4仅0.653秒、g6仅3.504秒，稳态覆盖不足。video/g8三个完整15秒窗接收60.063、解码59.996、video推进59.990fps；最终video/g10十二个完整15秒窗（seq2545–2785）接收59.911、解码59.950、video推进59.933fps，推进范围59.881–60.037fps，同窗候选52.594fps。

video输出表示浏览器播放计数推进，SR输出表示本地成功处理；物理屏幕呈现仍缺证。共享帧源presentedFramesDelta为counterAdvance/nativeAdvance合并推断，原始nativeDelta单独保存。

## CPU成本与呈现长尾

同1440p的918次后续SR处理，上传CPU调用加权均值0.477ms、总同步处理0.507ms、GL提交及后处理0.022ms。720p→1080p/g5的322次上传0.360ms、总0.373ms；全部1240次上传0.447ms、总0.472ms。前一份同1440p日志为0.490/0.516ms，低调用成本维持。实际GPU转换/拷贝、零拷贝、逐帧执行和物理scanout继续缺证。

SR提交年龄29份中位10ms/P95202.4ms/max370ms；含停顿的g3四份中位165ms/max370ms，其余25份中位10ms/P9528.8ms/max32ms。seq1240/289374ms即时年龄316ms、seq1262/290378ms年龄370ms，与媒体零推进窗口相邻；缓存回调窗截至289175/290184ms，按各自时钟读取。年龄表示最近成功提交的新鲜度，真实submit-to-present与glass-to-glass继续unavailable。

trackingSource全部videoFrameCallback。按相邻同代际telemetry窗口统计：

| SR代际 | callback次数 | 合并推进总量 | gap | jump | dropped-like |
| --- | ---: | ---: | ---: | ---: | ---: |
| g1 | 297 | 360 | 0 | 61 | 61 |
| g3 | 58 | 130 | 3 | 39 | 39 |
| g5 | 310 | 362 | 1 | 52 | 53 |
| g7 | 290 | 360 | 1 | 65 | 65 |
| g9 | 139 | 180 | 0 | 41 | 41 |

g3有界native实际间隔最大967ms，rAF仍约16.669ms；停顿时两次rAF配对样本分别间隔15–17ms，主计数持续零推进。源切换附近的seq1309另有native542ms和rAF482ms间隔；renderer/g5 pending291571ms→committed292038ms，467ms准备/等待区间与浏览器回调停顿重叠。上下游恢复积压、资源准备和浏览器内部调度的分段因果继续缺证。

metadata预计显示时间到实际JS入口的有界样本中位：g1 −5ms、g3 −1ms、g5 −10.5ms、g7 −7ms、g9 −2.5ms。dropped-like为gap/jump并集；独立连续streak与单次sample最大presentedFramesDelta继续缺证。独立renderBackpressureChanged/renderCauseClassified事件0，原因采用telemetry与动作读取：大部分为frameDeliveryLimited；首段g1有一次renderStarvation，触发sustainedRendererPressure，g3受供给停顿影响，g7/g9短覆盖样本之后完整窗口回到frameDeliveryLimited。

## 网络、冻结与恢复

全媒体94.735秒接收183910包、丢失588包，丢失率0.31870%；接收5659帧/59.735fps、解码5651帧/59.651fps，framesDropped+7，平均视频码率16.981Mbps。RTT中位15ms/P9566ms/max80ms，jitter中位4ms/P9514ms/max23ms。加权jitter buffer10.850ms、decode4.242ms、processing14.022ms、assembly4.431ms。丢失集中度和缓冲长尾决定本次停顿归因，逐帧耗时分位数仍缺证。

| 冻结报告seq/时钟 | 时长 | 主要证据 |
| --- | ---: | --- |
| 1121 / 284379ms | 0.210s | video/g2；仍接收12帧、解码8帧，局部解码/显示供给长尾 |
| 1211 / 288168ms | 0.234s | SR/g3；接收16帧、解码6帧，后续解码22帧追赶 |
| 1251 / 289931ms | 0.815s | SR/g3；此前504ms接收/解码帧均0而收包继续；恢复时丢失+516 |
| 1277 / 291195ms | 0.962s | SR/g3；此前757ms解码0，收帧先恢复23，随后decode31/drop7 |
| 1533 / 301968ms | 0.295s | 720p恢复1440p附近，关键帧+1；native302ms间隔在g7提交前结束 |

五次共2.516秒。最大丢失突增与0.815/0.962秒冻结位于同一个inbound id/SSRC，计数重置证据0。seq1242/1247两份连续504ms接收/解码帧均0，收包60/371；seq1251收到651包、丢失516、decode16，窗口jitter buffer259.510ms。seq1264/1267/1274三份757ms decode0，分别仍收包126；末份接收帧23。seq1277 decode31、drop7，窗口jitter buffer510.434ms、processing438.374ms。持续收包时媒体停止推进、缓冲积压和随后解码恢复形成可复盘时间线。

全媒体PLI/FIR增量0/0、NACK+84、RTX包+776、关键帧decode+8，375份连接均connected。关键帧与解码推进有累计事实；请求抑制/合并、请求→响应/usable IDR配对、NACK有效/晚到/跳过/过期继续unavailable。pictureRecoveryTransition、pictureRecoveryBlockerObserved、videoIngressTermination、firstFrameLatencyObserved及H264逐包事件0；ReferenceChain、cleanAnchorCommitted、DisplayStable、建链成功率、NACK有效率、repairability连续性和综合恢复评分均unavailable。空样本汇总中的0分与实际恢复健康分别解释。

browserBehavior PASS；browserRuntimeStability为MISSING，failures0，唯一缺项sharedSamples0，虽有46份panelStatsObserved，面板与监督相同sampleId配对仍缺证。receiveFeedback PASS且关键样本0，内部闭环验收仍unavailable。366份Rust statsSnapshot来自空闲singleton，原生midsegment对该浏览器媒体采用inapplicable。trace budget notice0、dropped/droppedEssential0、pending最大3、inFlight最大4；getStats采集耗时中位1ms/P953ms/max7ms。

## 动作闭环与下一步

首次g1回退video/g2后seq872验证improved；g3唯一试探失败后回退g4，随后源尺寸变化形成g5。g5持续供给差回退g6，源恢复1440p形成g7；g7回退video/g8后seq1773验证improved，随后新阶段一次试探g9、失败回退g10、seq2300验证improved。两次试探属于源阶段变化前后的独立预算，7次动作资格齐备、5份效果记录；短g4/g6效果覆盖各自不足。seq2810正常退出，资源收敛，自动实机操作保持暂停。

下一步优先在现有video→WebGL路径验证缺少RVFC时rAF读取的纹理内容是否已经推进，并分段测量源尺寸切换的资源准备/首候选等待；依据独立图像证据选择补供给方案。网络侧单独跟进516包突发丢失、缓冲积压和恢复等待，保持接收权威、低延迟恢复与已有有限显示回退。实际SR60fps、GPU/零拷贝、像素、真实呈现、Windows/ROG和2h/8h继续归属S4。

复算：`python3 -B dev-docs/validation/2026-10-30-browser-frame-counter-pairing-fresh-trace-validation.py <trace.jsonl> <output.metrics.json>`。本轮交付日志分析、可复算指标与验收记录。
