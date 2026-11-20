# iOS Trace 四项补齐验收

日期：2026-11-20。范围：落盘可靠性、轻量健康快照、性能统计口径、队列压力与源时间。关联[Report](../../../docs/reports/2026-11-20-ios-trace-reliability.md)及[RFC](../../../docs/rfcs/2026-11-20-ios-trace-reliability.md)。

## 验证结果

| 证据 | 结果与范围 |
| --- | --- |
| [simulator-summary.json](simulator-summary.json)、[simulator-tests.log](simulator-tests.log) | iPhone 18 Pro / iOS 27.0，11项新增XCTest通过，0失败；包含配置关闭前刷出证据与尾部摘要。 |
| [host-tests.log](host-tests.log) | Swift 6 / DEBUG，11项新增与9项原有Trace回归共20项通过。 |
| [device-build.log](device-build.log) | iPhoneOS目标，禁用签名构建通过；包含设备drawable呈现回调代码。 |
| [core-gate-tests.log](core-gate-tests.log)、[base-analyzer-tests.log](base-analyzer-tests.log) | 14项串流core gate与3项基础分析器既有回归通过。 |
| [format-lint.log](format-lint.log)、[session-boundary.log](session-boundary.log) | Diagnostics格式及会话边界检查通过；空格式日志表示零诊断。 |
| [validation.json](validation.json) | 验收范围、结果、开工差分及并发改动归属。 |
| [baseline-source.json](baseline-source.json)、[final-source.json](final-source.json) | 开工与收口源码哈希；同时记录相关合同及共享工作区测试上下文。 |
| [resolved-build-diagnostic.json](resolved-build-diagnostic.json) | 一次并发测试文件缺显式self的编译诊断；当前源码已有修复，最终重跑通过。 |
| [cleanup.json](cleanup.json) | 本轮临时构建与测试产物清理完成，删除逻辑大小1.022GiB。 |

## 回归范围

新增用例覆盖存储打开/写入故障、恢复与明确导出错误、硬队列/预留/精确配平、剩余丢弃摘要、配置关闭顺序、源时间/队列延迟、超大行、轻量快照/逐帧采样/手动录制、视频loss范围及缺失、窗口reset/零供给、GPU与drawable独立延迟窗口。

[host-harness](host-harness/Package.swift)包含已执行的诊断源码、媒体原始类型及测试。诊断源码与收口版本逐字一致，新增测试块仅有换行格式差异；在macOS通过`swift test --package-path dev-docs/validation/2026-11-20-ios-trace-reliability/host-harness --scratch-path <任务临时目录>`可复现宿主回归。模拟器使用`XBXRC` scheme及`-only-testing:XBXRCTests/TraceReliabilityTests`。

## 真实会话边界

本轮结果证明采集行为和代码装配。设备目标的系统drawable呈现仍需真实xHome数据确认；网络、恢复、media supply、steady supply、submit/present长尾和帧供给delta沿完整JSONL核验。

前序“My Mac (Designed for iPad)”正常运行的系统日志继续保留在`/private/tmp/xbxrc-xhome-mac-trace-analysis-20261120/r1/system-log.json`；新版本Trace沿现有设置导出到项目`runtime-logs/ios`。共享WebRTC缓存与其他任务临时数据保留。
