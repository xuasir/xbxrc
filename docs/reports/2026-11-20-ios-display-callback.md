# iOS 显示回调与健康回报收敛

2026-11-23串流帧入口、GPU完成和drawable回调已显式使用高优先级，周期健康快照使用utility；入口ticket/排队计时与触屏订阅收敛完成。52项模拟器回归、Device无签名构建及格式/解析/会话边界/lint全部通过。基线新Trace后台补送3.087ms、每peer一次绑定、串流core PASS；95.24秒对齐窗口drawable52.48FPS，730帧入口合并与663.881ms MainActor等待作为当前构建后续复验基线。实际帧率、等待长尾和播放期间多次恢复保持Validation Pending。

原Home日志的接收/解码约58.9FPS、提交/呈现约18FPS。成功GPU完成回调到主线程呈现记录的中位间隔51.7ms、P95 71.9ms，提交到GPU完成P95 2.05ms；重复playing状态15926次，解码后的入口合并丢弃约69%。

帧槽现在于成功GPU完成回调直接释放。后续显示tick可以取得新帧，主线程延后的反馈记账和遥测读取当前busy事实。失败回调继续在主线程重排；唯一frameId、会话context与前后台revision共同隔离迟到回调。暂停时清理待提交时间，累计提交/呈现计数继续保留。

Actor按实际可见状态去重，在跨MainActor等待前保存基线；FeatureStore在写Published前再次去重。新会话与stop重建基线，playing→suspending→waitingForFirstFrame→playing等真实变化正常送达。

健康回报在原renderer合并最新累计快照，每100ms派发一次，保留一个定时任务。首帧及context变化即时送达，前后台和shutdown边界刷出尾部快照。逐帧Trace与系统drawable观测继续保留。

## GPU完成后时间的含义

原约52ms差分覆盖完成回调入口到主线程写Trace之前的墙钟时间，包含排队、帧反馈锁内记账和遥测处理。当前设备媒体路径通过CVMetalTextureCache映射CVPixelBuffer，CPU提交绘制常量和帧反馈元数据；模拟器I420兼容路径采用像素上传。GPU完成与drawable的系统呈现独立计时，drawable延迟还包含显示调度。

| 新Trace字段 | 测量范围 |
| --- | --- |
| gpuToSlotReleaseMs | GPU完成回调入口→mailbox释放结束 |
| gpuToMainActorMs | GPU完成回调入口→MainActor处理入口 |
| frameFeedbackWorkMs | MainActor入口→代次检查及帧反馈记账结束 |
| displayTelemetryWorkMs | 反馈记账结束→呈现遥测和健康合并结束 |
| mainActorWorkBeforeTraceMs | MainActor入口→调用呈现Trace之前 |
| gpuToTraceRecordMs | GPU完成回调入口→调用呈现Trace之前 |

dev/production脱敏回归均保留六项数值。Trace writer的队列等待继续由既有源时间字段独立记录。

## 验证

- 最终iPhone 17e / iOS 27.0：33项通过，0失败。8项新增覆盖后台释放、迟到/重复完成、失败重排、健康合并/尾部/代次、分段计时/脱敏、暂停待提交时间和Actor新会话/恢复去重；19项既有Trace/帧反馈与6项串流集成继续通过。
- 连续NV12后台供给测试完成超过10帧呈现，普通健康回报相邻间隔至少90ms，尾部提交/呈现配平；首次呈现保持及时。
- GPU完成跨线程测试在MainActor尚未记账前取得下一帧，并验证上一帧记账时新帧的busy事实。
- Device SDK最终源码无签名构建通过；Swift格式、git diff check、会话边界与3项既有iOS Trace分析器回归通过。
- 首轮iPhone 18 Pro定向33项通过。补充轮启动时收到signal kill，结果包另记测试连接建立前退出；最终换目标33项通过。中断摘要随证据保留。

