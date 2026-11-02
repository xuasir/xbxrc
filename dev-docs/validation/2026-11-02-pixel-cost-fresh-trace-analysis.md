# 新像素成本日志：32ms首读、网络停顿与SR供给

2026-11-02 10:59新日志确认：首次像素读取32ms，初始化记录0ms、drawImage18ms、getImageData14ms。探测在原生video阶段触发8ms停止阈值，有效配对0。两段SR完整3秒窗口提交中位数48.09/58.44fps；第二段接近60fps，仍需更长完整窗口及真实呈现证据。主要可见帧差集中在解码→候选边界，首段同时存在网络突发丢失与缓冲长尾。

## 范围与构建

[runtime-trace-1793588378449-1.jsonl](/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793588378449-1.jsonl)范围10:59:38.449–11:03:13.191，共214.742秒、3124行。媒体统计窗口11:00:50.641–11:02:57.957，共127.316秒、506个样本。日志SHA256为 `4e7e7c73d5295d74447894db0d18c70b47998a92f34b6715cf3037f2f02b5ef0`。

源码737个输入与日志匹配，fingerprint为 `bd14d98f856a7432f0767a1d01b3ae5efe70544e951f5af466a2376a71258120`，commit `c4c2659a`，macOS/debug/home。新分项在127个telemetry快照中可见。本次路径按video g1→SR g2→video g3→SR g4→video g5推进，全程目标60fps、源2560×1440。SR为等尺寸passthrough、锐化0；放大效果路径继续单独验收。

## 像素分项已经定位

seq258在单调72,099ms启动惰性诊断，readCount0。首次读取发生在72,103–72,135ms，来源videoFrameCallback，localSequence16；seq278（`tsMs=1793588451864`）首次发布这份成本：

| 首读分项 | 记录耗时 |
| --- | ---: |
| canvas/context初始化 | 0ms |
| drawImage(video→16×9 Canvas2D) | 18ms |
| getImageData及取得data | 14ms |
| 完整首读 | 32ms |

初始化0ms采用当前毫秒时钟精度解释。coldReads=1/32ms，warmReads=0；firstRead和lastRead对应同一次尝试。outcome为readbackBudgetExceeded，readCount1、baselineCount/pairCount/changedPairCount均为0；后续快照保持相同状态，canvas和pending pair均为0。

首次SR提交在106,726ms，距离慢读结束34.591秒。后续两段SR的像素探测已停止。新证据把优化目标集中到图像绘制和同步回读的32ms成本；有效drawable推进配对继续缺证。18/14ms分别是两项API的同步调用时间，浏览器图像转换、拷贝、驱动等待及GPU执行的内部比例继续待验。

## 网络出现一次明显停顿

127.316秒新增接收238216包、正丢失381包，丢失率0.159683%；接收7632帧、解码7570帧，平均59.945/59.458fps。视频平均16.334Mbps，506个样本连接状态均为connected，单一inbound/SSRC。解码丢帧增量62，冻结1次/1.110秒，pause增量0。

| 网络/解码采样窗均值 | P50 | P95 | 最大值 |
| --- | ---: | ---: | ---: |
| RTT | 16ms | 28ms | 76ms |
| RTP jitter | 4ms | 9ms | 15ms |
| jitter buffer延迟 | 2.900ms | 11.243ms | 43.134ms |
| 解码耗时 | 4.439ms | 5.003ms | 26.082ms |
| processing延迟 | 7.440ms | 15.886ms | 51.248ms |

分位数描述采样窗均值，各字段沿其浏览器统计定义结算。按帧数加权的buffer/解码/processing均值为4.524/4.459/8.858ms。

冻结在g1原生video阶段。原始时间线：

