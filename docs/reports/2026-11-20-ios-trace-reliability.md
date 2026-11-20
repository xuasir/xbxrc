# iOS Trace 可靠性与性能口径补齐 Report

日期：2026-11-20。状态：Done。相关[RFC](../rfcs/2026-11-20-ios-trace-reliability.md)及[验收证据](../../dev-docs/validation/2026-11-20-ios-trace-reliability/README.md)。

## 交付结果

现有Swift Trace补齐落盘可靠性、轻量健康快照、性能统计口径、队列压力与源时间四项能力。日志沿“设置 → 高级诊断 → 导出全部 Trace”保存到项目`runtime-logs/ios`。传输、会话及媒体调度沿现有运行时执行；分析门禁深化保留为后续项。

| 项目 | 最终行为 |
| --- | --- |
| 落盘可靠性 | 打开、写入、同步故障记录阶段、错误码及累计数；失效句柄关闭，事件驱动重开限频为1秒。写入故障尝试回退半行并统计未写尾部，同步故障单独记录持久性不确定行。恢复输出`traceStorageRecovered`与文件头累计摘要；持续故障时导出返回明确错误。 |
| 轻量记录 | 保留essential/key健康快照，包括RTC、display与supervision。指定逐帧事件保留首次及每60次采样；手动录制沿原详细记录策略。 |
| 性能口径 | 视频received/lost使用同一统计范围，缺失的观测值保留null。RTC记录received/decoded、源样本时间与相邻窗口delta；display定期Trace以累计submit/present/drawable差分，零供给记录0，首窗、代次变化与计数回退记录null。 |
| 压力与源时间 | pending硬上限4096、debug/raw入口上限2048、key上限3968，为essential预留128。丢弃在入口合并计数并限制通知任务；flush、导出、关闭及配置切换刷出剩余摘要。单行上限64KiB、编码缓冲上限1MiB；envelope使用事件源Unix时间，payload分别记录单调源时间和队列等待。 |

GPU完成时间在commandBuffer完成回调入口采样，MainActor处理等待单独记录。系统drawable呈现通过`presentedTime`独立观测，两类延迟分别保留最近240个样本及P50/P95/P99、窗口边界和样本数。模拟器使用GPU完成观测；设备目标包含drawable呈现观测代码，实际显示数据由真实会话验收。

## 主要改动

- `IOSRuntimeTraceEnvelope/Policy/Writer`承接源时间、轻量采样、队列预算、故障状态与摘要；配置切换按IO队列顺序先刷出已接收证据。
- `LibWebRtcRuntime`聚合视频统计；`StreamSessionActor`在既有代次边界重置Trace窗口并输出关联样本。
- `StreamingMetalRenderer`分别采样GPU完成与drawable呈现，补齐帧ID、源时间和测量来源；percentile缓存供重复快照复用。
- `TraceReliabilityTests`新增11项回归；原rotation用例调整为1800字节预算并验证实际sample事件保留。

## 验证

| 验证 | 结果 |
| --- | --- |
| iPhone 18 Pro / iOS 27.0模拟器定向XCTest | 11通过，0失败 |
| Swift 6 / DEBUG宿主harness | 新增11项与原有9项，共20通过，0失败 |
| iPhoneOS设备目标构建 | `CODE_SIGNING_ALLOWED=NO`构建通过 |
| 既有Python串流core gate与基础分析器 | 14项与3项通过 |
| Diagnostics格式、会话边界及任务源码diff检查 | 通过 |

故障注入覆盖不可用目录、关闭句柄、恢复、导出错误、超大行、硬队列与关键锚点预留、各等级丢弃配平、60秒通知间隔内尾部摘要、配置关闭、源时间与队列延迟。统计用例覆盖视频范围、多轨缺失、零供给、代次/计数重置、GPU与drawable独立窗口。

宿主harness逐字复制最终诊断源码，提取媒体原始类型；新增测试块与当前源码仅有换行格式差异。共享工作区其他测试修改保留；一次并发测试文件缺显式self的编译诊断已归档，当前源码修复后定向重跑通过。

## 收尾与验收边界

最终日志、机器摘要、源码哈希与可复现harness已归档。确认本轮构建、测试和导出进程结束后，清理`/private/tmp/xbxrc-ios-trace-reliability-20261120/r1`及空任务父目录，删除逻辑大小1.022GiB，磁盘可用增加0.763GiB；共享WebRTC资源、前序xHome系统日志及其他任务临时目录保留。[清理记录](../../dev-docs/validation/2026-11-20-ios-trace-reliability/cleanup.json)。

自动验证证明采集行为、统计投影与编译有效。实际网络健康、恢复效果、稳态供帧和呈现长尾需要新版本真实会话的完整JSONL。建议在“My Mac (Designed for iPad)”再运行一次xHome，导出全部Trace到`runtime-logs/ios`核验六维证据。
