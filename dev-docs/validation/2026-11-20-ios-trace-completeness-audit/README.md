# iOS Trace 完善度审计

审计日期：2026-11-20。范围为当前工作区的 Swift Trace writer、串流埋点、Rust/Swift 调用边界和两个 Python 分析器。结论来自源码审计、17 项既有分析器测试及隔离故障注入；本轮运行时代码保持原样。

**现有 Trace 已覆盖应用与串流主流程。落盘可靠性、轻量记录、采样口径和性能门禁存在明确缺口，精确卡顿与恢复效果分析需要先补齐这些环节。** 日志交付沿现有设置页导出，目标目录为项目 `runtime-logs/ios`。

## 已具备的能力

- Schema v3、进程 sessionId、递增 seq、operationId，以及串流 attemptId/generation/peerEpoch。
- App 启动、认证恢复、Rust 认证调用边界、云库缓存/刷新/元数据、图片候选、UI 呈现和设置事件。
- 串流启动/SDP/ICE、四个 DataChannel/Handshake、首次收帧、Metal 首次呈现、playing、恢复动作/效果与退出清理。
- 每秒 RTC 健康与监督快照，Metal submit/present 累计量及最近 240 个样本的 P50/P95/P99，输入和震动累计摘要、音频路由/中断/glitch 事件。
- Debug 默认 dev（32 MiB × 6），Release 默认 production（8 MiB × 4）；40 ms 或 128 行刷新，敏感字段脱敏、标题指纹、完整 URL 过滤。
- 全部/当前 Trace 导出、最长 5 分钟/32 MiB 手动诊断录制、问题快照和保留策略。

静态盘点包含 21 个文件的 205 处直接 `IOSRuntimeTrace.*` 调用；串流 helper 动态发出的事件另有覆盖，该统计用于定位埋点。

## 六维证据覆盖

| 维度 | 当前证据 | 诊断边界与缺口 |
| --- | --- | --- |
| 网络健康 | RTC RTT/jitter、packet 累计、bitrate、candidate 类型、NACK/PLI/FIR | loss 的分子汇总音视频，received 分母只统计视频；videoPacketsLost 在运行时模型中存在，Trace 投影遗漏。缺统一样本时间与统计范围标记。 |
| 恢复链健康 | runtimeSupervisionSnapshot、动作开始/完成、recoveryActionEffectEvaluated | 可审计应用层恢复动作与结果；libwebrtc 接收侧 ReferenceChain/RecoveryLedger/cleanAnchorCommitted/DisplayStable 内部事实缺公开投影，需要明确 capability/unsupported 边界。 |
| media supply | 视频/音频 bytes、framesDecoded/dropped、首/末 media 时间、frameSupplyDelta | first/last media 时间来自每秒累计量采样；frameSupplyDelta 表示解码量变化。framesReceived 与 observedAtMilliseconds 已用于运行时监督，Trace 投影遗漏。 |
| steady supply | 首次正向 delta 的 steadyMediaObserved，监督中的 inbound/decode/present FPS | steadyMediaObserved 是一次性锚点；持续窗口长度、样本数量、时段覆盖和计数配平尚需分析器验证。 |
| submit/present 长尾 | submit/present count、P50/P95/P99、最近 240 个样本 | present 在 commandBuffer completion 转回 MainActor 后采样，指标包含回调处理调度。真实 drawable 显示时间、逐帧 ID/源时间和窗口边界需要补充。 |
| 帧供给 delta | 解码 delta、present 累计、displayedFrameDelta、overwrite/drop 累计 | displayedFrameDelta 在每次快照生成时重置；present 后的 busy 状态快照会将最新 delta 置为 null。可通过累计 present 差分补算，直接使用 delta 会误读。 |

## 优先缺口与源码证据

### F1 · P1：落盘失败静默丢失，打开失败缓冲持续增长

