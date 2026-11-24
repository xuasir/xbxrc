# iOS MainActor 排队与串流优先级续接

本轮沿原latest-only入口和Metal显示链完成实施。52项模拟器回归和Device无签名构建通过；源码、格式、解析、会话边界、lint及暂存指纹核验通过。新Trace的730帧入口合并及663.881ms主线程等待作为复验基线，实际帧率收益继续由当前构建的新运行Trace验收。

范围：显式串流任务优先级、drain ticket与排队计时、触屏观察范围。原会话/信令、libwebrtc、MainActor/UIKit所有权及GPU释放和恢复合同保持。

串流帧入口、尺寸就绪、GPU完成和drawable回调使用userInitiated；100ms周期健康快照使用utility，首帧与生命周期事件及时送达。原slot以唯一ticket隔离reset后的迟到/重复任务，每次最多接续两帧。接受和合并丢弃携带ingressDrainQueueMs及ingressDrainWorkBeforeFrameMs，保留最新帧年龄和接受工作计时。触屏覆盖层仅订阅去重后的抑制状态，触点沿原UIKit绘制和释放。

| 证据 | 结果 |
| --- | --- |
| [执行输入](task-input.json) · [实施摘要](implementation-summary.json) | 原始源码/文档基线、授权范围与优先级分配。 |
| [原始测试摘要](simulator-summary.json) · [测试树](simulator-tests.json) · [52项逐用例](verified-test-cases.json) | 52通过、0失败，新增4项，原始结果逐项核验。 |
| [模拟器日志](simulator.log.gz) · [执行](simulator-execution.json) | 最终成功日志压缩后逐字节核验，六份源码与执行指纹一致。 |
| [设备摘要](device-build-summary.json) · [日志](device.log.gz) · [执行](device-execution.json) | iPhoneOS无签名BUILD SUCCEEDED、errorCount=0。 |
| [首轮编译诊断](simulator-attempt1-diagnostic.json) · [次轮诊断](simulator-attempt2-diagnostic.json) | MainActor隔离、宿主SwiftUI导入与只读环境赋值均修复；失败日志及构建摘要保留。 |
| [运行诊断对照](runtime-warning-comparison.json) | 帧反馈测试既有QoS等待警告与前轮同源同消息。 |
| [格式范围](format-ranges.json) · [静态检查](static-checks.json) · [lint](frontend-lint-fix.log) | 六份源码格式幂等/解析、会话边界、diff及前端lint通过。 |
| [源码指纹](source-hashes.json) · [变更](task-changes.diff) · [源码快照](sources/iosapp/XBXRC/Platform/Streaming/StreamingMetalRenderer.swift) | 六份验收快照匹配两次执行，五份保留源码与工作区一致；本轮保留变更相对各文件基线。 |
| [并行编辑核验](concurrent-edit-audit.json) · [构建/测试计划](build-and-test-plan.json) | 原Git index保持，其他文件字节保持，任务专用临时与结果包路径完整记录。 |
| [验收汇总](validation.json) · [进程核验](process-audit.json) · [清理](cleanup.json) | 代码验收通过，真实串流Validation Pending；专用临时目录已清理865.59MiB。 |

相关进程均已结束，四个结果包、两套DerivedData、SystemTemporary、重复日志、脚本和基线已随专用根目录清理。清理后可用空间46.54GiB；六份源码快照、成功/失败日志与原始结果摘要、用户Trace和共享缓存保留。

当前构建固定原1440p场景运行至少60秒，并覆盖后台启动和多次恢复。对照基线95.24秒窗口的解码59.94、GPU52.52、drawable52.48FPS，复核网络、恢复、media supply、steady supply、submit/present长尾及帧供给delta；同时检查新drain排队字段、gpuToMainActorMs与入口合并数量。优先级为调度提示，实际长等待占用栈以采样证据确认。

[并行探针收尾核验](post-validation-concurrent-cleanup.json)确认文件末尾的临时UI截图探针已移除，既有测试前缀逐字节保持、所选52项用例保持、当前文件Swift解析通过。当时完整验收快照继续归档，任务保留变更为五份源码。
