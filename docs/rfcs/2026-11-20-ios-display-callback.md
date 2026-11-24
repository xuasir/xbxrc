# iOS GPU完成回调与健康回报收敛

状态：2026-11-23串流热路径显式高优先级、drain ticket/排队计时和触屏订阅收敛完成；52项模拟器、Device构建、格式/解析/会话边界/lint验收通过。基线新Trace串流core PASS，前台就绪补送3.087ms、每peer一次绑定；95.24秒对齐窗口解码59.94FPS、GPU完成52.52FPS、drawable52.48FPS，入口合并730帧、MainActor等待最高663.881ms。当前构建的帧率、排队长尾和播放期间多次恢复继续保持Validation Pending。

## 证据与范围

最新Mac Designed for iPad Home Trace第二段接收/解码58.9FPS、呈现约18FPS。成功GPU完成到主线程呈现Trace处理延迟中位51.7ms、P95 71.9ms，GPU提交到完成P95 2.05ms；重复playing发布15926次，入口合并丢弃约69%。[原始分析](../../dev-docs/validation/2026-11-20-xhome-mac-trace-analysis/latest-trace/README.md)。

修复沿原libwebrtc、CVPixelBuffer纹理映射、Metal命令队列与latest-only mailbox执行。GPU完成继续作为rendered/playing观测，系统drawable呈现仍独立记录。首帧、恢复、前后台、代次与失败重试合同保持。

## 实施合同

1. Actor按实际可见状态去重发布，FeatureStore在写Published前再次去重；真实状态变化正常送达，start/停止/前后台边界正确重建。
2. 显示健康回报在原renderer合并最新值，采用有界低频任务，保留首帧与context变化及时性、尾部快照与累计计数。
3. 成功GPU完成在回调中释放原mailbox在途帧；使用既有全生命周期唯一frameId隔离迟到回调。失败回调继续在原主线程重试路径处理。主线程延后遥测依据当前mailbox事实报告busy。
4. Trace分别记录GPU完成→帧槽释放、GPU完成→MainActor入口、反馈记账、遥测处理与主线程总处理；字段避开敏感片段误匹配，原累计量/窗口仍可复盘。

## 实现结果

- Actor在跨MainActor等待前保存已发布状态，FeatureStore在写Published前再次按值去重；start与stop重建发布基线。
- Renderer直接送达生命周期事件，健康快照每100ms合并一次，保留最新累计量。新context、首次呈现、前后台及shutdown边界及时送达或刷出尾部；每个surface最多保留一个待回报任务。
- 成功GPU完成回调直接释放锁保护的原mailbox帧槽；MainActor仅做反馈和遥测记账。失败继续重排，busy读取当前mailbox事实。前后台revision隔离同context下的迟到GPU/drawable记录，暂停时清理待提交时间且保留累计计数。
- 新增安全字段：`gpuToSlotReleaseMs`、`gpuToMainActorMs`、`frameFeedbackWorkMs`、`displayTelemetryWorkMs`、`mainActorWorkBeforeTraceMs`、`gpuToTraceRecordMs`。最后两项的测量截止点为调用Trace API之前，writer排队仍由既有字段独立记录。

最终iPhone 17e / iOS 27.0定向33项通过，包括8项新增、11项Trace可靠性、8项帧反馈及6项串流集成。连续NV12测试覆盖100ms回报节流和最终submit/present快照配平；设备增量构建、Swift格式及会话边界通过。首轮33项通过；补充轮测试启动被signal kill终止，换目标后的最终33项全部通过。[Report](../reports/2026-11-20-ios-display-callback.md) · [证据](../../dev-docs/validation/2026-11-20-ios-display-callback/README.md)。

## 验证与边界

- 定向回归：mailbox跨线程释放、reset/迟到完成隔离、失败重排、健康合并/尾部/重置、状态重复及前后台/新会话去重边界、耗时字段与脱敏。
- Swift 6编译与iOS定向XCTest、原Trace可靠性和串流合同回归；设备目标构建、格式与会话边界检查。
- 真实修复收益由用户新运行Trace验证六维健康。当前52ms覆盖排队与主线程处理，进一步用新分段字段确定占比；原媒体路径采用纹理映射和少量常量提交。
- 保留并发修改与暂存状态，归档结果并清理本轮临时构建/测试目录。

