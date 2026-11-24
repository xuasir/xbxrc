# iOS 后台恢复与帧供给修复验收

2026-11-23。后台管线就绪补送、恢复验证与新上屏收口、底层轨道身份去重和按需显示调度已落地。最终 iPhone 17e / iOS 27.0 **45 项通过，0 失败**；Device SDK 无签名构建、串流 core 分析器 17 项、基础 Trace 分析器 3 项及源码门禁通过。实际帧率与呈现长尾由当前构建的新 Trace 验收，任务保持 `Validation Pending`。

## 本轮修复

- 管线就绪分别保存观测与送达状态。后台首帧补齐能力事实，前台恢复和晚接回报补送当前 context；每个前台区间送达一次，context 切换清空旧事实。
- 恢复先发布等待状态，会话验证和恢复后的新上屏共同决定 `playing`。验证期间的新帧保留，验证与 peer 快照异步返回核验 generation、peerEpoch、visibility revision；首帧守护按仍待新上屏的事实装配，重复可见性通知合并。
- 每秒 receiver 扫描与 SwiftUI surface 绑定共用底层轨道相等性。相同 native 轨道的新 SDK 包装对象保留首帧探针与 renderer 绑定；真正的新轨道和 videoGeneration 变化正常重装。
- 原 latest-only 入口每次 MainActor 处理最多接续两帧，CADisplayLink 按待处理帧启停。GPU 完成和新帧及时唤醒原 scheduler，失败重排等待下一显示 tick。

新增 `ingressQueueMs`、`frameAcceptWorkMs`、`drawToSubmitMs`、`drawableAcquireMs` 分别记录入口排队、接受处理、绘制准备和 drawable 获取。GPU 完成继续作为 rendered/playing 观测，系统 drawable 呈现独立记录。`displayPipelineReadyDelivered` 记录实际补送路径；`steadyMediaObserved.presentationActive` 为显式后台供给提供门禁证据。core 门保留前台、缺字段与异常值的原顺序检查，以及首显、playing、清理的完整门槛。

## 原日志定位

输入为 [旧版原始 Trace](../../../runtime-logs/ios/runtime-trace-ios-1795187951399-1.jsonl)，33877 行、两个 Home 尝试，原文件 SHA-256 保持一致。正常供给窗口解码 59.94FPS、GPU 完成 58.62FPS、系统 drawable 58.32FPS；解码到完成计数差约 2.19%。189 帧应用丢弃全部记为 `ingressCoalesced`，mailbox 覆盖为 0。

两个 peer 的 surface 绑定分别为 50 / 149 次，间隔中位约 1002ms；171 次入口丢弃事件中，68 次落在前一次绑定后 50ms 内。源码在 receiver getter 返回新包装对象时按对象地址判定新轨道，重复更换首帧探针并发送视频 handle；surface 同样按包装地址解绑和重绑。[原始锚点与统计](baseline-analysis.json)保留时序和前台窗口口径。实际 SDK 回归验证连续 60 次 receiver 读取产生不同包装对象，其底层轨道相等；同 trackId 的不同 native 轨道比较为不同。

## 验证证据

| 证据 | 结果 |
| --- | --- |
| [模拟器摘要](simulator-summary.json) · [用例日志](simulator-tests.log) | 45 通过、0 失败。12 项新增覆盖后台/晚接补送、恢复两种先后顺序、迟到验证/peer 快照、验证失败、入口有界接续、SDK 包装身份与字段脱敏。 |
| [设备构建](device-build-summary.log) | 最终 iPhoneOS SDK 无签名构建通过，包含真机 Metal 分支。 |
| [core 分析器](core-analyzer-tests.log) · [基础分析器](base-analyzer-tests.log) | 17 + 3 项通过。显式后台供给允许先于首显；前台、缺字段、字符串/整数假值和缺首显继续触发原门禁。 |
| [会话边界](session-boundary.log) · [解析/diff](parse-and-diff.log) | 公共 Rust 会话边界、五份 Swift parse、两份 Python AST 和 diff 检查通过。 |
| [格式配置](swift-format.json) · [范围](format-ranges.json) · [Actor 格式](actor-format.log) | 四空格；renderer/tests 全文件检查通过，Actor/LibWebRTC/Player 本轮修改范围幂等。 |
| [源码指纹](source-hashes.json) · [源码差异](task-changes.diff) · [诊断](diagnostics.json) | 七份最终源码与快照逐字节一致；首轮脚本断言范围错误、格式校正和结果包导出诊断归档，最终门禁通过。 |
| [总验收](validation.json) · [清理](cleanup.json) · [归档指纹](artifact-manifest.json) | 自动验收结果、临时产物清单及空间核验。 |

## 新运行验收

固定原 1440p 设置、场景与窗口，连续前台至少 60 秒，并覆盖后台启动及多次前后台恢复。核对 `playing`、触控/菜单和退出；在稳定 peer、固定 videoGeneration 的片段核对首次绑定和明确重装次数，观察入口合并丢弃与入口排队。

| 维度 | 同窗口核对项 |
| --- | --- |
| 网络健康 | RTC RTT、jitter、包丢失与候选对；原日志 RTT P95 9ms、丢包 0。 |
| 恢复链健康 | 标准 PLI/NACK/FIR、恢复状态与 keyframe 动作；ReferenceChain/RecoveryLedger 等 SDK 内部事实继续按已有能力边界报告。 |
| media supply | 接收/解码 counter delta、帧尺寸、码率和供给停止。 |
| steady supply | 连续有效前台窗口、freeze、零供给段及恢复后的新帧。 |
| submit/present 长尾 | 原 GPU/系统 drawable 独立分位数，新增入口排队、drawable 获取与绘制准备分段。 |
| 帧供给 delta | 对齐 sampleId 和可见性窗口的 decode/submit/GPU complete/drawable 计数与实际 sampleWindowMs；100ms 健康合并的采样边界纳入解释。 |

最终截图、长测与新 Trace 由用户按实际串流场景验收。原始用户日志、前轮证据、共享 WebRTC/Rust 缓存继续保留；本轮构建、结果包和一次性日志按 [清理记录](cleanup.json)收尾。

本轮构建、测试、导出和三个测试宿主均已结束，专用临时根目录已清理；移除分配量857.72MiB，观察到可用空间增长814.87MiB，清理后可用空间55.62GiB。源码、测试/诊断、原始用户Trace和共享缓存按清单保留。

后续收到[新运行Trace](../2026-11-23-ios-display-lifecycle-fresh-trace/README.md)：26163行、串流core PASS，后台就绪在前台恢复后3.087ms补送、每peer一次surface绑定；95.24秒对齐前台解码59.94FPS、GPU完成52.52FPS、drawable52.48FPS。入口合并730帧与MainActor等待最高663.881ms继续定位，播放期间多次恢复继续覆盖；任务保持Validation Pending。