[IOSRuntimeTraceWriter.swift](../../../iosapp/XBXRC/Shared/Diagnostics/IOSRuntimeTraceWriter.swift#L550) 的目录创建、文件创建和打开失败均被吞掉，activeFileURL 仍被设置；每个后续事件继续尝试打开并追加 fileOpened。

[flushBuffer](../../../iosapp/XBXRC/Shared/Diagnostics/IOSRuntimeTraceWriter.swift#L587) 忽略 write/synchronize 错误后清空缓冲。隔离注入中，文件句柄关闭后的事件完全丢失，缓冲为 0，seq 仍推进到 2；普通文件作为 rootDirectory 时，5 个事件积压 11 行、10 个事件积压 21 行，报告的 activeFileURL 指向不存在的文件。

建议在现有 writer 内补充明确的打开/写入错误状态、失败计数和有界缓冲策略，使导出结果能说明证据完整性。

### F2 · P1：轻量模式过滤三类核心健康快照

[record](../../../iosapp/XBXRC/Shared/Diagnostics/IOSRuntimeTraceWriter.swift#L104) 过滤 importance 高于 essential 的所有 snapshot；[RTC/display](../../../iosapp/XBXRC/Platform/Streaming/StreamSessionActor.swift#L684) 和 [supervision](../../../iosapp/XBXRC/Platform/Streaming/StreamSessionActor.swift#L1040) 均为 key snapshot。

故障注入确认轻量后台仅留下 fileOpened/terminalSelected，三类健康快照全部被过滤；手动录制期间 rtcHealthSnapshot 恢复。轻量模式仍持续记录逐帧 key 状态事件，健康证据与事件量的取舍需要调整。

建议保留低频关键健康快照，对高频逐帧事件使用摘要或有界采样。

### F3 · P1：性能统计的样本口径需要统一

[makeSnapshotLocked](../../../iosapp/XBXRC/Platform/Streaming/StreamingMetalRenderer.swift#L403) 每次调用推进 lastReportedPresentCount；[handleCompletedCommandBuffer](../../../iosapp/XBXRC/Platform/Streaming/StreamingMetalRenderer.swift#L1077) 在 present 之后再生成 busy 快照。原始类型提取探针确认同一帧的 present 快照 delta=1，最新 busy 快照 delta=null，累计 presentCount=1。

[LibWebRtcRuntime](../../../iosapp/XBXRC/Platform/Streaming/LibWebRtcRuntime.swift#L1392) 将视频 packetsReceived 和全部 inbound-rtp packetsLost 投影到同一 Trace 快照；运行时监督已有单独视频 loss 和 framesReceived，Trace 尚需同步字段。源时间为 CACurrentMediaTime 单调时钟，envelope tsMs 为写入时 Unix 时间，缺统一的样本时间映射。

建议围绕固定统计窗口记录 received/decoded/presented 累计量和 delta，加入 sampledAt/窗口长度/样本数，并明确 GPU 完成与 drawable 呈现的测量来源。

### F4 · P1：核心 gate 能证明流程完整，性能与配对门禁深度有限

[check-streaming-core-trace.py](../../../iosapp/scripts/check-streaming-core-trace.py#L505) 要求有 submit/present、正向 display snapshot 和稳态锚点。[build_report](../../../iosapp/scripts/check-streaming-core-trace.py#L987) 汇总结构/schema/privacy/budget 违规。

合成夹具包含 RTT=1500 ms、jitter=500 ms、lost=99000/received=1000、frameSupplyDelta=0，以及 submit-to-present P95=1000/P99=2000 ms，核心 gate 仍 PASS。该结果说明当前 PASS 的证明范围是核心流程链。

基础分析器 [sequence_report](../../../.agents/skills/analyze-ios-runtime-trace/scripts/analyze_ios_runtime_trace.py#L271) 对 seq=1→10 返回零违规；[pairing_report](../../../.agents/skills/analyze-ios-runtime-trace/scripts/analyze_ios_runtime_trace.py#L388) 将事件转为集合，结果先出现、随后两个同 operation 的 start 仍返回零违规。signalingOffer/ICE 等边界也尚未进入基础 PAIR_RULES。

建议扩展既有脚本：报告完整输入范围内的缺号，校验 start/outcome 的数量与顺序，并形成六维连续窗口、累计量配平和 P95/P99 报告。性能阈值按设备、目标帧率与验收合同确定。

### F5 · P2：队列压力与源时间存在诊断盲区

[record](../../../iosapp/XBXRC/Shared/Diagnostics/IOSRuntimeTraceWriter.swift#L112) 的 4096 阈值只作用于 debug/raw；逐个丢弃仍派发 queue.async，丢弃任务本身位于 pendingRows 计数之外。关键队列注入达到 5000，全部 key 被接收。[recordDrop](../../../iosapp/XBXRC/Shared/Diagnostics/IOSRuntimeTraceWriter.swift#L597) 每 60 秒报告，flush/关闭阶段缺剩余计数报告；丢弃 50 条只报告了 1 条，49 条保留在内存计数中。

[encode](../../../iosapp/XBXRC/Shared/Diagnostics/IOSRuntimeTraceWriter.swift#L533) 在队列执行时生成 tsMs。人为暂停队列 180 ms，事件时间偏移约 194 ms。部分媒体 payload 使用单调源时间，普通边界事件的 envelope 时间仍受队列等待影响。

建议在事件入口记录源时间、合并丢弃计数任务、收尾输出最终 loss summary，并限制高频 key 的队列成本。实际开销需要完整会话测量。

### F6 · P2：自动问题留存与录制元信息需要补全

[saveIssueIfNeeded](../../../iosapp/XBXRC/Shared/Diagnostics/IOSRuntimeTraceWriter.swift#L365) 依赖事件名 Failed 后缀；[终态](../../../iosapp/XBXRC/Platform/Streaming/StreamSessionActor.swift#L2048) 使用 sessionTerminal/terminalSelected + reason=runtime.failed。探针确认失败终态生成 0 份问题记录，随后 Failed 后缀事件生成 1 份。失败终态仍写入常规 Trace。

手动录制文件包含录制期间事件，fileOpened 经独立路径写入常规日志，录制包缺预算/配置头；metadata 只有 endedAt，缺手动、时间上限、字节上限、I/O 错误等结束原因。建议用结构化终态触发问题快照，录制增加开始上下文和结束原因。

### F7 · P2：跨边界关联信息有限

[Rust 调用边界](../../../iosapp/XBXRC/Platform/Streaming/StreamingRuntime.swift#L842) 有独立 operationId 和 elapsedMs，signalingOffer/ICE 的 payload 缺 attemptId/generation/peerEpoch。Metal context 虽然持有 videoGeneration，[通用 Trace 投影](../../../iosapp/XBXRC/Platform/Streaming/StreamingMetalRenderer.swift#L108) 只输出 attemptId/generation/peerEpoch。

建议将现有父上下文投影到边界事件，并记录运行版本/build、运行目标和可观测能力，便于区分旧产物、恢复代次及 SDK 能力。Swift 日志可定位调用阶段，native 崩溃栈继续结合 .ips/LLDB 证据。

## 验证与真实会话限制

- 基础分析器 3 项、串流 core gate 14 项既有测试全部通过。
- Swift 6 / DEBUG 宿主 harness 直接编译当前 envelope/policy/writer；覆盖打开失败、关闭句柄写入、轻量过滤、手动录制、队列压力/尾部计数、时间偏移和失败终态留存。
- Metal telemetry 探针提取当前源码原始类型，通过 present→busy 的真实调用顺序验证 delta。
- Python 对抗夹具验证 core PASS 的性能边界、seq 缺号及反向/重复操作配对。夹具为合成数据。
- 前序 xHome 系统日志有 1180 条事件名，缺 payload/seq/sessionId；当前 xHome 的完整 JSONL 仍待导出，实际网络质量、卡顿比例及恢复效果继续依赖该证据。
- 本轮完整 App 构建和实机长测范围留给实现/实机验收；当前结果证明源码行为和隔离诊断缺口。

证据：`writer-probes.json`、`telemetry-probe.json`、`analyzer-probes.json`、`adverse-core-gate.json`、`validation.json`、源码哈希与复现 harness。清理记录见 `cleanup.json`。

建议先修 F1/F2/F3，再扩展 F4，继续通过“设置 → 高级诊断 → 导出全部 Trace”保存到 `runtime-logs/ios`。
