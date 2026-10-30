# 主机串流新日志验证

结论：主机直连启动、首帧和主动退出清理通过；空 ICE 快照后继续轮询的修复获得实机证据。播放阶段的 SR 提交供给仍低于视频呈现推进，自动回退视频路径后恢复到约 60fps。

## 范围与构建身份

- 日志：`/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793342364772-1.jsonl`。
- 北京时间 2026-10-30 14:39:24.772–14:42:08.693，2,269 行、163.921 秒；一次 home / macOS / webrtc-direct 启动。
- `traceProfile=dev`，`traceBudgetNotice=0`。构建记录的源码 SHA-256 为 `68f18dd5d500af8fb25601b90fca337679c808b10136287896fa96c94e30b1ab`；按 `src-tauri/build.rs` 算法重算当前工作区后完全一致。
- 浏览器行为 gate 与 runtime stability gate 均 PASS。该 gate 验证生命周期、观测和回退合同；SR 满帧性能另看下方帧供给。
- 当前播放器事实来自 browser 系列事件；后台 xbxengine 的 `priming/0fps` 是未装配原生播放器的空闲快照。通用原生 midsegment/ReferenceChain gate 对本样本缺少对应运行证据。

## 启动与退出时间线

| 北京时间 | seq | 事实 |
| --- | --- | --- |
| 14:39:48.749 | 66 | 开始解析启动上下文 |
| 14:39:55.974 | 90 | 开始等待主机会话 ready |
| 14:40:15.007 | 143 | ready 等待完成，耗时 19.033 秒 |
| 14:40:15.670 | 151 | 启动直连 runtime |
| 14:40:19.082 | 179 | SDP answer 返回 |
| 14:40:22.575 / 25.341 | 199 / 209 | 前两次 ICE GET 返回空，继续轮询 |
| 14:40:27.888 | 219 | 第三次返回 18 项候选数据 |
| 14:40:28.287 | 227 | transport connected，最终 UDP prflx → srflx |
| 14:40:28.811 | 238 / 239 | 首帧呈现；connected → firstPresented 约 526ms |
| 14:41:59.599 | 2233 | 菜单主动退出，开始清理 |
| 14:41:59.616 | 2240 / 2241 | runtime stop 完成，耗时 13ms |
| 14:42:01.264 | 2252 | 所有 execution cleanup 完成 |

请求启动到首帧 40.062 秒，其中 ready 等待 19.033 秒，runtime 启动到首帧 13.141 秒。首帧后播放约 90.788 秒。输入释放 3ms，远端清理约 1.659 秒；5 组 cleanup 均成功，clientClosed 后媒体元素、renderer、音频节点、回调、计时器、输入释放请求等资源计数归零。

本次 `executionFailed`、`browserRuntimeError`、`initialConnectionTimedOut`、连接恢复提交和 TURN fallback 均为 0。直连正常启动和关闭已验证；TURN、ICE restart、Peer 重建旧 SDP、原生恢复异常需要各自故障场景触发后验证。

## 网络、恢复与帧供给

- 357 个有候选对的网络样本：RTT P50 11ms、P95 15ms、最大 17ms；jitter P95 5ms、最大 11ms。
- 接收 111,177 包、累计丢失 43 包，累计丢包率约 0.0387%。最终视频为 2560×1440，收到和解码帧数均为 5,439，浏览器 `framesDropped=0`。
- PLI 累计 1、FIR 0、NACK 37、重传包 202。启动后关键帧成功解码，累计关键帧 6；逐 NACK/PLI 请求到结果的配对证据缺失，有效恢复率保持未量化。Rust `cleanAnchorCommitted/DisplayStable` 链在该 browser 样本中无对应证据。
- 一次 `freezeCount` 增量出现在 seq 532，累计冻结 212ms；相邻 seq 526 的 renderer submit age 为 207ms。该单次短停的具体起因仍缺少关联证据。

以下为去重的 `browserObservationConsumed`，按 renderer generation 分组，只取 renderer 覆盖满 3 秒的短窗；数值为这些窗口的均值，窗口相互重叠：

| renderer | 满覆盖样本 | received / decoded fps | video 呈现推进 fps | callback / renderer submit fps | video → submit 差值 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 首轮 SR（gen 1） | 4 | 59.87 / 60.02 | 59.67 | 44.66 / 44.66 | 15.01fps |
| 第一次 video 回退（gen 2） | 17 | 59.98 / 59.96 | 59.93 | 56.65 / 59.93 | 0 |
| SR 恢复试探（gen 3） | 1 | 59.95 / 60.06 | 59.39 | 45.29 / 45.29 | 14.10fps |
| 最终 video（gen 4） | 86 | 59.93 / 59.92 | 59.93 | 55.17 / 59.93 | 0 |

最终 video 窗口的 received → decoded 差值约 0.007fps、decoded → video 差值约 -0.002fps，支持稳定供给；SR 的主要缺口出现在 video → renderer submit 段。

91 个 render telemetry 窗口均为 `videoFrameCallback`。callback gap 累计 2 次，`presentedFrames` jump 累计 395 次；`droppedFramesSinceLastSample` 的合批计数也为 395，需按呈现推进解释。每窗最大 callback 间隔 P95 51ms、全局最大 216ms；90 个 submit-age 样本 P95 27ms、最大 207ms。最终 video 的 3 秒窗口平均 present/submit age 均值约 10.18ms。

`sourceFpsEstimate` 全程缺失，原因是 `mediaTimeDeltaTooSmall`，供给判断采用 RTP 帧计数与呈现推进。上述 age 为客户端观测；实际 GPU 提交到屏幕呈现和 glass-to-glass 长尾需要外部测量。

## SR 回退事实

- seq 384/387：持续帧交付不足，SR → video；seq 477 记录效果 `improved`。
- seq 775/778：健康窗口后试探恢复 SR。
- seq 862/863：约 4 秒后试探失败，回到 video；seq 959 记录效果 `improved`。
- `enhancementTrialFailed` 属于渲染增强试探回退，连接持续保持 connected。当前日志支持保留 video 回退，并继续定位 SR 的 callback/绘制供给缺口。

[结构化指标与日志摘要](2026-10-30-home-startup-fresh-trace.metrics.json)。下一轮优先验证 SR 满帧供给，并单独安排直连失败转 TURN 的故障场景。