- seq394，单调78,691ms：252ms窗口仅收6包、接收/解码2帧。
- seq397/404/410，78,942–79,447ms：连续251/253/252ms窗口收包、接收帧和解码帧增量均为0，合计756ms。
- seq413（`tsMs=1793588458463`），79,699ms：收包恢复308包、丢失+14，接收73帧、解码1帧、丢帧60，freeze+1/1.11s，NACK+1，PLI和关键帧解码增量0。
- seq417，79,950ms：接收16帧、解码25帧、丢帧2；seq424恢复接收/解码15帧。该恢复区间保持PLI和新关键帧解码增量0。

收包与媒体计数停顿发生在慢读结束约6.56秒以后，早于首次SR开启。它支持独立的接收侧进度停顿证据；远端停发、链路或浏览器接收内部停顿的具体责任继续缺少独立事实。

另外seq273突增173包丢失时仍接收16/解码15帧。首次SR内seq1100突增73包丢失、接收12/解码7帧，buffer窗口均值43.134ms；随后seq1105为40.036ms。首段SR回调最大间隔136ms，缓冲长尾与回调稀疏在同一阶段出现，性能归因同时保留网络扰动和浏览器交付事实。

## 帧供给与SR处理

窗口按renderer代际、前台状态及接收/解码/video推进/候选/处理五类完整覆盖筛选；窗口允许重叠。下表video输出采用浏览器推进计数，SR输出采用成功本地处理。

| 阶段 | 完整窗口 | 解码fps中位数 | 候选fps中位数 | 输出fps中位数 | 同窗解码→输出差中位数 |
| --- | --- | ---: | ---: | ---: | ---: |
| g1 video | 20个×15s | 59.894 | 50.386 | 59.902 | −0.051fps |
| g2 SR | 4个×3s | 60.174 | 48.421 | 48.087 | 12.087fps |
| g3 video | 3个×15s | 59.929 | 50.035 | 59.996 | −0.067fps |
| g4 SR试探 | 1个×3s | 59.778 | 58.444 | 58.444 | 1.333fps |
| g5 video | 48个×15s | 59.948 | 51.756 | 59.927 | 0fps |

窗口中的微小负差来自各统计窗口的推进与时钟边界；它按原始值保留。两段SR的完整15秒窗口均为0，g2/g4实际阶段6.534/3.983秒。58.44fps覆盖短时试探，同条件长期足帧继续待验。

实时同代际诊断：

| SR区间 | seq | 时长 | rAF观察率 | RVFC/候选率 | rAF补帧 | 候选→提交 |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| g2 | 1028→1165 | 6.025s | 60.083Hz | 49.129Hz | 0 | 296→294，99.32% |
| g4 | 1588→1663 | 3.014s | 60.053Hz | 58.394Hz | 0 | 176→176，100% |

两段rAF均值间隔16.666/16.669ms，RVFC为20.382/17.136ms。首段latest覆盖累计2、pacing等待11；试探累计1/1，包含交接事实，实时试探区间完整交付。重复绘制和提交失败均为0。

诊断全会话累计7578次rAF观察、接受0；6451次RVFC均接受。主播放计数更新482次，其中476次在RVFC、6次在rAF；500组配对的播放质量计数有效500、WebKit有效0。SR两段主计数更新均值265.913/259.750ms。持续帧差的主要可见规模仍由解码→候选差解释；同窗候选→提交损失分别为2/0。

`trackingSource`全部videoFrameCallback，127个sourceFpsEstimate均因mediaTimeDeltaTooSmall缺证，源帧率以RTC增量判断。g2/g4 telemetry的callback gap为4/0，多帧跳跃75/13，dropped-like77/13，最大回调间隔136/82ms；推进量分别422/241、回调338/227。这些数值用于回调/计数合批诊断。sourceFrameIntervalMs、maxPresentedFramesDeltaSinceLastSample、droppedLikeStreak和持续renderBackpressure缺少字段；独立renderFrameDropped/renderBackpressureChanged/renderCauseClassified事件均为0。renderCause投影为frameDeliveryLimited121、renderStarvation3、decodeBackpressure2、unknown1。

