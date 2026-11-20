# 最新 Mac iOS Home Trace 健康与低帧率分析

日期：2026-11-20。运行目标由用户明确确认为“My Mac (Designed for iPad)”。日志记录appVersion=0.1.0、buildNumber=1、debug；兼容运行下的`deviceModel=iPad8,6`结合用户确认的Mac目标解释，Mac型号与GPU能力仍缺直接字段。

**第二段串流的接收与解码接近60FPS，实际提交/呈现约18–25FPS。GPU完成到主线程写出呈现事件的中位处理延迟约52ms，单帧在途槽在该处理路径中释放；显示入口累计合并丢弃约69%的解码帧。高频display health回报重复发布playing状态是首要代码疑因。超分成功配置路径缺运行证据，锐化实际强度与画质收益需要补充运行配置和同场景A/B。**

## 输入、门禁与日志质量

- 原始文件：`/Users/guo.xu/Downloads/runtime-trace-ios-1795166210528-1.jsonl`；项目副本：`runtime-logs/ios/runtime-trace-ios-1795166210528-1.jsonl`。
- 19554765字节（18.65MiB），单个进程session，36195行，seq=1–36195连续；时间17:16:50.527–17:23:51.844，共421.317秒，全部dev profile。
- Schema、序列、隐私及文件预算通过；完整JSON行零损坏、零缺号，零`traceBudgetNotice`。文件低于32MiB预算，本次提交目录仅此一份原始Trace。
- 基础全流程gate为FAIL：seq138的图片候选拥有start而缺outcome；启动、认证与云库必需锚点齐全。该图片配对缺口属于应用图片流程。
- 串流core总gate为FAIL：第一段缺完整peer epoch及playing；第二段独立PASS，连接、握手、首帧、稳态和退出链齐全。core PASS证明流程链，其性能结果由下述计数与时间分析给出。
- 文件头storageFailures/storageDroppedRows/durabilityUncertainRows为0，文件中持续写入至导出请求，未出现存储恢复或丢弃摘要。writer队列等待全文件P95约0.64ms、P99约1.26ms。

完整结果见[基础分析](base-analysis.json)、[串流core](core-analysis.json)、[输入哈希](input.json)及[验算摘要](validation.json)。

## 最小时间线

| 段落 | 时间与seq | 事实 |
| --- | --- | --- |
| 第一段Home | 17:19:42.774，seq500 | 启动generation=1；随后出现前后台切换。 |
| 第一段重建 | 17:20:13.057，seq551–552 | directPathExhausted触发fallback重试及runtime rebuild。 |
| 第一段媒体 | 17:20:17.761–17.776，seq645/650 | 收到首帧并首次呈现；握手、控制及输入bootstrap已完成。 |
| 第一段终止 | 17:20:31.261，seq3399–3401 | 15秒initialConnectionTimedOut后以ice.failed终止；peerConnected与playing锚点缺失。 |
| 第二段Home | 17:20:34.287，seq3417 | 启动generation=2。 |
| 第二段连接 | 17:20:59.676，seq3468 | peerConnected，peerEpoch=3。 |
| 第二段首帧/握手 | 17:21:00.029–00.163，seq3494/3499/3514/3517/3530 | 首帧、message ACK、控制与输入bootstrap、首次呈现完成。 |
| 第二段稳态 | 17:21:01.040，seq3731 | steadyMediaObserved；后续持续约146秒媒体供给。 |
| 第二段正常退出 | 17:23:26.900–27.606，seq36165/36181/36191 | 主机关机被接受，user.stop，peer与远端session关闭、access释放。 |

原始关联信息和代表帧见[evidence-anchors.json](evidence-anchors.json)。第一段中的媒体事实与连接状态确认存在脱节：`hasEverConnected`仍为false，初始连接期限继续运行。具体connected事件/重建归属问题需要定向复现。

## 六维健康评估：第二段

| 维度 | 观测 | 判断 |
| --- | --- | --- |
| 网络健康 | UDP、IPv4、srflx→srflx；RTT中位13ms、P95 21ms，峰值137ms；jitter中位3ms、P95 7ms。累计丢包165/317942≈0.052%，近5秒窗口峰值约1.23%；视频码率中位20.39Mbps。 | 主体链路健康，存在短时波动。80Mbps为请求上限，实际码率由内容与编码决定。 |
| 恢复链健康 | NACK=123、PLI=3、FIR=0；127个监督样本connectionRecoveryActive=false、decodePressureSustainedMs=0；quality/network状态stable 65次、unknown 62次，fresh=false 5次。 | 应用层连续连接与纠错有证据，内部ReferenceChain/RecoveryLedger/cleanAnchorCommitted/DisplayStable缺SDK投影，结论保留此能力边界。 |
| media supply | 实际源2560×1440，末次received=decoded=8606，libwebrtc framesDropped=0；按真实窗口加权接收/解码58.87FPS。 | 解码供给足够；下降发生在解码后的显示路径。 |
| steady supply | 145个有效窗口中141个解码≥50FPS；freezeCount=5、totalFreezesDuration=3431ms。 | 多数窗口接近60FPS；另有局部短停，持续低呈现FPS贯穿各30秒区间。 |
| submit/present长尾 | 同帧提交→GPU完成P50 0.824ms、P95 2.052ms、P99 2.839ms；系统drawable的滚动240样本P95约24–25ms。GPU完成→呈现Trace主线程处理P50 51.747ms、P95 71.874ms、P99 105.110ms，最大710.515ms。 | GPU工作很快，主线程处理与在途帧释放存在明显长尾。 |
| 帧供给delta | 相邻窗口加权submit=18.080、present=18.073、drawable=18.046FPS；末次submit2643/present2642/drawable2638，入口合并丢弃5957，mailboxOverwrite=0。 | 接收/解码到提交损失约69.2%；进入GPU的帧基本完成并真实呈现。 |

