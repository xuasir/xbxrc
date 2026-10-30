# 帧抵扣窗口与节拍修正后的实机日志验收

2026-10-30 16:09新日志确认候选进入SR后的持续额外吞帧已消除：两段同代际即时计数分别299→299、146→146，候选到成功处理均为100%。完整3秒窗口SR仍约49.14/48.60fps，同窗解码约60fps；原生video恢复前台后4个完整15秒窗的推进中位59.97fps。剩余SR供给缺口优先进入浏览器逐帧身份与RVFC时序取证。真实屏幕呈现、保留SR效果的60fps和平台长测继续归属S4。

## 范围与证据口径

- [日志](/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793347777498-1.jsonl)：16:09:37.499–16:12:47.021，189.522秒、2217行、7,010,435字节、schema3/dev。浏览器媒体统计358份、117.243秒；render telemetry 81份。
- macOS/home/webrtc-direct/H264，源与输出2560×1440、预算60fps；SR为等尺寸passthrough、单RCAS、stops0.88。736个源码输入的fingerprint `e84ab0262d8e4f38ec93b3b33f89780f72d0fc7ca3bef4b2a257834d4ed0cdbd` 与当前源码完全匹配，SR资源记录 `videoUploadMethod=texImage2D`。
- [复算脚本](2026-10-30-frame-credit-window-and-pacing-fresh-trace-validation.py)复用公共统计；[metrics](2026-10-30-frame-credit-window-and-pacing-fresh-trace-validation.metrics.json)保存同窗供给、前后台分段、抵扣过期、原始身份环、冻结锚点和门禁。旧分析的video/g4包含后台与冻结，最终稳态按本次可见性边界重新计算。
- 完整3秒/15秒监督窗口均按renderer代际、五项覆盖和可见区间筛选，窗口相互重叠。即时候选/成功处理使用同代际单调计数差；资源处理成本排除每个SR renderer首次处理。有界身份环的比例仅描述抽样证据。
- video输出表示浏览器播放计数推进；SR输出表示本地成功处理。共享帧源的 `presentedFramesDelta=max(counterAdvance,nativeAdvance)` 是合并推断，原始RVFC推进另存 `rawNativeDelta`。这些计数与物理屏幕呈现分别验收。

## 供给与提交

以下为完整3秒窗口中位数。

| 管线/阶段 | 完整窗数 | 接收fps | 解码fps | video推进fps | 候选fps | 当前路径输出fps |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| SR/g1，seq250–310 | 4 | 59.855 | 59.989 | 60.093 | 49.140 | 49.140 |
| video/g2，seq400–696 | 15 | 59.873 | 59.875 | 60.014 | 51.300 | 60.014 |
| SR/g3，seq789 | 1 | 59.875 | 59.875 | 59.906 | 48.603 | 48.603 |
| video/g4，隐藏前seq889–1517，含冻结 | 30 | 59.947 | 60.022 | 59.835 | 49.397 | 59.835 |
| video/g4，恢复前台seq1790–2130 | 16 | 59.875 | 59.882 | 59.898 | 51.077 | 59.898 |

SR/g1同代际即时区间seq193–317：6.064秒、299个候选→299次成功处理，49.307fps；rAF观察359次、接受0，native299次全部接受。SR/g3区间seq722–796：3.013秒、146→146，48.457fps；rAF观察181次、接受0，native146次全部接受。末次g3即时诊断与g4提交同在67440ms，通过诊断自带renderer代际保留g3终点。SR资源原始累计差分别304次/6.167秒、194次/3.979秒，实际处理49.295/48.756fps。

两段SR完整窗口的候选→提交delta均0；解码→SR供给缺口逐窗中位10.814/11.272fps。g1累计latestOverwrite0、pacingWait1；g3累计latestOverwrite1、pacingWait3。重复处理和提交失败均0，等待尝试和覆盖累计与上述即时区间各自解释。上一份g1即时300候选→280处理的持续额外损失已消除，剩余缺口集中在帧源交付。

video/g2三个完整15秒窗，接收59.963、解码59.959、video推进59.989fps。g4隐藏前18个完整15秒窗中位55.565fps，窗口包含1.414秒冻结；冻结恢复到隐藏仅约10.72秒，纯恢复后完整15秒窗为0。seq1705/137598ms恢复前台后约18.76秒、4个完整15秒窗（seq2066–2130）：接收中位59.926、解码59.942、video推进59.975fps，推进范围59.904–59.981fps；同窗候选50.759fps。此次原生video推进的60fps证据成立，实际屏幕呈现仍缺证。

