# iOS 显示回调收敛验收

2026-11-20原轮显示收敛已完成定向与新Trace验收，正常前台约58FPS。2026-11-23续接完成后台就绪补送、恢复状态收口、底层轨道身份去重及按需调度，最终45项模拟器与Device构建通过；本次帧率收益由新运行验收。

| 证据 | 结果 |
| --- | --- |
| [retry-simulator-summary.json](retry-simulator-summary.json) · [测试日志](retry-simulator-tests.log) | 最终iPhone 17e / iOS 27.0：33通过，0失败。含连续NV12回报频率/尾部和首帧验证。 |
| [initial-simulator-summary.json](initial-simulator-summary.json) | 首轮iPhone 18 Pro：33通过，0失败。 |
| [interruption-simulator-summary.json](interruption-simulator-summary.json) · [诊断](interruption-diagnostics.log) | 补充轮收到signal kill，测试启动连接提前结束。换目标复验全部通过。 |
| [最终设备构建](final-device-build-summary.log) · [设备首轮构建](device-build-summary.log) | Device SDK无签名构建成功。 |
| [base-analyzer-tests.log](base-analyzer-tests.log) · [session-boundary.log](session-boundary.log) · [swift-format.log](swift-format.log) | 分析器3项、会话边界和当前renderer/测试Swift格式通过。状态发布文件仅修改去重位置，保留原有周边格式。 |
| [task-changes.diff](task-changes.diff) · [source-hashes.json](source-hashes.json) | 本轮相对开始基线的源码差异与SHA-256。 |
| [续接核验](continuation-audit.json) · [源码快照](sources/iosapp/XBXRC/Platform/Streaming/StreamingMetalRenderer.swift) | 四份关键源码与最终验收哈希一致；原始成功结果包与归档测试日志核验通过。 |
| [新日志性能复验](fresh-trace/README.md) · [前后对照](fresh-trace/comparison.json) | 33877行、6362项完整分段计时；正常58FPS，GPU后记录中位0.329ms、P95 1.557ms。后台创建管线的playing缺口与恢复顺序继续收口。 |
| [11月23日后续修复](../2026-11-23-ios-display-lifecycle-frame-delivery/README.md) | 后台补送/恢复顺序/轨道稳定绑定/按需调度；45项模拟器、Device及17+3项分析器通过，当前帧率待新Trace。 |
| [cleanup.json](cleanup.json) · [artifact-manifest.json](artifact-manifest.json) | 本轮临时目录清理及归档指纹。 |

实现合同与耗时字段见[报告](../../../docs/reports/2026-11-20-ios-display-callback.md)。原始用户日志继续保留在Downloads与项目runtime-logs/ios；共享WebRTC/Rust缓存保留。

测试使用XBXRC scheme、`-parallel-testing-enabled NO -collect-test-diagnostics never`，选择`StreamingDisplayCallbackTests`、`TraceReliabilityTests`、`StreamingFrameFeedbackTests`及6项既有串流集成用例。临时构建和结果包归属`/private/tmp/xbxrc-ios-display-callback-20261120/r1`。

续接收尾已完成：相关构建、测试、导出和宿主进程均已结束，整个临时根目录及本任务系统临时目录已核验清理，回收978.29 MiB，清理后可用空间62.56 GiB。最终测试与失败诊断、设备构建摘要、四份源码快照及原始用户Trace继续保留。续接收尾时的原始Trace包含36195行，六项新增计时字段的记录数为0；随后用户提供的新Trace已完成[真实帧率与六维健康复验](fresh-trace/README.md)。