## 同步成本、切换与上屏边界

542次冷首提交后的成功处理，CPU总同步调用均值0.428ms，上传0.393ms、GL命令提交0.031ms；g2/g4稳态上传0.358/0.442ms。video纹理继续使用texImage2D。最新成功提交年龄11个样本P50/P95/P99/max为6/15/15/15ms。

| SR创建 | 请求→提交 | setup | setup→候选 | 候选→draw | 首draw | 首上传 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| g2 | 37ms | 16ms | 1ms | 0ms | 18ms | 16ms |
| g4 | 34ms | 8ms | 1ms | 1ms | 23ms | 23ms |

SR→video分别1/2ms。源尺寸全程1440p，这些切换覆盖同尺寸重建；源尺寸变化继续待验。像素回读32ms、冷上传16/23ms和稳态上传0.393ms分开结算。submit年龄表达采样时最新成功提交的新鲜度，GPU执行、submit→present、合成排队、glass-to-glass和物理屏幕长尾继续缺少直接证据。

seq1159持续帧差回退→seq1226 improved；seq1561执行一次健康增强试探→seq1656试探失败回退，试探效果seq1660 neutral；seq1754恢复video评估improved。最终video稳定推进59.927fps。seq3082正常关闭，媒体、renderer、回调、timer与像素资源释放归零。

## 恢复、门禁与结论边界

首个RTC样本已有PLI0/FIR0/NACK1、关键帧解码3和RTX包69；媒体窗口新增PLI0/FIR0/NACK80、关键帧解码4、RTX包672。NACK与RTX粒度各自保留。逐请求抑制/合并、usable response、NACK及时/迟到/跳过/过期、建链成功率、repairability连续性和综合恢复评分均为unavailable。

按固定顺序，pictureRecoveryTransition、pictureRecoveryBlockerObserved、videoIngressTermination、firstFrameLatencyObserved和H264内部检查均为0；ReferenceChain、cleanAnchorCommitted和DisplayStable缺少浏览器内部事实。默认receive报告PASS且决策样本0；严格必需媒体/显示闭环退出2，缺少两项链条证据。native midsegment的media/steady supply为SKIPPED、提交/呈现年龄n/a；实际供给采用本报告browser完整同窗结果。

browser行为与runtime稳定性PASS，12组共享样本和174组frame-chain样本通过。schema3/dev全部3124行，essential/key/debug为73/1984/1067；traceBudgetNotice0，前端dropped/droppedEssential0。内部恢复与真实呈现边界持续保留，SR60fps以实际足帧验证收口。

相较[10月30日17:43](2026-11-02-network-render-fresh-trace-analysis.md)，本次丢失率0.039889%→0.159683%，新增1次1.11s freeze；SR两段完整3s输出51.50/54.25→48.09/58.44fps。两份独立会话的运行顺序、网络和短时阶段分别保留，具体改造收益继续依赖同条件配对。

## 验证与下一步

[复算脚本](2026-11-02-pixel-cost-fresh-trace-analysis.py)复用既有底层同窗算法，兼容初始video尚无源尺寸/提交计数的阶段。3124个唯一seq、184个完整窗口、同代际增量、像素cold/warm总值、生命周期顺序及日志SHA核对均通过。[metrics](2026-11-02-pixel-cost-fresh-trace-analysis.metrics.json)保存具体窗口、分项、门禁、动作和指纹。

下一步优先沿当前资源所有权评估低频异步缩略取证，针对drawImage和回读的转换/同步成本保留独立计时与8ms预算；有效配对后再选择共享帧源补供给策略。继续记录网络突发窗口，使用持续前台运动场景和更长有效SR阶段验证60fps与实际呈现。当前原生video提供接近60fps的可靠基线，S4保持In Progress，实机采集由用户手动完成。

```sh
python3 -B dev-docs/validation/2026-11-02-pixel-cost-fresh-trace-analysis.py
```