2026-11-20，续接收尾完成。四份关键源码与最终验收SHA-256一致，原始成功结果包确认33项通过，归档测试日志逐字节一致；中断结果包与signal kill诊断对应。[续接核验](../../dev-docs/validation/2026-11-20-ios-display-callback/continuation-audit.json)及四份源码快照已归档。相关进程结束后，临时构建、三个xcresult、重复日志和基线目录已清理，回收978.29 MiB，可用空间62.56 GiB；[清理记录](../../dev-docs/validation/2026-11-20-ios-display-callback/cleanup.json)保留路径与空间核验。当前用户Trace为修复前运行，真实性能继续由当前构建的新Trace验收。

## 新Trace复验与剩余收口

随后用户提供`runtime-trace-ios-1795187951399-1.jsonl`，33877行、两个Home尝试，6362次呈现保留全部六项计时。第二段正常前台片段完成58.26、58.86、57.47、58.39FPS，GPU完成→记录中位0.329ms、P95 1.557ms；帧槽释放P95 0.00258ms、主线程工作P99 0.962ms。对齐前台窗口解码→完成计数差2.19%，playing共7次，对应实际首次播放和三次恢复切换。新旧日志同为1440p，窗口、内容和网络路径有变化，分段计时与状态计数直接证明目标热路径收敛。[完整复验](../../dev-docs/validation/2026-11-20-ios-display-callback/fresh-trace/README.md)。

第一段在后台创建管线，displayPipelineReady一次性标记已设置，回报在presentationActive筛选处跳过送达，前台同context恢复缺少补送，持续出画面却缺playing。第二段恢复时新健康事实可先发布playing，validateResume返回后固定发布waiting，形成可见回退。后续保持既有readiness合同，补齐持久就绪事实的送达和同visibility revision内的恢复收口顺序。

第二段末尾两个RTC窗口接收/解码/包增量及码率为0，约3.15秒后因`dataChannel.closed.input`终止；RTT仍3ms，多通道随后关闭。2026-11-21用户确认主动关闭主机或会话，本段供给停止与通道关闭归为主动结束情境。第二段core闭环PASS，第一段playing缺口使总gate FAIL；保留Validation Pending，并以连续前台60秒及后台启动/多次恢复覆盖收口。

## 2026-11-23 续接实施合同

用户已要求修复后台就绪补送与恢复状态发布顺序，并明确帧损目标为解码约60FPS、实际上屏约58FPS之间的计数差。本轮在原renderer、Actor与latest-only入口中直接执行。

1. 管线就绪的观测与送达分别记账。观测事实保留到context结束；后台、回报接线与前台恢复边界补送当前context的就绪事实，每次前台区间送达一次。首帧接受、提交和呈现继续要求可见状态。
2. 恢复验证期间保存新上屏事实，playing同时要求当前visibility revision的会话验证通过和恢复后的上屏。validateResume与peer快照的每次异步返回核验generation、peerEpoch和visibility revision；等待状态先发布，验证完成按最新readiness收口，首帧守护仅在仍等待新上屏时装配。
3. 原线程安全入口每次MainActor处理最多接续两帧，然后让出执行机会；slot继续只保存最新帧。CADisplayLink按待处理帧启停，重排失败帧等待下一显示tick，GPU完成与新帧到达继续及时唤醒原scheduler。
4. 增加入口排队、接受处理、drawable获取与绘制准备的数值计时，保留原帧计数、drop reason、GPU完成与系统drawable独立观测。对照日志189帧应用丢弃均为ingressCoalesced、mailboxOverwrite=0；主线程排队及drawable获取各自占比由新字段继续验证。
5. `steadyMediaObserved`补充布尔`presentationActive`事实。串流core门仅对显式后台供给豁免“首显先于稳态”顺序项，并记录对应锚点；前台与旧版缺字段记录继续按原顺序检查，首显/playing/清理门槛完整保留。
6. 新Trace两个peer分别出现50/149次surface绑定，间隔中位1002ms，68/171次入口丢弃紧跟绑定50ms内。轨道发现按SDK包装对象地址去重，会在每秒stats扫描时重复更换首帧探针并触发surface解绑/重绑。改用libwebrtc的底层轨道相等性共享判断；同一native轨道的新包装对象维持原绑定，真正的新轨道与videoGeneration变化正常重装。用实际receiver生成的不同包装对象和同trackId的不同native轨道验证身份合同；实际帧率改善由新Trace复核绑定计数、入口drop及六维健康。

