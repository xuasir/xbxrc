# PC 串流网络延迟、停顿恢复、反馈和音频修复 RFC

## Status

- Completion: 已完成（代码实施与自动冒烟）
- Current State: completed
- Owner: 主窗口，单窗口直接执行
- Last Updated: 2026-11-26（Asia/Shanghai）
- 来源：关联源码评估的第 2、3、4、5 项，以及用户补充的“音频播放消费协商声道”。
- 当前产出：M1–M5 代码与自动冒烟完成；实机短测进入 Validation Pending，见 [Report](../reports/2026-11-26-pc-streaming-latency-recovery-audio.md)。

## Background

本轮以当前 main 的源码核对问题。关联评估用于确定问题编号；修复依据采用以下现行实现。

| 编号 | 当前行为与缺口 | 源码 |
| --- | --- | --- |
| 2 | 网络压力入口要求丢包覆盖；丢包低于 3% 时，RTT 持续增长仍归为 stable。质量动作的效果主要依据丢包下降，因此延迟改善也需要独立的评估口径。 | [supervision.rs](../../crates/xbox-streaming/src/session/recovery/supervision.rs)、[quality.rs](../../crates/xbox-streaming/src/session/recovery/quality.rs) |
| 3 | 稳态 30/60fps 停滞门槛下限 1.5s；Browser 对入流持续的停顿再加 8s 宽限。解码资格还要求 3s 覆盖、持续压力与 15s 动作冷却，叠加后局部恢复较慢。 | [browser-runtime.ts](../../src/streaming/runtime/browser-runtime.ts)、[browser-supervision.ts](../../src/streaming/runtime/browser-supervision.ts)、supervision.rs |
| 4 | 手柄变化立即 flush；常规帧反馈等 50ms 重试，队列超过 30 条才主动 flush。反馈独立于输入变化的发送时限需要补齐。 | [InputService.ts](../../src/player/app/input/InputService.ts)、[frame-feedback.ts](../../src/player/app/input/frame-feedback.ts) |
| 5a | Rust 将 Auto/Stereo 编译为 stereo_audio，并投影为 forceMonoAudio；Browser 仅向 transport 传递该字段。AudioRuntimeConfig 只有 volume/enableAudioControl，音频图使用默认声道设置；SDP stereo 参数依赖一次字符串替换。 | [compiler.rs](../../crates/xbox-streaming/src/policy/negotiation/compiler.rs)、[projection.rs](../../crates/xbox-streaming/src/policy/projection.rs)、[SessionService.ts](../../src/player/app/session/SessionService.ts)、[media.ts](../../src/player/domain/media.ts)、[AudioEffectsService.ts](../../src/player/app/media/AudioEffectsService.ts) |
| 5b | audioReady 在元素创建后立即发出；实际 play、AudioContext 状态和音频入流样本需要形成闭环。StatsService 当前按视频入流选择统计。 | [PlaybackService.ts](../../src/player/app/media/PlaybackService.ts)、[MediaService.ts](../../src/player/app/media/MediaService.ts)、[StatsService.ts](../../src/player/app/stats/StatsService.ts) |

## Goal

1. 识别低丢包下的持续排队，并依据实际延迟效果决定质量动作和回退。
2. 已建立播放的局部停顿使用短证据窗口，动作归属与连接恢复保持清楚。
3. 可写通道上的帧反馈及时发送，手柄边沿和释放继续优先。
4. 协商声道传到播放端；实际输出、就绪和恢复结果能通过同代次观测核对。

## Scope

- 交付主线：main 的 Browser PC；Vue/TypeScript 承担浏览器 API 与播放执行，Rust 公共模块承担网络和恢复判定。
- 复用现有 WebRTC、协商队列、输入通道、MediaService 和 AudioEffectsService；TWCC/GCC、NACK、PLI/FIR、ICE/TURN 沿现有标准路径。
- Rust/TypeScript DTO 的新增事实采用可选字段及明确的缺失语义；公共判定按覆盖和能力资格消费。
- 单窗口按阶段推进。运行时测试和真实收益分别记录；实机启动、串流与听感由用户手动验收。

## Plan

### M1 / 5a：协商声道进入播放配置