## 抵扣窗口生效与播放计数批量更新

SR/g1资源抵扣余额0–13帧，g3为5–14帧；两段过期累计0。g4前台资源余额0–16帧；seq1274/89362ms记录首次过期，清理14帧。seq1526/100957ms隐藏、预算0；seq1529余额35帧，seq1544/104234ms过期累计69帧、次数2，即后台再清理55帧。其后后台余额持续0，seq1705恢复后回到个位至十几帧，过期累计仍69/2。窗口清理与暂停追加资格均有实机资源证据。

当前播放计数来源始终 `playbackQuality`。两段SR有界身份环20个rAF样本的counterDelta均0；5个正增量样本全部在native入口读取，批量15–17帧、距上次推进249–266ms。关键锚点：

| telemetry seq / 证据时钟 | 入口 | 播放计数delta | 抵扣帧数 | 剩余advance | 余额变化 | 距上次计数推进 |
| --- | --- | ---: | ---: | ---: | --- | ---: |
| 317 / 44863ms | rAF | 0 | 0 | 0 | 14→14 | 232ms |
| 317 / 44880ms | native | 16 | 14 | 2 | 14→0 | 249ms |
| 766 / 66180ms | native | 17 | 16 | 1 | 16→0 | 250ms |
| 796 / 67197ms | native | 16 | 15 | 1 | 15→0 | 250ms |

这批实机计数约250ms合批更新，native入口常先消费这一增量；rAF难以获得及时的新身份。累计轮询接受仍0，清理陈旧抵扣余额与逐帧补供给分别具备条件。此前受控回放的60fps使用逐帧推进播放计数，设备的合批节拍需要单独验收。批量计数一次只能读取当前video图像，历史帧已缺少独立绘制机会；将delta直接重复绘制多次会放大处理计数。

原始RVFC metadata `mediaTime` 固定0，全部sourceFpsEstimate缺证（`mediaTimeDeltaTooSmall`）。原始nativeDelta抽样中位1、最大2；源约60fps由完整接收/解码计数取证。当前代码优先有效 `getVideoPlaybackQuality()`，`webkitDecodedFrameCount` 的同时取值和更新及时性缺证，下一步应低成本配对观察两种计数与RTP/native身份，再决定补帧来源。

## 上传成本、回调与提交长尾

两段SR共498次后续成功处理：上传CPU调用加权均值0.490ms，总同步处理0.516ms，GL提交及后处理0.024ms；前一份对应0.436/0.465/0.020ms，低调用成本维持。首次上传19/22ms，总处理21/22ms。实际GPU转换/拷贝命中、CPU像素拷贝次数与零拷贝状态继续缺证。

SR实时提交年龄11份：中位12ms、P9570ms、最大116ms；资源快照3份年龄中位13ms。seq289的即时提交年龄116ms取自44347ms，其缓存回调窗截至44050ms；随后seq317记录最大回调间隔117ms。缓存窗与即时年龄采用各自时钟，116ms长尾继续取证帧源/回调供给。这个年龄表示最近成功提交的新鲜度，真实submit-to-present、GPU执行及glass-to-glass仍unavailable。

trackingSource全部为videoFrameCallback。按相邻telemetry同代际、完整落在可见阶段内结算：

| 阶段 | callback次数 | gap计数 | 合并推进总量 | jump计数 | dropped-like | 最大回调间隔 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| SR/g1 | 299 | 1 | 368 | 61 | 62 | 117ms |
| video/g2 | 877 | 1 | 1023 | 144 | 144 | 100ms |
| SR/g3 | 146 | 0 | 181 | 34 | 34 | 50ms |
| video/g4隐藏前，含冻结 | 1561 | 5 | 1860 | 282 | 283 | 1401ms |
| video/g4恢复前台 | 928 | 1 | 1083 | 150 | 150 | 110ms |

合并推进与callback次数差说明一次回调可跨多帧推进；dropped-like为gap/jump并集。独立 `droppedLikeStreak` 与单次sample最大presentedFramesDelta继续缺证。SR/g1的12个native抽样均晚于metadata预计显示时间，晚到中位8ms、最大15ms；g3的8个样本晚到中位6ms、最大18ms。这是浏览器metadata时序提示，实际scanout和高频全过程需要额外证据。