验收覆盖后台创建管线与晚接回报、恢复验证/新帧两种先后顺序、迟到验证与可见性变化、入口有界接续、失败重试、连续NV12呈现与字段脱敏；执行Simulator定向回归、Device无签名构建、Swift格式、会话边界与diff检查。新运行保持原1440p场景，连续前台至少60秒，并覆盖后台启动和多次恢复；同时复核网络、恢复、media supply、steady supply、submit/present长尾与帧供给delta。定向验证和源码归档至`dev-docs/validation/2026-11-23-ios-display-lifecycle-frame-delivery`；临时产物归属`/private/tmp/xbxrc-ios-display-lifecycle-20261123/r1`并按轮次清理。

本轮最终验收45项模拟器、Device无签名构建、17项core分析器、3项基础分析器、源码解析/四空格格式/会话边界/diff均通过。12项新增覆盖恢复竞态、后台补送、入口接续及实际SDK包装身份。七份源码与证据见[本轮验收](../../dev-docs/validation/2026-11-23-ios-display-lifecycle-frame-delivery/README.md)，临时清理见[清理记录](../../dev-docs/validation/2026-11-23-ios-display-lifecycle-frame-delivery/cleanup.json)。当前构建的真实帧率、呈现长尾和六维健康保持新运行验收入口。

2026-11-23收尾：相关进程已结束，专用临时根目录已清理857.72MiB，七份源码与验收证据保留；当前构建的新运行Trace继续作为真实帧率收口入口。

## 2026-11-23 新运行 Trace 验收

用户提供runtime-trace-ios-1795401150207-1.jsonl，26163行、单次Home尝试、两个peer代次。串流core PASS；后台创建管线在前台恢复后3.087ms补送原peer就绪事实，两个peer各绑定surface一次。恢复先waiting再validateResume，随后沿既有fallback重建peer2，首显后playing一次，最终user.stop清理闭合。播放期间多次恢复与同peer后台持续供给后直接恢复继续覆盖。

95个对齐前台窗口共95.24秒，接收/解码59.94FPS、GPU完成52.52FPS、drawable52.48FPS，解码→实际上屏计数差12.45%。原正常供给口径下，本轮drawable52.97FPS，前轮第二段58.32FPS；两份日志的场景、网络、时长与负载分别记录。5047接受−5次mailbox覆盖=5042提交=5042GPU完成；730帧应用丢弃全部为ingressCoalesced。seq25465 GPU→MainActor等待663.881ms，seq25468连续合并38帧，同窗口解码61帧、丢包增量0；优先定位MainActor长任务与入口drain的执行等待。

单帧提交→GPU完成P95 1.651ms，drawable获取P95 0.093ms；滚动drawable P95最大32.411ms、P99最大40.142ms。网络RTT P95 44ms、jitter P95 9ms，视频丢包42、NACK33、PLI1；SDK记录两次freeze共1314ms，其中一个约1秒接收/解码供给缺口与后续追赶完整保留。基础全流程gate FAIL的精确诊断为seq128、131两个图片候选开始缺少同operationId的终结事件；源码取消路径及邻接HTTP200锚点单独归档。结构、序列、预算、隐私与启动/库页面必需锚点通过。

原件、项目副本、逐窗口CSV、精确锚点、全量压缩core结果和源码指纹见[新Trace验收](../../dev-docs/validation/2026-11-23-ios-display-lifecycle-fresh-trace/README.md)。此前七份验收源码保持一致，代码与自动验收证据保留，任务继续Validation Pending；本轮分析临时目录按验收清单清理。

