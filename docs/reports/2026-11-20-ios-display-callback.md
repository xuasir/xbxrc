# iOS 显示回调与健康回报收敛

2026-11-20。实现与定向验收完成，真实Mac串流帧率收益由新运行Trace验收。

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

证据见[验收目录](../../dev-docs/validation/2026-11-20-ios-display-callback/README.md)。项目当前原始Trace包含36195行，新增计时字段记录数为0，任务保持Validation Pending。建议在My Mac (Designed for iPad)沿原1440p设置运行同场景至少60秒，导出至runtime-logs/ios，复核重复状态数量、六项分段耗时以及网络、恢复、media supply、steady supply、submit/present长尾和帧供给delta。
