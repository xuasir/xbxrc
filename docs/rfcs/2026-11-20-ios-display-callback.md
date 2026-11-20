# iOS GPU完成回调与健康回报收敛

状态：实现与定向验收完成；Mac Designed for iPad 的真实串流性能待新Trace复验。2026-11-20用户明确要求收敛重复状态、高频健康回报并改善帧槽释放及时性。

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