1. 在 AudioRuntimeConfig 增加 requestedChannelCount（1 或 2），默认 2；由同一 RuntimeLaunchSpec 的 forceMonoAudio 映射，贯通 BrowserPlayerClient → MediaService → AudioEffectsService。运行时配置更新、音频轨道替换和重连都按当前代次同步。
2. 将声道 SDP 修改收进现有 SdpManipulator：按有效音频媒体段、方向和 Opus payload 修改接收偏好；Mono 设置 stereo=0，Stereo 设置 stereo=1。保留其他 fmtp 参数，并支持参数缺失、重排及重复协商的幂等更新。
3. SessionService 在远端描述成功和接收轨道到达后，发布同代次 AudioPlaybackFormat 并更新 MediaService 的实际格式；答案/轨道先后到达都重算一次。格式分别保存 requestedChannelCount、codecChannelCount、sourceChannelCount、effectiveChannelCount、outputChannelCount 及其来源/能力，绑定到有效接收音频的 mid/PT/track。Opus 的 opus/48000/2 表示编解码器参数，源样本声道按轨道可用事实另行记录。
4. 播放消费 effectiveChannelCount：Mono 策略执行 1 声道下混；Stereo 按已确认源声道使用 1/2 声道，缺少源证据时沿请求模式回退并标记来源。在既有音频图内使用明确的声道数、explicit 模式与 speakers 解释，destination 输出受 maxChannelCount 限制；Mono 的本地下混复用现有图，直接播放沿浏览器已协商轨道输出。重协商、增益切换和图重建同步当前格式。
5. AudioContext 启动失败沿现有直接播放回退，并记录声道执行能力与回退原因。音量 0、增益设置和麦克风的单声道采集分别按各自合同执行。

最小验收：Mono/Stereo、fmtp 缺失与重复协商、轨道/答案先后到达、增益开关、输出能力限制、失败回退、旧代次完成回调各覆盖一次。真实左右声道分离与 Mono 下混采用实机听感验收。

### M2 / 4：帧反馈短批次与年龄预算

1. 给队列条目记录本地 enqueuedAtMs；wire 时间戳继续由 normalizeFrameFeedback 规范化。
2. 采用一个可取消的一次性定时器，首条反馈入队后 8ms 合批；在通道可写且主线程可调度的条件下，调度年龄预算为 16ms。输入事件仍立即 flush，并可顺带发送已排队反馈。
3. 复用当前 29 条/包、58 条队列、浏览器字节/年龄预算、释放余量和 onWritable。受阻后交由可写通知及现有 50ms 重试；本地反馈保留年龄上限 100ms，过龄条目按最新窗口策略释放并计数。
4. 队列排空取消反馈定时器；stop/start/通道替换取消旧代次任务。增加反馈 queue→send 的 P95/P99、最大年龄、过龄丢弃数及受阻原因，按既有低频 trace 节拍发布。

最小验收：静止手柄下的 30/60fps 反馈发送、短批次合并、输入释放优先、背压解除、100ms 过龄处理、停止与重启。16ms 预算同时报告真实调度晚到和通道受阻，实机收益以实际 queue→send 分位数衡量。

### M3 / 2：低丢包排队识别与质量效果闭环

1. Rust 网络判定保留 loss >= 8% 和 loss >= 3% 的既有分支；RTT、jitter buffer 按各自连续覆盖独立消费。覆盖不足和字段缺失给出 unknown/证据不足原因。
2. 使用同一路径的 3s 新鲜窗口和健康 RTT 基线。RTT 增量超过 max(50ms, baseline × 25%) 时进入 warning；同时满足 3s 缓冲延迟增长 >= 100ms 时进入 congested，缓冲证据独立于修复包数量。RTT 单项增长先发布警告。
3. 压力期间冻结健康 RTT 基线的向上更新；健康窗口可更新基线，peer/candidate-pair 切换重新建立基线。稳定高 RTT 路径、单次尖峰、旧样本和路径切换分别有定向用例。
4. 质量动作复用现有 3s 持续资格、0/1/2 档、15s 单动作冷却、远端接受验证与串行协商。原因明确区分 transportLoss、delayAccumulation、networkEvidenceIncomplete；本地 read/receive→submit 与呈现帧龄归入显示诊断。
5. 按触发原因评估效果：丢包动作沿既有丢包改善口径；延迟动作要求 RTT 至少下降 max(10ms, 动作前 RTT × 20%)，缓冲延迟至少下降 max(20ms, 动作前缓冲延迟 × 20%)，接收供给保持 >= 95%，丢包增长 <= 1 个百分点。使用动作完成后的完整 3s 窗口及现有响应期限。
6. neutral/degraded/unknown 复用现有单次回退和暂停；记录基线、覆盖、RTT/缓冲前后值、供给变化与实际远端码率结果。

