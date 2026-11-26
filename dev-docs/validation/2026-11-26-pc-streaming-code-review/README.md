# PC 串流当前源码优化评估

- 日期：2026-11-26（Asia/Shanghai）
- 检出分支：`release/test`
- HEAD：`a98e5457c8f9d67b099033710aa34d08d2fcd3cb`
- 范围：当前 Browser PC 装配，包括 Vue/TypeScript 播放与输入、WebRTC、Rust 公共监督决策。
- 依据：当前源码和本轮针对性验证。
- 操作：源码只读评估，产出本目录证据。

## 已确认的当前实现

- Browser GamepadBackend 使用 rAF 与 **4ms** 定时补采，同态心跳为 100ms；输入变化即时进入发送，纯摇杆积压合并最新值，数字边沿与释放保留顺序。
- 解码 TrackProcessor 缓冲为 1；Worker 保留单帧在途及最新候选，旧候选及时关闭。
- 常规 60fps 准入即时追逐新帧；像素读回诊断按显式请求开启。
- 当前 PC 网络与恢复权威决策位于 `crates/xbox-streaming/src/session/recovery/`，由 `browser-runtime.ts` 调用 Rust RPC。

## 优化入口

| 优先顺序 | 源码事实 | 建议方向 | 收益验证 |
| --- | --- | --- | --- |
| 1. 渲染热路径 | `browser-render-policy.ts:124` 的 auto 原画选择 WebGL；`Renderers.ts:760` 对新帧执行 `texImage2D`，呈现尺寸包含显示像素预算。 | 对既有 video 与 WebGL 做帧龄/成本 A/B；评估稳定尺寸下纹理复用及 `texSubImage2D` 的实际性能，约束输出像素预算并保留源尺寸下限。 | 上传与绘制 P95/P99、主线程输入采样间隔、GPU 完成上下界、稳定帧供给。 |
| 2. 网络排队延迟 | `supervision.rs:159` 以丢包率作为拥塞入口；丢包率 <3% 时，较大 RTT、jitter 与 jitter buffer 增长仍归为 stable。`quality.rs:161` 的 stable 路径会更新 RTT 基线。 | 在现有 Rust 策略中加入持续排队延迟的资格和效果判断，保留路径代际与证据窗口；按远端已验证能力调整质量。接收端初始化可另行评估 `jitterBufferTarget` 能力与播放预算。 | 同路径 RTT 相对低位基线、jitter buffer 延迟、帧龄下降，入流与解码供给保持。 |
| 3. 本地停顿恢复 | 稳态 60fps 停滞门槛为 1500ms，`browser-runtime.ts:83` 的入流宽限为 8000ms；`browser-runtime.ts:2041` 在执行局部恢复之前应用宽限。监督间隔为 1000ms；解码资格使用 3000ms 窗口和持续证据。 | 给已确认的“入流继续、解码/画面停滞”设置专用短宽限；根据故障域选择关键帧请求或播放重挂，按动作阶段确认持续新帧。 | 停顿到首次恢复新帧时间、恢复后连续帧、动作次数及误触发。 |
| 4. 视频反馈调度 | `InputService.ts:18` 的重试周期为 50ms；`addProcessedFrame` 常规入队，水位 >30 才主动 flush；输入变化、排空事件也会顺带提交反馈。 | 增加帧反馈最老年龄预算，用短批次合并并继续给输入边沿优先权。手柄状态变化继续即时发送。 | metadata 入队到发送的 P95/P99、发送包率、输入队列等待与数据通道缓冲年龄。 |
| 5. 音频健康与恢复 | `PlaybackService.ts:144` 在创建 audio 后报告 audioReady；`StatsService.ts:566` 投影 video、transport 和 candidate pair；音量增强开启后 `AudioEffectsService.ts:23` 创建 AudioContext 并在 attach 时 resume。 | 用 playing/实际播放进展确认音频就绪，补充音频丢包、concealedSamples、jitter buffer 和音频上下文状态；为前后台和输出设备切换建立有界恢复。 | 音频连续性、补偿样本比例、音频延迟、音画同步及切换后恢复。 |

## 验收口径

本轮结果证明当前代码行为与既有测试冒烟状态。优化收益需由实机短测量化；原生 video 与直接解码 WebGL 的播放等待、上传和合成成本应按设备比较。

后续优化使用同窗指标覆盖：

- 网络健康：选中路径、RTT、jitter、丢包、NACK/PLI 和 jitter buffer 延迟。
- 恢复链健康：关键帧与新帧恢复、恢复后持续供帧、动作资格和效果。
- media supply：接收与解码数量、速率和断供区间。
- steady supply：稳态绘制间隔与长尾，按实际内容帧率比较。
- submit/present 长尾：read/receive → submit、GPU 完成上下界；当前 fence 表示 GPU 完成代理，实际屏幕呈现通过平台取证或外部短测确认。
- 帧供给 delta：received → decoded → delivered → submitted 的同窗增量及本地丢帧。

## 本轮最小验证

- 单次运行 4 个既有 Vitest 文件，**58/58 通过**：输入服务 22 项、帧调度 23 项、渲染计划 10 项、音量增强 3 项。
- 从当前 Rust 文件直接提取 `classify_runtime_network_pressure`，使用独立临时二进制验证 3 个边界：RTT 基线 30ms、当前 180ms、jitter 60ms、jitter buffer 从 10ms 增至 200ms；0% 与 2.9% 丢包输出 stable，4% 丢包输出 congested。3 个断言均通过。
- 独立核验包装器产生了一条类型可见性编译告警，程序成功结束；告警来自临时包装器的本地 struct 可见性。
- 测试摘要见 `smoke-summary.json`；分类结果见 `network-pressure-results.json`。

## 临时产物收尾

- 本轮临时目录：`/private/tmp/xbxrc-pc-stream-code-review-20261126-t5lc8oq6`。
- 构建与测试进程结束后，归档必要 JSON 摘要并清理该目录中的临时 Rust 源、二进制和完整测试报告。
- 保留项：本目录 README 与 2 份 JSON 摘要。
- 清理结果：目标目录已删除；本轮移除临时文件 467702 字节，清理后可用磁盘空间 90931200000 字节。