本轮分析与导出命令均已返回完成状态，专用临时根目录已清理，移除分配量4.89MiB；清理后可用空间51.88GiB。原始用户Trace、项目副本、源码/分析证据和共享Rust/WebRTC缓存保留。

## 2026-11-23 MainActor 排队续接合同

用户继续授权沿原串流显示链优化，并明确要求串流功能优先调度。本轮以730帧入口合并、663.881ms主线程等待为依据，在原帧入口、显示回调和触屏观察范围中实施。

1. 帧入口、尺寸就绪、GPU完成和系统drawable回调显式使用`TaskPriority.userInitiated`；100ms周期健康快照使用`utility`。首帧、context变化与生命周期尾部继续即时送达。优先级为调度提示，MainActor同步任务的实际占用由新计时及后续采样验证。
2. 原latest-only入口在锁内分配唯一drain ticket，包含任务入队时间；两帧有界接续后为下一任务分配新ticket。reset撤销当前ticket，迟到或重复任务按ticket隔离；reset与新帧并发时，新任务完整拥有新pending及替换计数。
3. 保留最新帧的`ingressQueueMs`，新增`ingressDrainQueueMs`和`ingressDrainWorkBeforeFrameMs`，分别观测drain调度请求到执行、同次drain内前序工作。初次请求时间在入口提交前取得，接续请求时间在前一轮结束时取得，涵盖短锁与任务投递开销。接受和`ingressCoalesced`丢弃事件携带相同排队事实，接受工作继续独立计时；数值字段遵循现有脱敏合同。
4. 触屏入口按设备可用性更新局部状态，触屏覆盖层仅订阅去重后的`isSuppressed`。触点通过现有UIKit显示链接直接绘制；抑制变化及时取消触点，布局绑定、退出释放与neutral合同完整保留。
5. 定向回归覆盖替换时保留入队时间、接续ticket、reset后迟到/重复任务、消费中reset、计时脱敏、连续后台NV12供给、健康尾部及触屏抑制。执行Simulator回归、Device无签名构建、四空格格式、Swift解析与会话边界检查。实际收益由当前构建的新Trace同时验证网络、恢复、media supply、steady supply、submit/present长尾与帧供给delta。

本轮证据归档至`dev-docs/validation/2026-11-23-ios-mainactor-drain`，专用临时目录为`/private/tmp/xbxrc-ios-mainactor-drain-20261123/r1`。保留原暂存状态、其他聊天并行修改和共享缓存，按任务清单完成清理。

本轮最终52项模拟器回归全部通过，包含4项新增；原始结果包摘要与逐用例树逐项核验。Device无签名构建、六份Swift解析、范围格式幂等、会话边界、前端lint与diff通过；六份验收快照与最终Simulator/Device执行指纹一致，原暂存及其他并行改动保持。两轮编译诊断与修复归档：方向选择函数补齐MainActor隔离，新宿主测试补齐SwiftUI导入，截图探针以关闭本地动画替代只读环境赋值。帧反馈测试的既有QoS等待诊断与前轮原始结果同源同消息。实际串流任务优先级的收益、664ms等待的占用栈和六维健康继续由当前构建的新Trace及采样验收。

本轮进程与结果导出均已完成，专用临时根目录清理865.59MiB，清理后可用空间46.54GiB。六份源码快照、52项原始测试摘要和逐用例树、设备摘要、失败诊断及用户Trace保留；共享缓存与原暂存保持。[清理记录](../../dev-docs/validation/2026-11-23-ios-mainactor-drain/cleanup.json)。

最终工作区核验确认五份保留源码与验收快照一致。临时UI截图探针在并行验收收尾中移除，既有测试文件前缀逐字节保持，所选52项用例保持；当前文件Swift解析通过。[探针收尾核验](../../dev-docs/validation/2026-11-23-ios-mainactor-drain/post-validation-concurrent-cleanup.json)保留差异与当时验收快照。