最小验收：0% 丢包、RTT 30→180ms 的 warning；同窗缓冲增长触发 congested；平稳高 RTT、尖峰、缺字段和路径切换；延迟改善成功及无效回退。Browser 实机观察现有 GCC 与应用协商动作的共同结果。

### M4 / 3：已确认的本地停顿快速恢复

1. 为 supervision sample 增加可选 localCounters，携带累计视频包、接收帧、解码帧和视频推进；同时携带 renderer 提交计数和 rendererGeneration。Browser 从 StatsService 的已提交样本读取事实；Rust 在既有监督模块内按相邻累计样本构造局部 1s 连续证据。
2. 快路径资格：当前前台、已出现首帧、启动 warmup 完成、peer/video/renderer 代次一致、至少两个相邻新样本及 >= 1s 连续证据，并达到当前稳态停滞门槛。30/60fps 解码停顿的首动作目标为最后进展后约 2–3s，计时器量化按当前 1s 监督节拍记录。
3. 区分局部事实：视频入流推进、解码计数停滞且视频/提交均停滞 → decode；解码继续、最终显示执行停滞 → presentation；canvas 提交继续推进时，稀疏 video callback 计入观测差异。
4. decode 快路径返回明确的局部资格，执行一次既有 keyframeRequest，并结束当前入流 8s 宽限对这次局部动作的阻挡；证据不足的常规观察继续走既有宽限。presentation 路径交给现有 renderer 失败/重试所有者，必要时复用 playbackRebuild。
5. 同级重复沿 15s 冷却；上一动作完成效果观察后，以新鲜、连续的无效证据允许同 epoch 升级一次 playbackRebuild，消费现有预算。动作在途保持单次执行，旧样本、后台与计数重置中止本轮快路径资格。
6. 连接恢复继续由 mediaConnectionEligible 和公共生命周期仲裁决定；局部恢复记录 faultDomain、证据持续时长、动作、效果及新帧时间。

最小验收：持续入流下的解码停顿、仅显示停顿、真实无入流、30/60fps、稀疏 callback、后台/恢复前台、旧代次/计数重置、动作在途和升级预算。Browser 恢复效果通过连续解码及视频/renderer 提交证明；原生 ReferenceChain/物理上屏指标采用各自能力口径。

### M5 / 5b：音频就绪、统计和局部恢复

1. 将“元素挂载”和“输出路径就绪”分开观测。直接播放消费 audio.play()、playing/pause/waiting/error 及 track mute/unmute/ended；Web Audio 消费 context statechange/resume 结果和当前 live 轨道。media.audioReady 由有效输出路径就绪后发出，所有异步结果检查代次。
2. StatsService 在既有 getStats 单次采样内选择实际音频接收轨道，增加音频累计值与同代次差分：bytes/packets、jitterBufferDelay/EmittedCount、totalSamplesReceived、concealedSamples/concealmentEvents、insertedSamplesForDeceleration/removedSamplesForAcceleration，以及浏览器提供的 playout 字段。
3. 音频输出状态、声道执行结果和采样覆盖汇入既有诊断/trace；同步更新类型和生产 trace 字段白名单。A/V playout 差值仅在两者时间域和覆盖可比时发布，字段缺失明确标记能力不足。
4. 前台恢复、输出设备变化、track unmute 与 context suspended 触发有界检查：先恢复当前 play/context；确认失败后按当前轨道重挂现有音频图一次。保留当前音量与声道，释放上一图和监听器；操作结果由状态及后续样本推进确认。
5. 音量 0、远端静音和用户主动暂停按各自状态解释。浏览器的同步和音频缓冲继续由现有 WebRTC/媒体栈负责。