2026-11-20续接核验确认四份关键源码与最终验收哈希一致，原始成功结果包及归档测试日志均确认33项通过；四份源码快照和[续接核验](../../dev-docs/validation/2026-11-20-ios-display-callback/continuation-audit.json)已归档。相关构建、测试、导出和宿主均已结束，临时根目录及本任务系统临时目录已核验清理，回收978.29 MiB，可用空间62.56 GiB。最终测试摘要、失败诊断和原始用户Trace继续保留，共享WebRTC/Rust缓存保留；路径、产物清单和空间变化见[清理记录](../../dev-docs/validation/2026-11-20-ios-display-callback/cleanup.json)。

证据见[验收目录](../../dev-docs/validation/2026-11-20-ios-display-callback/README.md)。续接收尾当时的原始Trace包含36195行，新增计时字段记录数为0，任务保持Validation Pending。当时建议在My Mac (Designed for iPad)沿原1440p设置运行同场景至少60秒，导出至runtime-logs/ios，复核重复状态数量、六项分段耗时以及网络、恢复、media supply、steady supply、submit/present长尾和帧供给delta。

## 后续新运行复验

随后收到`runtime-trace-ios-1795187951399-1.jsonl`，33877行、两个Home尝试；启动/schema/序列/隐私/配对/预算通过，6362次呈现保留全部六项计时。第二段正常前台完成约58.3–58.9FPS；GPU完成→记录中位0.329ms、P95 1.557ms，主线程工作P99 0.962ms，帧槽释放P95约2.6微秒。对齐前台窗口的解码→完成计数差为2.19%，playing共7次，原15926次重复发布已收敛。[前后对照与六维健康](../../dev-docs/validation/2026-11-20-ios-display-callback/fresh-trace/README.md)保留全部范围与计算方法；窗口和网络路径变化构成这组对照的观察边界。

新证据同时定位后台创建管线时一次性displayPipelineReady事实送达缺口：renderer设置已报告标记，后台回调筛选跳过送达，前台恢复缺少补送。第一段2223次完成且约58FPS，却持续缺playing。第二段三次恢复存在playing→验证完成→waiting→playing的短时回退。保持Validation Pending，后续补齐持久就绪事实及恢复发布顺序。

第二段结束前出现两个0供给RTC窗口，RTT仍3ms，最后完成到输入通道关闭3.149秒；随后多个通道关闭并以retryable的`dataChannel.closed.input`终止，清理完整。2026-11-21用户确认主动关闭主机或会话，本段供给停止及通道关闭归为主动结束情境，归因已收口。第二段串流core独立PASS；第一段playing与后台接收/前台显示的顺序诊断使总gate FAIL。新文件原件及项目副本、六份源码哈希、原始锚点、CSV与分析器结果已归档。

## 2026-11-23 后台恢复与帧供给续接

后台创建管线时，就绪观测与实际回报送达分别记账。能力事实跨前后台保留，恢复和晚接sink补送当前context；每个前台区间送达一次，context切换释放旧事实。后台首帧只建立管线能力，接受/提交/呈现继续按可见性执行。

恢复先发布waiting，validateResume与peer快照异步返回同时核验generation、peerEpoch、visibility revision。验证期间的新上屏事实保存，验证通过后直接按最新readiness决定playing；验证失败保持终止流程，迟到返回与新可见性隔离。后台在异步麦克风清理之前撤销旧呈现事实，首帧守护仅在仍等待恢复新上屏时装配；重复active通知合并。

原日志的189帧应用丢弃全部为ingressCoalesced，mailbox覆盖为0；正常解码59.94FPS、完成58.62FPS、系统drawable58.32FPS。进一步统计同peer的surface绑定50/149次，间隔中位约1002ms；171次丢弃事件中的68次发生在绑定后50ms内。receiver getter反复产生新的SDK包装对象，原地址比较会让每秒stats扫描更换首帧探针并推动surface解绑/重绑。现在runtime与surface共用底层轨道相等性；真实新轨道与videoGeneration变化保留重装。实际SDK测试覆盖连续60次receiver getter的新包装对象，以及同trackId的不同native轨道。[统计及原始锚点](../../dev-docs/validation/2026-11-23-ios-display-lifecycle-frame-delivery/baseline-analysis.json)保留关联证据；帧率改善由新运行验证。

