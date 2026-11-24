# 显示回调修复后新 xHome Trace 复验

2026-11-20，用户提供 `runtime-trace-ios-1795187951399-1.jsonl`。**正常前台呈现恢复到约58FPS，GPU完成到主线程记录的中位间隔由上一份日志约51.75ms降至0.33ms，成功GPU回调中的帧槽释放P95约2.6微秒。第一段后台启动的playing状态缺口与恢复状态回退继续作为流程待办；2026-11-21用户确认第二段末尾属于主动关闭主机或会话。**

本轮执行日志分析、源码核对和证据归档。运行时代码沿已验收版本保持。原件保留在Downloads，逐字节一致的项目副本位于 `runtime-logs/ios/runtime-trace-ios-1795187951399-1.jsonl`。下述数值使用原始行与同一单调时钟，字段中的时间单位均为毫秒。

## 范围与门禁

- 单个进程session，两个Home尝试，generation分别为1、3；33877行，seq=1–33877连续。北京时间23:19:11.399–23:24:07.993，296.594秒，全部dev profile。
- 文件20267868字节，19.33MiB，低于32MiB单文件预算。提交集包含这一份文件；writer目录的完整轮转留存需要其余原始文件。完整JSON、schema、序列、隐私、配对及预算检查通过；pressure notice为0，文件头storageFailures/storageDroppedRows/durabilityUncertainRows为0。
- 启动范围gate为PASS。全流程gate为FAIL，缺少云库页面、目录激活和库呈现三个锚点；本次记录Home入口，相关库UI覆盖留待库场景验收。
- 串流core总gate为FAIL。第二段独立PASS，peerEpoch=2，连接、控制/输入握手、首帧、稳态及清理齐全。第一段缺playing，并触发`displayFirstPresent < steadyMediaObserved`顺序门：后台先接收媒体、回到前台后首次显示，与当前steady事件的接收进度语义一致；验证器需要按可见性区分该顺序。playing缺口属于真实状态流程问题。
- writer队列等待全文件P50 0.207ms、P95 0.921ms、P99 2.774ms。6362次GPU完成均带六项计时，字段保留为数值。

[全流程门](base-analysis.json) · [启动门](startup-analysis.json) · [串流core门](core-analysis.json) · [独立算术核验](validation.json)。

## 前后对照与CPU耗时

| 观测 | 修复前第二段 | 新日志第二段 |
| --- | ---: | ---: |
| 正常显示推进 | 实际drawable约18.05FPS | 正常供给窗口实际drawable58.32FPS；四个前台片段完成速率58.26、58.86、57.47、58.39FPS，其中第三段仅1.34秒 |
| GPU完成→调用呈现Trace，中位 / P95 | 51.75 / 71.87ms | 0.329 / 1.557ms |
| GPU完成→MainActor入口，中位 / P95 / P99 | 旧字段覆盖排队与处理总量 | 0.0206 / 1.127 / 21.008ms |
| GPU完成→帧槽释放，中位 / P95 / P99 | 释放位于主线程记账路径 | 0.00104 / 0.00258 / 0.00471ms |
| 主线程入口→调用Trace，中位 / P95 / P99 | 旧字段覆盖排队与处理总量 | 0.274 / 0.757 / 0.962ms |
| 帧反馈记账，中位 / P95 | 旧日志缺分段字段 | 0.00550 / 0.01754ms |
| 显示遥测与健康合并，中位 / P95 | 旧日志缺分段字段 | 0.266 / 0.734ms |
| 同帧提交→GPU完成，P95 | 2.052ms | 2.523ms |
| playing状态记录 | 15926次 | 7次，对应首次播放及三次恢复的实际状态切换 |
| 对齐前台窗口的解码→完成计数差 | 约69.2%的显示供给损失 | 2.19% |

同为2560×1440，当前窗口点尺寸1330×998，上一份为669×880；当前host→host直连，上一份为srflx→srflx，内容和码率也有变化。这组数据构成真实运行对照，分段计时、唯一帧配对和状态计数直接验证热路径收敛；严格同场景A/B需固定窗口、内容和网络路径。

正常供给定义为同sampleId、完全前台且解码≥50FPS的窗口：第二段66个窗口共66.185秒，解码59.94、提交/完成58.62、系统drawable58.32FPS。含尾部零供给的69窗口结果保留在六维表中。[正常供给窗口摘要](normal-supply-summary.json)可从CSV独立复算。

199条surface附着记录均标记`metalZeroCopy`。设备视频通过`CVPixelBuffer → CVMetalTextureCache`纹理映射，CPU处理绘制常量和反馈元数据。当前主线程工作P99低于1ms，GPU完成到记录的P99仍为21.22ms，稀疏长尾主要落在MainActor排队阶段。例如第二段P99代表帧的原始行保存在[evidence-anchors.json](evidence-anchors.json)。Trace writer排队独立计量。

代表首帧：seq246提交frame1；seq249已提交frame2；seq250才对frame1完成主线程记账。frame1的GPU后排队41.80ms，帧槽释放0.00125ms，证明后续提交已能使用提前释放的槽。

## 六维健康