最小验收：play 成功/拒绝、context running/suspended 与恢复、增益路径静音元素、轨道替换、设备事件、零音量、音频计数缺失/回退和销毁。macOS 前后台、输出设备切换及持续串流听感采用实机短测。

## Impacted Modules

| 阶段 | 主要修改范围 |
| --- | --- |
| M1 | player/domain/media.ts 与 config.ts；browser-runtime.ts；SessionService、SdpManipulator、MediaService、AudioEffectsService 及其既有测试 |
| M2 | InputService、输入反馈队列及既有 InputService/InputDelivery/frame-feedback 测试 |
| M3 | Rust recovery/supervision.rs、quality.rs 与 supervision.test.rs；共享 RPC DTO、Browser 网络投影/诊断、共享监督基线测试 |
| M4 | Rust recovery/local_stall.rs、supervision.rs；共享 RPC DTO、Browser 统计事实/监督/恢复执行及其既有测试 |
| M5 | PlaybackService、MediaService、AudioEffectsService、StatsService/统计类型；PlayerEvents、Browser 生命周期与播放健康投影、生产 trace 合同及既有测试 |

## Validation

- [x] 当前源码问题和入口核对；证据见[本轮规划验收](../../dev-docs/validation/2026-11-26-pc-streaming-latency-recovery-audio/README.md)。
- [x] 各阶段定向冒烟及新增边界通过；失败按受影响用例复验，最终前端193/Rust23项去重结果全部通过。
- [x] Rust cargo fmt 和监督用例通过；Tauri 库编译检查通过。
- [x] pnpm lint:fix、pnpm typecheck 与相关音频/恢复集成通过。
- [ ] 实机同场景短测：网络 RTT/jitter/丢包与缓冲；恢复动作→新帧；media supply 与 steady supply；read/receive→submit 和实际 submit/present 长尾；received→decoded→delivered→submitted 增量；反馈发送年龄；音频样本推进、声道分离和前后台/设备恢复。
- [x] 生产字段投影与字节预算通过15项断言；提交/GPU 完成边界和物理上屏保留各自能力口径。
- [x] 必要摘要与失败诊断归档；本轮清理记录见实施验收证据。

## Risks

- RTT 基线和接收缓冲反映多类因素：独立警告、连续覆盖、同路径基线与动作后供给检查共同限制误判。
- 提前的局部动作可能遇到短暂浏览器调度迟滞：启动/可见性/代次资格、提交计数与单动作预算共同限制误恢复。
- 浏览器音频轨道声道与物理设备能力的可观测性有限：requested/codec/source/output 分别记录；直接播放和图回退附带能力与原因。
- 反馈包频率会增加：用短批次、可写通知、低频统计和队列预算约束额外开销。

## Progress

- [x] 明确编号与追加的声道问题，完成当前 main 源码核对。
- [x] 固定 M1 → M2 → M3 → M4 → M5 的单窗口实施顺序。
- [x] 完成可执行方案、风险边界与最小验收计划。
- [x] 用户确认开始实施。
- [x] 实施 M1–M5 并完成自动冒烟。
- [x] 完成代码实施 Report 与任务状态更新。
- [ ] 用户实机短测后收录真实收益和设备结果。

## Execution Notes

- 2026-11-26 | completed：用户确认后单窗口完成五阶段。声道贯通、反馈短批次、Rust 低丢包排队/效果判定、局部恢复和音频播放闭环均已落代码；193前端、23Rust、15生产投影断言及格式/类型/Tauri编译通过。
- 实现收敛：局部事实以同一已提交样本的累计计数构造，覆盖监督节拍抖动与可见性资格；renderer scope 使用提交代次闭环；声道解析支持空 fmtp、重复参数与线性匹配。
- 实机验收：Home/Cloud 听感、声道、设备切换及六维串流健康接续短测，账本保持 Validation Pending。

- 2026-11-26 | in-progress：用户确认实施；先完成声道配置与播放格式消费。共享 Rust 文件已有 iOS 缓冲统计改动，沿当前内容追加 PC 资格和判定。

- 2026-11-26 | planned：本轮仅完成规划文档与源码引用冒烟；运行时实现待执行确认。
- 当前工作区已有其他任务的渲染实现和证据改动。本轮产出限于此 RFC、本轮证据和任务账本的一条登记。
