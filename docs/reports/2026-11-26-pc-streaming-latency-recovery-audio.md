# PC 串流第 2–5 项及协商声道修复 Report

## Summary

- 日期：2026-11-26（Asia/Shanghai）。
- 关联 [RFC](../rfcs/2026-11-26-pc-streaming-latency-recovery-audio.md)。
- 用户确认的 M1–M5 代码实施与自动冒烟已完成，当前账本状态为 Validation Pending，接续实机短测。
- 主窗口单窗口执行。公共网络/恢复判定由 Rust 承担，浏览器音频与媒体动作沿现有 PlayerClient/MediaService 执行。

## Delivered

| 问题 | 完成的行为 |
| --- | --- |
| 2：低丢包延迟累积 | RTT 与缓冲按各自覆盖判定；RTT 超过健康基线预算进入 warning，同时出现持续缓冲增长进入 congested。健康基线向更低 RTT 收敛，延迟动作按 RTT、缓冲、丢包和帧供给共同评估，效果不足复用既有回退。 |
| 3：局部停顿恢复 | 同代次、当前样本和持续入流构成局部短窗口；解码停顿请求关键帧，显示提交停顿复用 renderer/playback 重建。效果观察完成后允许一次有证据的升级；同级冷却、恢复预算与公共连接仲裁继续约束执行。 |
| 4：视频帧反馈 | 增加 8ms 合批、16ms 调度年龄预算、100ms 反馈保留上限及发送年龄/过龄/水位丢弃计数；输入变化即时发送，受阻后通过可写通知和既有重试接续。 |
| 5：声道消费与音频健康 | Rust 声道偏好贯通到播放格式；按有效 Opus 段幂等修改 stereo 参数，消费轨道设置与 sender hint，显式配置音频图并记录输出限制。实际 play/context 状态决定 audioReady；同一次 getStats 采集音频缓冲、样本、concealment、补偿与可比较的 A/V 估计。前台、设备、轨道和 context 事件触发有界恢复。 |

## Changes

- 声道协议逻辑集中在现有 SdpManipulator 的音频辅助模块；requested/codec/source/effective/output 分别取证。Mono 下混与增益共用 AudioEffectsService，图失败进入有原因的直接播放回退。
- SessionService 在答案和轨道到达后更新当前格式，轨道选择与统计来源对齐；增益开关保留当前协商声道。
- AudioEffectsService 统一管理 play/resume、事件监听、两秒操作期限、旧代次撤销及单次图重建；直接播放与图路径分别确认就绪，音量 0 保持明确的静音语义。
- Rust local_stall 辅助模块比较相邻累计样本，接受监督节拍抖动并要求至少一秒连续证据；可见性资格更新、采样过期、计数回退和资源代次变化重置局部证据。
- Browser 的恢复 probe 增加 renderer scope，按新渲染器提交代次评估重建；稀疏 video callback 与持续 canvas 提交分别观测。
- 生产 trace 增加音频状态和局部停顿事件，并将音频样本、实际声道及反馈发送年龄纳入字段白名单与既有健康桶。

## Validation

- **193 项前端定向用例最终通过**，按最后一次实际执行结果去重，覆盖 18 个测试文件。
- **23 项 Rust 监督用例最终通过**，覆盖网络压力、延迟效果/回退、局部恢复、预算、代次和可见性资格。
- **15 项实际生产投影断言通过**：音频、声道、局部恢复和反馈数据保留；示例健康行 3034B/4096B，音频状态 688B、局部停顿 579B、反馈 589B，均在各自 1024B 预算内。
- cargo fmt、pnpm lint:fix、pnpm typecheck、cargo check --offline -p xbxrc --lib 通过。Tauri 编译保留数据模块三条既有 dead_code 提示。
- 首轮失败及修正范围见[失败摘要](../../dev-docs/validation/2026-11-26-pc-streaming-latency-recovery-audio/failures-summary.json)；完整去重结果、源码指纹和清理记录见[验收证据](../../dev-docs/validation/2026-11-26-pc-streaming-latency-recovery-audio/README.md)。
- 本轮验证范围为代码、确定性媒体/网络模拟与生产序列化；实机收益接续同场景短测。

## Risks

- RTT 单项增长发布警告；自动质量动作需要持续证据、远端接受验证和效果检查。设备/路径差异通过实机数据校准。
- 声道观测来自协商、轨道设置和音频图；物理设备的左右分离与回退听感由实机确认。
- Browser 的 renderer 提交、GPU 完成边界、预计呈现与物理上屏分别取证；缺少 A/V 估计字段时保持 unavailable。

## Follow-up

| 验收面 | 实机短测重点 |
| --- | --- |
| 网络健康 | 同场景 RTT/jitter/丢包、缓冲趋势、请求/接受码率和延迟动作效果。 |
| 恢复链健康 | 局部动作到新解码/提交的耗时、连续恢复结果、预算消耗及连接仲裁。 |
| media supply | 同窗接收、解码与交付进展，确认样本与资源代次一致。 |
| steady supply | 30/60fps 源、稳定帧间隔、短缺口和恢复后的持续供给。 |
| submit/present 长尾 | read/receive→submit P95/P99、GPU 完成边界与实测 submit→屏幕呈现。 |
| 帧供给 delta | received→decoded→delivered→submitted 同窗增量，结合丢弃与回调合批解释。 |
| 音频输出 | Home/Cloud 左右声道、Mono 下混、音量/增益切换、前后台与输出设备切换。 |

建议使用同一源分辨率、显示尺寸、增强设置和网络做一次 Home/Cloud 短测，将新 trace 与听感结果归入本任务证据。