renderBackpressureChanged、renderCauseClassified、renderFrameDropped独立事件均0；原因由telemetry及动作证据读取。g3初始renderStarvation对应短覆盖，完整3秒窗归因为frameDeliveryLimited。稳定期本地同步成本低、接收/解码约60fps，SR帧交付不足持续触发已有有限回退。

## 网络冻结与恢复健康

全媒体117.243秒收包227828、丢失112，丢失率0.04914%，平均视频码率16.983Mbps；接收/解码均6943帧、59.219fps，framesDropped增量0。RTT中位15ms/P9541ms/max90ms，jitter中位5ms/P959ms/max34ms。按帧加权jitter buffer4.305ms、decode4.497ms、processing8.829ms、assembly3.277ms；这些是累计/窗口均值，逐帧耗时长尾缺证。

仅保留增量区间完全落在可见政策内的统计：前台79.465秒、接收/解码58.906/58.919fps、丢失0.04456%，包含一次1.414秒freeze；后台36.516秒、接收/解码59.919fps，freeze0。跨政策边界的增量区间单独排除。连接始终connected，候选对有值时身份保持；两段SR的freeze增量均0。

冻结发生在原生video/g4前台：

1. seq1255/88470ms仍接收11帧、解码12帧。
2. seq1262/1267/1271/1279四份连续统计共1009ms，包、字节、接收帧和解码帧增量均0；连接保持connected。
3. seq1286/89730ms收包2126、2,301,255字节、丢失14；接收帧/解码帧仍0，PLI+1、NACK+5、jitter34ms。
4. seq1289/89983ms接收13、解码12、关键帧+1，freeze+1/duration1.414秒；seq1292/90237ms解码16帧、节拍恢复。

收包观测停顿→包恢复但媒体供给继续等待→关键帧解码恢复的链条有证据；发送端、路径、浏览器内部停顿和内部恢复策略的细分因果继续缺证。全程PLI+1/FIR0、NACK+93、RTX包+962、关键帧decode+7。请求抑制/合并、单次PLI响应与解码配对、NACK有效/晚到/跳过/过期分别unavailable。

按恢复主线读取的pictureRecoveryTransition、pictureRecoveryBlockerObserved、videoIngressTermination、firstFrameLatencyObserved及H264逐包事件均0。ReferenceChain、cleanAnchorCommitted、DisplayStable、建链成功率、NACK有效率、repairability连续性与恢复综合评分均unavailable。关键帧decode计数具备证据，内部闭环继续独立验收。

browserBehavior与browserRuntimeStability PASS，sharedSamples24、三次软动作证据完整、closedClients1。receiveFeedback返回PASS且关键样本0，恢复闭环验收状态为unavailable。189份Rust statsSnapshot来自空闲singleton，原生midsegment门禁在浏览器直连媒体中采用inapplicable。trace budget notice0、dropped/droppedEssential0、pending最大3、inFlight最大4，观测丢失证据为0。

## 动作闭环与下一步

- seq311–316：完整3000ms窗口、持续3052ms的frameDeliveryDeficit回退video/g2；seq401新代际完整3秒验证improved。
- seq697–703：健康持续15061ms后唯一增强试探，提交SR/g3；seq788–795完整3秒输出48.603fps低于98%源预算，试探失败并回退video/g4，增强效果neutral；seq888验证回退improved。用户SR意图保留，video回退期间增强暂停。
- seq1526隐藏与seq1705恢复分别落实预算0/60；seq2141正常退出，资源收敛。自动实机操作保持暂停。

下一步优先补齐同一帧源内播放质量计数/WebKit计数的配对更新节拍，以及RVFC进入、rAF机会与RTP身份的有界时序；离线回放增加约250ms合批计数且45–50Hz RVFC的输入合同，再以同机前台60fps源检验真实新增帧和SR处理覆盖。保持当前媒体与renderer边界、同帧去重、latest-only及用户限帧合同。网络冻结单独追踪收包停止到媒体恢复的延迟；GPU/零拷贝、像素、真实呈现、Windows/ROG和2h/8h继续按S4验收。

复算：`python3 -B dev-docs/validation/2026-10-30-frame-credit-window-and-pacing-fresh-trace-validation.py <trace.jsonl> <output.metrics.json>`。本轮交付离线日志分析、可复算指标与验收记录。
