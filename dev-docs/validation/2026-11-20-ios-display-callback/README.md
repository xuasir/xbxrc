# iOS 显示回调收敛验收

2026-11-20。当前实现已完成定向验收，Mac真实串流性能接续由用户运行。

| 证据 | 结果 |
| --- | --- |
| [retry-simulator-summary.json](retry-simulator-summary.json) · [测试日志](retry-simulator-tests.log) | 最终iPhone 17e / iOS 27.0：33通过，0失败。含连续NV12回报频率/尾部和首帧验证。 |
| [initial-simulator-summary.json](initial-simulator-summary.json) | 首轮iPhone 18 Pro：33通过，0失败。 |
| [interruption-simulator-summary.json](interruption-simulator-summary.json) · [诊断](interruption-diagnostics.log) | 补充轮收到signal kill，测试启动连接提前结束。换目标复验全部通过。 |
| [最终设备构建](final-device-build-summary.log) · [设备首轮构建](device-build-summary.log) | Device SDK无签名构建成功。 |
| [base-analyzer-tests.log](base-analyzer-tests.log) · [session-boundary.log](session-boundary.log) · [swift-format.log](swift-format.log) | 分析器3项、会话边界和当前renderer/测试Swift格式通过。状态发布文件仅修改去重位置，保留原有周边格式。 |
| [task-changes.diff](task-changes.diff) · [source-hashes.json](source-hashes.json) | 本轮相对开始基线的源码差异与SHA-256。 |
| [续接核验](continuation-audit.json) · [源码快照](sources/iosapp/XBXRC/Platform/Streaming/StreamingMetalRenderer.swift) | 四份关键源码与最终验收哈希一致；原始成功结果包与归档测试日志核验通过。 |
| [cleanup.json](cleanup.json) · [artifact-manifest.json](artifact-manifest.json) | 本轮临时目录清理及归档指纹。 |

实现合同与耗时字段见[报告](../../../docs/reports/2026-11-20-ios-display-callback.md)。原始用户日志继续保留在Downloads与项目runtime-logs/ios；共享WebRTC/Rust缓存保留。

测试使用XBXRC scheme、`-parallel-testing-enabled NO -collect-test-diagnostics never`，选择`StreamingDisplayCallbackTests`、`TraceReliabilityTests`、`StreamingFrameFeedbackTests`及6项既有串流集成用例。临时构建和结果包归属`/private/tmp/xbxrc-ios-display-callback-20261120/r1`。

续接收尾已完成：相关构建、测试、导出和宿主进程均已结束，整个临时根目录及本任务系统临时目录已核验清理，回收978.29 MiB，清理后可用空间62.56 GiB。最终测试与失败诊断、设备构建摘要、四份源码快照及原始用户Trace继续保留。当前原始Trace包含36195行，六项新增计时字段的记录数为0；当前构建的真实帧率及六维健康继续按报告中的同场景运行要求验收。