原latest-only入口每次MainActor处理最多接续两帧，随后让出执行机会；CADisplayLink按待处理帧启停。成功GPU完成和新帧及时唤醒原scheduler，失败重排等待下一显示tick。新增ingressQueueMs、frameAcceptWorkMs、drawToSubmitMs、drawableAcquireMs分离入口排队、接受工作、绘制准备与drawable获取。steadyMediaObserved增加presentationActive布尔事实，core门记录显式后台供给先于首显的锚点；前台、缺字段与异常值继续沿原顺序检查，首显/playing/清理门槛保留。

最终iPhone 17e / iOS 27.0定向45项通过、0失败，包含12项新增与既有帧反馈/Trace/串流集成；Device SDK无签名构建、四空格格式、Swift/Python解析、会话边界、diff与17+3项分析器通过。七份最终源码快照及SHA-256、测试摘要、失败诊断与配置见[本轮验收](../../dev-docs/validation/2026-11-23-ios-display-lifecycle-frame-delivery/README.md)。源码保持现有Rust会话/信令、libwebrtc传输/解码及Metal显示主线。

任务保持Validation Pending，当前代码的真实帧率与长尾等待新Trace。建议固定原1440p场景和窗口，连续前台至少60秒，覆盖后台启动与多次恢复，并核对playing、触控/菜单、退出，以及网络、恢复链、media supply、steady supply、submit/present长尾与帧供给delta。临时构建、结果包和一次性日志按[本轮清理记录](../../dev-docs/validation/2026-11-23-ios-display-lifecycle-frame-delivery/cleanup.json)收尾，原始用户Trace及共享WebRTC/Rust缓存保留。

本轮收尾确认构建/测试/导出进程和三个测试宿主均已结束。专用临时根目录已移除，清理分配量857.72MiB、观察可用空间增长814.87MiB，清理后可用55.62GiB；源码证据、原始Trace和共享缓存保留，路径与空间核验归档。

## 2026-11-23 新运行 Trace 验收

用户提供runtime-trace-ios-1795401150207-1.jsonl，26163行、单次Home尝试、两个peer代次。串流core PASS；后台创建管线在前台恢复后3.087ms补送原peer就绪事实，两个peer各绑定surface一次。恢复先waiting再validateResume，随后沿既有fallback重建peer2，首显后playing一次，最终user.stop清理闭合。播放期间多次恢复与同peer后台持续供给后直接恢复继续覆盖。

95个对齐前台窗口共95.24秒，接收/解码59.94FPS、GPU完成52.52FPS、drawable52.48FPS，解码→实际上屏计数差12.45%。原正常供给口径下，本轮drawable52.97FPS，前轮第二段58.32FPS；两份日志的场景、网络、时长与负载分别记录。5047接受−5次mailbox覆盖=5042提交=5042GPU完成；730帧应用丢弃全部为ingressCoalesced。seq25465 GPU→MainActor等待663.881ms，seq25468连续合并38帧，同窗口解码61帧、丢包增量0；优先定位MainActor长任务与入口drain的执行等待。

单帧提交→GPU完成P95 1.651ms，drawable获取P95 0.093ms；滚动drawable P95最大32.411ms、P99最大40.142ms。网络RTT P95 44ms、jitter P95 9ms，视频丢包42、NACK33、PLI1；SDK记录两次freeze共1314ms，其中一个约1秒接收/解码供给缺口与后续追赶完整保留。基础全流程gate FAIL的精确诊断为seq128、131两个图片候选开始缺少同operationId的终结事件；源码取消路径及邻接HTTP200锚点单独归档。结构、序列、预算、隐私与启动/库页面必需锚点通过。

原件、项目副本、逐窗口CSV、精确锚点、全量压缩core结果和源码指纹见[新Trace验收](../../dev-docs/validation/2026-11-23-ios-display-lifecycle-fresh-trace/README.md)。此前七份验收源码保持一致，代码与自动验收证据保留，任务继续Validation Pending；本轮分析临时目录按验收清单清理。

本轮分析与导出命令均已返回完成状态，专用临时根目录已清理，移除分配量4.89MiB；清理后可用空间51.88GiB。原始用户Trace、项目副本、源码/分析证据和共享Rust/WebRTC缓存保留。