supervision的滑动窗口present FPS中位20.71，常见20–25；每秒Trace窗口全程加权均值18.07。两者观察窗口不同，均证明实际显示推进偏低。按首次呈现后的30秒分段，呈现均值依次约16.98、17.62、20.76、16.99、17.97FPS。

RTC与display快照间存在异步采样边界；末次8606解码帧对5957丢弃+2642呈现留有7帧差额。采用同sampleId的delta/真实sampleWindowMs计算速率，避免以末尾异步累计量强求逐帧等式。全部掉帧事件5990为截至退出的入口丢弃计数，末次快照5957为前一秒采样值。

数值见[health-summary.json](health-summary.json)、[逐窗口CSV](steady-windows.csv)、[因果计算](causal-analysis.json)及[网络恢复摘要](recovery-network-summary.json)。

## 20多帧的直接机制与代码疑因

1. `StreamingVideoFrameIngress`接收libwebrtc解码帧，将最新帧投递给MainActor。MainActor处理滞后时，pending帧被新帧替换，Trace输出`ingressCoalesced`。
2. Metal mailbox保留一个inflight帧；GPU完成回调采样时间后切回MainActor，`handlePresentedFrame`才调用`mailbox.finishRender`释放该帧槽。第二段GPU完成后的处理延迟中位52ms，将后续提交限制到约18–20FPS量级。
3. 每帧busy、submit、present、busy清除、drawable呈现及掉帧健康回报都派发presentation事件。`StreamSessionActor.displayHealthUpdated`在playbackReady后每次调用`publish(.playing)`，`StreamingFeatureStore.apply`每次写入`@Published state`并记录同名状态。
4. 第二段出现15926条重复playing状态，约109次/秒、约6次/呈现帧。重复状态通知与MainActor任务派发会触发SwiftUI更新并争用帧入口/完成处理。第一段缺playing锚点，GPU完成后的主线程处理延迟中位0.38ms；第二段约52ms。该对照支持优先检查这条热路径，其具体CPU占比与修复收益需要Time Profiler或A/B确认。

代表帧1534：seq15641接收、15642提交、15649呈现；同帧`sourceMonotonicMs-presentedAtMonotonicMs`=51.754ms。该差分包含GPU回调排队、Swift锁内帧反馈记账和写Trace之前的遥测处理；writerQueueDelay独立计算，使用同一单调时钟。具体排队/执行占比由后续分段埋点或Time Profiler确定。

源码锚点：`StreamingMetalRenderer.swift:733/1198/1232/1848`，`StreamSessionActor.swift:572`，`StreamingRuntime.swift:614`。相关当前源码哈希和片段已保存到[source-hashes.json](source-hashes.json)与[source-excerpts.txt](source-excerpts.txt)。

## 锐化与超分有效性

- 锐化shader已接入NV12/I420/BGRA，归一化强度为0、0.25、0.65；兼容模式采用默认效果。Trace缺`sharpness`、兼容模式及实际configuration投影，因此本次启用强度与视觉增益只能留待运行配置/同场景截图验证。
- 全文件`spatialScalerConfigured`为0。当前实现每次首次成功创建MetalFX资源都会记录此key事件；本文件从启动完整连续记录，因此本次成功配置路径缺运行证据。具体原因涉及设置开关、兼容模式、设备支持和放大倍率>1.01，当前Trace字段不足以区分。
- `videoSurfaceSized`为669×880，源码记录的是bounds点尺寸；源视频2560×1440。drawable实际像素、内容模式与输出尺寸缺日志，超分触发倍率保留待测。放大收益测试使用720p/1080p输入和较大窗口，并对照实际MetalFX激活状态。
- `completionHandlerDelayMs`被脱敏规则中的`handle`片段误匹配，3207条present事件均为`<redacted>`。本轮使用两个保留的源时间字段恢复处理延迟；后续埋点应采用安全字段名或精确敏感字段匹配。

## 建议顺序与归档

优先给playing状态发布去重、合并高频显示健康回报，检查GPU完成后帧槽释放的及时性；保留低延迟latest-only策略。补齐实际锐化、兼容模式、MetalFX支持/激活/回退原因、drawable像素/输出尺寸及安全的主线程处理延迟字段，再做同场景A/B与60FPS回归。

本轮运行时代码保持原样，原始Trace在项目`runtime-logs/ios`及Downloads保留。临时分析目录仅清理本轮r2；前序r1系统日志继续保留。清理记录见[cleanup.json](cleanup.json)，证据哈希见[artifact-manifest.json](artifact-manifest.json)。