| 维度 | 第一段 | 第二段与判断 |
| --- | --- | --- |
| 网络健康 | host→host UDP；RTT中位3ms、P95 10ms；jitter P95 7ms。 | host→host UDP；RTT中位3ms、P95 9ms、最大12ms；jitter P95 3ms。所有RTC样本packetsLost=0。主体链路健康，末尾接收供给变为0。 |
| 恢复链健康 | 45个监督样本connectionRecoveryActive=false；PLI=1、NACK/FIR=0。 | 74个样本connectionRecoveryActive=false，63次stable、11次unknown；PLI=2，NACK/FIR=0，72次fresh、2次stale。ReferenceChain、RecoveryLedger、cleanAnchorCommitted与DisplayStable属于SDK内部能力边界，当前Trace提供应用层观测。 |
| media supply | 前台36个对齐窗口，接收/解码59.95FPS。 | 前台69个对齐窗口，接收/解码57.98FPS；统计包含尾部两个0供给窗口。正常阶段接近60FPS，源2560×1440，libwebrtc framesDropped=0。 |
| steady supply | 36个有效前台窗口全部解码≥50FPS，freezeCount=0。 | 69个窗口中66个解码≥50FPS、一个约45FPS、两个0FPS；freezeCount=0。后台期间解码继续推进，最终零供给区间由counter delta直接观测。最长连续前台呈现约31.8秒，四段合计约70.8秒。 |
| submit/present长尾 | GPU完成P95 2.668ms；drawable滚动窗口P95约23–28ms。 | GPU完成P95 2.523ms、P99 3.819ms；drawable滚动窗口P95约25–26ms。GPU后记录P95 1.557ms、P99 21.220ms；主线程工作P99 0.962ms。 |
| 帧供给delta | 同36窗口解码2163、完成2112、drawable2084；计数差2.36%，完成→drawable差1.33%。 | 同69窗口解码4012、完成3924、drawable3905，计数差2.19%，完成→drawable差0.48%；加权完成56.71、drawable56.43FPS，包含尾部供给停止。正常前台完成约58.3–58.9FPS。 |

全文件接受/提交6366帧、完成6362帧，原始入口合并丢弃189帧；两段mailboxOverwriteCount均为0。后台持续解码、前台暂停呈现，两段最后RTC累计解码合计11475帧，与前台显示6362帧覆盖不同可见性区间。上述delta采用同sampleId、完整前台区间和实际sampleWindowMs计算，归档在[逐窗口CSV](steady-windows.csv)和[健康摘要](health-summary.json)。100ms健康回报采用最新累计快照，秒级窗口边界可能含约6帧采样错位，部分单窗口速率超过60FPS。

## 生命周期待办与结束记录

### 后台创建管线的一次性就绪事实

第一段seq32进入后台，seq64记录displayPipelineReady；seq91连接成功、seq132控制就绪、seq145首视频帧、seq165接收稳态。seq243回到前台，seq251首次呈现，随后2223帧完成、正常约58FPS，playing记录数为0，直至seq11623主动user.stop。

当前renderer在记录一次性管线就绪时设置`didReportDisplayPipelineReady=true`，随后`emitPresentationEvent`在presentationActive=false时跳过送达；FeatureStore也按presentationActive筛选回报。恢复时保留同context和该一次性标记，管线就绪事实缺少重送路径。`StreamingPlaybackReadiness.requiredFacts`包含displayPipelineReady，因此Actor持续停在waitingForFirstFrame。这条源码链与原始时间线共同支持高置信度根因；Actor的完整readiness集合仍缺直接Trace投影。

第二段三次恢复还出现`playing → resumeValidated → waitingForFirstFrame → playing`。`setVisible(true)`等待validateResume期间，新健康快照已能提交playing；验证返回后又固定发布waiting。需要将恢复验证与新帧事实的收口顺序统一，保证最终状态按同一visibility revision推进。

### 尾部供给停止与输入通道关闭

第二段最后完成seq33808，GPU时间309078468.600ms。seq33827/33840连续两个RTC窗口framesReceivedDelta、framesDecodedDelta、packetsReceivedDelta和receiveBitrate均为0；RTT仍3ms。seq33851监督presentAge=3106ms、presentFps=0，随后seq33852 peerDisconnected，seq33853输入通道closed，seq33863选择`dataChannel.closed.input`，retryable=true。seq33870–33872完成peer/remote session/access清理。

最后完成到终止3.149秒。Trace记录媒体供给停止先于输入通道关闭，最终多个通道同时关闭。2026-11-21用户明确确认现场操作为“主动关闭主机或会话”，本段供给停止与通道关闭归为主动结束情境；客户端按dataChannel.closed.input终止并完成清理。[用户确认](user-confirmation.json)与[尾部原始行](terminal-anchors.json)共同保留归因依据。

## 收口与后续

显示吞吐与GPU完成回调的目标修复已获得真实运行证据。显示任务保持Validation Pending，保留后台启动readiness与恢复发布顺序待办。优先修复一次性就绪事实的前台补送和恢复状态顺序，再以连续前台至少60秒验收playing、触控/菜单和退出。尾部通道关闭已按用户操作事实归为主动结束情境。

原始用户文件和项目副本保留，六份当前源码哈希及关键片段归档。本轮临时根目录为`/private/tmp/xbxrc-ios-display-callback-trace-20261120/r1`，相关分析进程均已结束，临时目录与系统临时匹配路径核验清理，回收5.91MiB；清理结果见[cleanup.json](cleanup.json)，证据指纹见[artifact-manifest.json](artifact-manifest.json)。[分析脚本](analyze_post_fix.py)使用标准库，分位数采用nearest rank，速率采用真实窗口加权；[输入与方法](analysis-input.json)和[核验](validation.json)可复算。

## 2026-11-23 后续代码验收

后台就绪补送与恢复状态发布顺序已修复；每秒重复轨道绑定改用底层轨道相等性，入口采用有界接续和按需显示调度。最终45项模拟器、Device无签名构建及17+3项分析器通过。[本轮证据](../../2026-11-23-ios-display-lifecycle-frame-delivery/README.md)保留SDK包装身份回归与七份源码；原日志中的状态缺口和计数维持原始分析口径，优化后的实际帧率由新运行Trace验证。