## 2026-11-23 串流优先级与入口排队收敛

帧入口、尺寸就绪、GPU完成和系统drawable回调使用`TaskPriority.userInitiated`，周期健康快照使用`utility`。首帧、context变化和生命周期尾部保持即时回报。优先级用于调度选择，MainActor同步任务仍按实际执行时长占用主线程；当前构建的收益以新Trace复验。

原latest-only slot在锁内分配单调递增ticket；最多接续两帧后，为下一任务建立新ticket和请求时间。reset撤销当前ticket，迟到、重复和消费中reset的旧任务均按归属筛选，新pending与替换计数由新任务消费。该边界覆盖前后台、换context及shutdown时已排队的入口任务。

| 字段 | 观测范围 |
| --- | --- |
| ingressDrainQueueMs | drain调度请求→MainActor drain开始，含短锁及任务投递开销 |
| ingressDrainWorkBeforeFrameMs | drain开始→当前帧接受入口，第二帧覆盖前序帧同步工作 |
| ingressQueueMs | 最新帧到达→当前帧接受入口 |
| frameAcceptWorkMs | 当前帧接受入口→调用接受Trace之前 |

接受与`ingressCoalesced`丢弃事件携带同一组排队事实，连续38帧替换保持最初调度时间。dev/production脱敏回归均保留数值字段，原GPU、drawable与累计帧计数继续独立观测。

触屏入口使用设备可用性的局部状态并按值更新，覆盖层仅订阅去重后的`isSuppressed`。触点与视觉绘制继续通过原Accumulator和UIKit显示链接更新；抑制时取消触点，恢复后先取得neutral再接受新触点。宿主回归实际验证诊断状态更新期间保持原Surface，抑制后触点释放及命中区撤销，恢复后命中与输入再次生效。

最终iPhone 17e / iOS 27.0共52项通过、0失败，新增4项覆盖替换计时、跨线程reset/迟到/重复、消费中reset及宿主触屏抑制。既有首帧、后台补送、恢复竞态、GPU释放、失败重排、健康尾部、Trace和帧反馈回归继续通过；Device无签名构建、六份Swift解析、四空格格式幂等、会话边界、前端lint及diff通过。六份验收快照与两次最终执行指纹一致，五份保留源码与工作区一致；原暂存和其他聊天并行改动保留，用户Trace原件与项目副本保持同一SHA-256。

前两轮构建诊断已归档并修复：方向选择辅助函数补齐MainActor隔离，新宿主测试补齐SwiftUI导入，截图探针使用关闭本地动画的transaction。结果摘要首次导出需写TestReport缓存，系统访问后完成原始摘要与逐用例树导出。帧反馈测试的既有QoS等待警告与前轮原始结果同源同消息；设备构建的既有Text拼接弃用诊断随原始构建摘要保留。

进程核验确认本轮构建/测试/导出完成，专用临时根目录清理865.59MiB，清理后可用空间46.54GiB。原始结果摘要、逐用例树、成功和失败压缩日志、六份源码及用户Trace继续保留；共享缓存与原暂存状态保持。[清理记录](../../dev-docs/validation/2026-11-23-ios-mainactor-drain/cleanup.json)覆盖四个结果包、两套DerivedData、系统临时子目录、重复日志、脚本与基线。

[本轮证据](../../dev-docs/validation/2026-11-23-ios-mainactor-drain/README.md)包含逐用例结果、两轮失败诊断、源码快照和执行命令。建议使用当前构建固定原1440p场景，连续前台至少60秒并覆盖后台启动和多次恢复；同步复核网络、恢复链、media supply、steady supply、submit/present长尾和帧供给delta，以新排队字段定位长等待及入口合并。

最终工作区核验发现临时UI截图探针已按并行验收收尾移除，探针所在文件的既有前缀和所选52项用例保持，当前文件Swift解析通过。该临时探针的只读环境编译修复随探针移除；当时成功编译的完整快照、当前保留源码及[收尾差异](../../dev-docs/validation/2026-11-23-ios-mainactor-drain/post-validation-concurrent-cleanup.json)分别记账。
