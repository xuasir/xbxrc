# 浏览器方案完整功能保留与代码精简 RFC

## Status

- Completion: P1–P4 主体及 S0–S3 策略代码、自动验证完成；P0 配对性能基线与 P5 / S4 实机验收进行中。
- Current State: in-progress
- Owner: 主窗口直接分析、执行与验证，遵循用户指定的协调方式。
- Last Updated: 2026-10-29
- 执行交接：用户指定 Mac + 主机手动测试，并要求暂停自动运行；后续依据新日志继续分析与验收。
- 规划基线：`02f6db40d6138787ca85a582dedf3b5028ad57ea` 加规划时工作树改动；实施起点为公共可靠性改造提交 `bec3acfb`。
- 实施授权：用户明确“按计划实施，直到改造完成”，持续执行 P0–P5。
- 当前策略交付：2026-10-29 用户授权的源画质保真、新帧节拍、故障域隔离、提交监督、稳定软动作与观测预算已实现；追加的标准 SR 输出和稀疏回调供给修正已完成[专项验证](../../dev-docs/validation/2026-10-29-browser-standard-output-and-latest-frame-validation.md)。全量 44 文件 / 326 项、18 项报告 fixtures、lint、类型检查及生产构建通过。[Mac/cloud 日志](../../dev-docs/validation/2026-10-29-browser-policy-fresh-trace-validation.md)保留为修复前基线，新构建的帧供给、共享面板样本与完整配对/长测继续验收。

- 16:47 新构建持续前台日志已完成[查验](../../dev-docs/validation/2026-10-29-browser-standard-output-fresh-trace-validation.md)：源下限、标准 SR 尺寸、正常退出和观测预算通过；同窗 received/decoded/video 约 30fps，marker/SR 提交记录约 3.6fps。用户补充实际观感明显好于3fps，[离线核验](../../dev-docs/validation/2026-10-29-sr-frame-delivery-offline-audit.md)确认正常CSS中SR canvas覆盖video，成功处理计数覆盖三种SR路径及截图；两路帧身份合批复现4/3.7fps，具体实机输入与最终合成层继续缺证，S4先补观测再决定调度；采样边界、共享样本及1440p成本继续验收。

## Background

承接[桌面实现复杂度审计](../isu/2026-10-27-desktop-implementation-complexity-audit.md)的“浏览器方案”部分。目标是完整保留现有功能，通过统一事实来源、合并重复策略、明确动作权限与资源归属，降低维护成本并提升播放稳定性。

本 RFC 采用当前功能全集：浏览器 WebRTC、video/WebGL2、USM/CAS、FSR1 EASU/RCAS、动态显示和画质适应、麦克风、截图、统计、输入及故障恢复全部进入保留清单。[早期浏览器稳定版探索](../isu/2026-10-27-browser-stable-desktop-plan.md)中的功能范围收缩作为历史选项保存；本次交付按完整保留合同推进。

[串流公共主线 RFC](2026-10-28-streaming-common-reliability-simplification.md)已实施启动取消、连接事务、协商队列及部分结构拆分，整机验收继续进行。本任务复用其 `browser-negotiation`、`browser-supervision`、`connection-recovery` 和 operation/generation 合同，实施起点以公共主线最新源码及实际验证结果为准。

阶段顺序：**功能与故障基线 → 单点统计 → 能力与动作校准 → 播放/显示资源收敛 → 策略与状态精简 → 整机回归。** 当前 P1–P4 主体已落地，源画质下限、帧调度与控制域隔离的自动回归通过；后续以 P0 配对基线和 P5 / S4 设备验收收口。采样精度与用户配置继续承接，策略触发与默认行为按本轮体验合同校准。

## Goal

1. 主机/云游戏启动、音视频、输入、菜单、显示增强、麦克风、截图、诊断及异常恢复完整保留，已有用户配置继续生效。
2. 同一媒体代际只有一个 WebRTC 统计采集入口；监督、画质、显示和面板使用同一份有身份、时间窗和新鲜度的数值事实。
3. 动作资格、动作执行和播放效果分别取证；远端画质、播放元素重建、关键帧消息与真实解码/显示进展具有准确语义。
4. 浏览器 runtime 收缩为装配与生命周期入口，状态按连接、观测、播放/显示、局部恢复归属；重复状态和重复决策得到实际削减。
5. 正常游玩、弱网、前后台变化与显示/麦克风操作保持稳定；误恢复、重复连接、晚到事件复活、持续卡键和正常退出资源泄漏按发布阻断处理。
6. SR 各级计划、资源分配与实际输出保持真实源尺寸下限；固定用户/源/显示/能力时，网络样本与告警变化保持基础显示计划稳定。
7. 新帧获得可用显示机会，received/decoded/video advancement/callback/renderer submitted 独立统计；软异常经过本域证据、连续时间覆盖、单动作与效果闭环。

## Scope 与任务交接

| 范围 | 当前入口 | 本次责任 |
| --- | --- | --- |
| 浏览器统计与投影 | `src/player/app/stats/StatsService.ts`、`domain/stats.ts`、`browser-runtime.ts`、`diagnostics.ts` | 单点采集、数值事实、窗口、新鲜度、面板与监督一致性 |
| 能力与协商动作 | `PlayerClient.ts`、`SessionService.ts`、`WebRtcTransport.ts`、`browser-negotiation.ts` | 接收端能力表达、画质动作提交、协商串行化 |
| 播放与显示 | `PlaybackService.ts`、`MediaService.ts`、`Renderers.ts`、`SuperResolutionWebGL2Renderer.ts`、`browser-render-policy.ts`、`browser-super-resolution-state.ts` | 帧事实、元素/renderer 所有权、后处理、SR、失败回退、截图 |
| 运行期监督与恢复 | `browser-supervision.ts`、`browser-runtime-profile.ts`、`browser-runtime.ts`、`connection-recovery.ts` | 有限局部动作、效果判断、策略状态收敛及连接动作接线 |
| 用户行为与验证 | `runtime-contract.ts`、`runtime-host.ts`、串流页面、既有输入模块与测试/trace 工具 | 保留外部合同；面板/设置/输入交接的组合回归 |

远端会话、认证、排队、keepalive、取消与远端清理继续由公共主线管理。统一 GamepadBackend、公共 InputController、Windows 默认 keep-active 和 geometric pathfinding 导航继续采用[统一手柄 RFC](2026-10-27-unified-gamepad-input.md)。Rust runtime、协议传输与原生媒体算法保持当前归属；共享类型变更同步验证它们的消费者。发行依赖、默认构建和产品功能范围由对应任务维护。

公共任务与本任务共享浏览器文件。每个实施阶段先吸收公共主线的已验证结果，按本 RFC 的增量改动落单一版本；共享取消、连接事务和恢复期限以公共 RFC 为权威。公共实机缺项在两份文档关联记录，整机完成结论需要完整证据。

## 实施起点证据与优化优先级

| ID | 当前证据 | 优化决定 |
| --- | --- | --- |
| B1 统计重复采集 | `StatsService.start()` 同时运行 1s snapshot 与 250ms 专项采样，两处直接调用 `peer.getStats()`；监督每秒另取 snapshot，面板轮询及显示策略也会取 snapshot | P1 保留 250ms 精度，统一采集；其他入口读取缓存或订阅 |
| B2 监督与面板口径错位 | `checkMediaStalled()` 直接读取 client 原始统计；`runtime.snapshotStats()` 另合并 `resolveBrowserStatsFallback`。首批 Mac cloud 报告的 39 个恢复窗口记录 decodeFpsAvg=0、jitterMsAvg=-1，同时 RTP 统计已有实际解码与网络 jitter | P1 共用数值快照与窗口，增加同 sampleId 的监督/面板一致性回归 |
| B3 增量与身份保护不足 | 两条采集路径共同更新 `lastBrowserWebRtcSample`；旧 snapshot 的 `lastStat` 在 stop 后保留；delta 函数直接相减；候选对样本与路径描述各自选取候选对 | P1 按 peer/track/SSRC/candidate pair 身份重置对应基线；一次解析后单向投影 |
| B4 画质动作与效果混合 | video transceiver 为 recvonly；sender policy 查找本地 video track。当前测试刻意允许 missingVideoSender 时把 qualityLadderLevel 推到 L0；SDP 降档在异步协商前更新逻辑档位和配置 | P2 保留质量评估与可执行的 SDP 适应；分别持有用户意图、已提交配置和观测结果 |
| B5 播放元素与解码器语义 | `resetVideoDecoder()` 销毁 renderer/video 并把原 MediaStream 挂到新 video；boolean 只表达同步重建动作 | P2/P3 使用 playback rebuild 语义；效果依靠当前代际的关键帧、解码和新帧进展 |
| B6 帧采集和调度重复 | PlaybackService 负责 RVFC metadata/呈现观测；WebGL 与 SR 各自订阅帧回调，SR 另有 rAF 绘制节拍；browser-video-display 的独立 tracking helper 当前生产调用为零 | P3 共用一个视频帧事实源；renderer 保留自身显示预算；清理经过调用链核对的重复实现 |
| B7 资源交接分散 | PlaybackService 的 attach/fallback 异步链直接使用可变 renderer；切换、重建与销毁共享这些字段；视频尺寸还写入 `globalThis.resolution` | P3 为视频/renderer 资源加代际保护，尺寸归当前播放对象；异步提交前验证身份 |
| B8 策略重算与状态规模 | browser runtime 已从原审计的约 3,925 行收缩为 3,429 行；函数直属 `let` 绑定为 129 个，含统计、画质、显示、恢复、摘要等多类状态。fresh trace 记录 refresh estimate 触发重算 90 次 | P4 收回各域状态与唯一 plan；按有效输入变化提交显示；重算频率与实际 draw 成本分别验证 |

B1 实施前受控源码模拟：运行实施起点 StatsService 的两条时钟，另模拟每秒监督与每两秒面板，在 `getStats` 立即完成的 2s 窗口得到 13 次调用，即标称 6.5 次/s；显示事件调用另计。该值证明实施前调用组合，运行时调用耗时、GPU 成本和性能收益由实测给出。

实施前维护规模快照：`browser-runtime` 3,429 行、negotiation 453、supervision 114、PlayerClient 392、StatsService 768、PlaybackService 375、Renderers 927、SR renderer 1,235；这八个生产文件共 7,693 原始行，包含注释与空行。实施时统计同一集合及其新拆出文件的总量，测试单列，文件拆分后的门面行数单列。

实机依据见[首批 Mac 云游戏报告](../reports/2026-10-28-cloud-streaming-reliability-trace-validation.md)：构建、协商、首帧与播放区间已取得证据；可见启动段记录 7 次冻结/3.021s，后续包含后台，程序由用户强制终止。正常退出、真实故障恢复、长期前台播放和跨平台准入继续待验。

## 功能保留清单

每行在实施报告中填写“原入口 → 新入口 → 自动回归 → 实机结果”。用户配置的值、保存/应用时机和实验标识继续承接。自动策略的触发、权限与默认行为采用下述 2026-10-29 修订合同，已有成功功能按该合同保留。

| ID | 保留功能 | 当前归属 | 验收要点 |
| --- | --- | --- | --- |
| F1 | home/cloud、主机唤醒/预检查、云排队/取消、凭证刷新/保活、远端终态 | 公共 session、execution、browser negotiation | 原业务链通过；播放期状态回退与观测失联保持正确处理 |
| F2 | codec/profile、分辨率/码率/音频 SDP、直连/TURN、增量 ICE 与结束标志 | SessionService、SdpManipulator、browser-negotiation | 实际 offer/answer、DataChannel 和重协商互通；home/cloud 分列 |
| F3 | video 音视频播放、音量/静音、音频处理、自动播放失败与输出设备变化 | PlaybackService、MediaService、AudioEffectsService | 声画连续、用户手势恢复、独立音轨/媒体对象释放；下行音视频分别检查 |
| F4 | 标准手柄、键位/死区、键盘桥、虚拟输入、震动、菜单/软 Home、几何导航 | InputController、InputService、GamepadDriver、协议通道 | 当前 owner、按住交接、中性释放、热插拔、keep-active、follow-app 和重连后恢复 |
| F5 | 文本、控制消息、input/control/chat/message 四通道及 metadata 节拍 | 协议通道、InputPacketEncoder、页面/RPC | 通道 profile、握手顺序、包序列和 metadata 语义保持；背压/关闭得到有界结果 |
| F6 | video/WebGL2、USM/CAS、色彩/锐化、Contain/Stretch/Zoom、全屏/DPR/视口 | browser-render-policy、Renderers、browser-video-display | 模式/设置变化生效，源尺寸与输出预算正确，视觉与截图对照 |
| F7 | FSR1 EASU/RCAS、真实源/用户目标重算、稳定锐化、保真回退、用户显式重试 | SR state/ladder、SR renderer、PlaybackService | 各分支输出保持源下限；同尺寸跳过 EASU；锐化消费源/用户意图；回退和开关重试保留 |
| F8 | 有资格的网络适应、增强成本处理、画像诊断/warmup、显示刷新估算 | browser-runtime-profile、render policy、browser runtime | 网络与本地动作各用本域证据；健康时计划恒定；持续软故障单动作、效果验证与稳定恢复 |
| F9 | 麦克风开启/关闭、权限/设备失败、chat 协商和重连后的意图恢复 | MicrophoneService、ChatChannel、negotiation queue | 协商顺序正确；权限/上行失败后下行继续；取消/退出释放音轨 |
| F10 | 首帧保护、短断连宽限、关键帧请求、播放重建、ICE restart/full rebuild、手动重试 | supervision、connection coordinator、runtime | 局部修复、硬断链、预算/期限与真实新帧效果闭环；stop 优先 |
| F11 | 统计面板、trace/故障导出、截图/SR 对比导出、准确错误展示 | StatsService、diagnostics、renderer capture、串流页面 | 同窗数值一致；截图使用实际当前管线；超时/关闭 waiter 清理；报告可复算 |
| F12 | 配置持久化与热更新、运行时选择、Browser/Rust 输入选择、公共消费者兼容 | 配置/RPC、runtime contract、公共输入 | 用户偏好完整迁移/保留；共享类型、原生与 iOS 公共调用继续编译 |

浏览器和设备能力缺项按当前支持范围记录。例如 RVFC metadata、网络字段、扳机震动和实际远端画质控制需要能力与服务证据；功能的入口、成功分支、失败反馈及既有回退一起保留。

## 目标职责与核心合同

### 1. 一个统计采集器，一份事实快照

沿现有 StatsService 收敛采集和解析。初始采样周期继续为 250ms，FPS/公开 stats 事件继续按 1s 聚合，面板保留当前刷新体验。`snapshot()` 的 Promise 接口继续可用，返回最近一次合并快照；一次读取保持基线与控制状态稳定。连接后首次采集与必要的显式刷新共用该采集入口和当前 single-flight。

快照包含：session/runtime/peer generation、sampleId、采样开始/完成时刻、elapsedMs、数据源、可用字段与新鲜度；数值层包含网络、接收、解码、音频、视频回调、绘制及控制通道事实。面板字符串由数值层产生，控制判断直接读取数值。

统一规则：

- 每个当前 peer 至多一次在途 `getStats`；长调用期间跳过周期 tick。观测超时标记 stale/unknown，原 Promise 的在途屏障保持到真实结束或旧 peer 失效；晚到结果核验身份后提交。
- peer 替换重置全体基线；track/SSRC/reportId 变化、候选对切换与计数器回退分别重置对应增量。首样本及不可比较样本使用 unavailable，记录 reset reason。
- packetsLost 的负增量按浏览器统计修正处理，保留原值与修正原因，窗口损失比例采用归一后的有效计数；bytes/frames 等累计计数回退按其资源身份重置。各类计数的修正规则分别测试。
- 接收视频按当前 transceiver/track 身份确定；候选对优先按当前 transport.selectedCandidatePairId 关联，平台字段缺失时使用明确标识的兼容选择。路径、码率、RTT 和候选对诊断复用这次解析。
- 网络 jitter、jitter-buffer 平均等待、receive/decode/callback/draw FPS 分开。累计计数、5s 面板 loss 窗口、3s/15s 策略窗口采用同一有界样本账本；窗口聚合按真实 elapsedMs 加权。
- 数值缺失与真实零值分别表达；freshness 初值 1s，由 P0 固化并在目标 WebView 上校准。stale/unknown 触发观测诊断，破坏性动作需要新鲜事实或独立硬故障证据。
- `StatsService.stop/reset/dispose` 区分暂停和永久销毁；销毁解除 FPS/input 订阅并失效在途结果。分辨率来自当前 track/video，公共全局变量的生产读取迁到快照。

### 2. 能力、动作、效果各自准确

在现有 PlayerClient/SessionService 表达浏览器动作能力：本地 video sender 参数、远端 SDP 画质请求、Xbox keyframe 消息、播放元素重建、renderer 切换、chat/麦克风协商、ICE restart、full rebuild。能力随 peer 创建更新；动作记录 supported/unsupported/failed 及原因。

质量评估等级可以继续用于策略解释；真正应用的画质由 `requestedQuality → negotiatedQuality → observedRemoteVideo` 三项表达。recvonly 场景直接跳过缺失本地 video sender 的尝试；已有 SDP 升降档与网络适应走当前协商能力。服务忽略请求或有效观测缺失时分别标记效果，保留上一份已验证状态。

SDP 变更以不可变意图进入现有 negotiation queue：配置、画质、麦克风、连接恢复有各自原因，stop 通过取消代际先失效；连续配置合并到最新待执行意图，已提交动作保持单次结算。发生实际 ICE restart/full rebuild 时接入现有连接事务预算。协商完成核验当前身份后提交配置，失败保留原有效配置与用户目标并给出诊断。

`resetVideoDecoder` 迁为“播放接收端/元素重建”的准确 API 与 trace 名称，内部调用一次性切换。旧导出确有消费者时保留薄转发并在迁移完成后清理。keyframe `sent`、renderer `attached`、playback `rebuilt` 和 recovery `observed` 分别记录；播放恢复依靠当前代际持续真实新帧，WebRTC decoder 内部重置效果由平台证据说明。

### 3. 播放/显示拥有资源与帧事实

PlaybackService 持有当前 video/audio/MediaStream、一个权威帧观察源和当前 renderer。RVFC 优先，平台缺失时保留兼容节拍，明确回调、呈现推进和实际 draw 的区别。metadata、FPS、首帧与恢复观察消费同一个带 sequence/video generation 的帧事件，InputService 的 Xbox metadata 保持协议语义。

WebGL 与 SR 从同一帧源接收 latest frame marker，保留应用侧显示预算和必要的 rAF 调度。高刷新屏的一帧至多执行一次对应 FSR/锐化处理；截图可申请一次有界绘制。兼容回调可证明的内容继续按可用范围报告，sourceFps 缺失原因保留。

2026-10-29 新日志后的用户修订：SR 基础档位计算继续承接，放大输出采用 1920×1080、2560×1440、3840×2160 等标准档位。1600×900、1707×960 等视口需求归一到对应标准档位，最终 CSS 视口独立映射；passthrough 继续保留真实源尺寸。执行层消费已确定的 SR 输出，GPU 能力故障沿现有保真路径处理。

帧供给修复继续使用同一 VideoFrameSource：RVFC 保留原始 RTP/时序 metadata，rAF 在同一 owner 内读取可用的浏览器呈现帧计数，补足稀疏或合批回调间的新帧交付。两个入口共用帧身份、delta 和取消代际；原始回调与补足交付各自记录。SR 收到新帧立即尝试处理，明确用户显示上限的等待才使用已有累计节拍。设备帧计数来源和缺证如实进入资源快照。

接收目标帧率始终保持用户设定。窗口隐藏只暂停本地绘制，网络适应保持有限码率请求权限；本地调度、SR 成本、回调抖动与实际供给下降各自保留观测。自动回归覆盖 RVFC 15fps 与实际 30/60fps 供给、mediaTime=0、重复/晚到 metadata、前后台恢复与码率协商。

renderer attach/update/fallback/capture 使用资源代际。先完成新 renderer 可用/首帧，再切显示；旧 renderer 在交接后销毁。交接阶段至多一个当前 renderer 和一个待提交 renderer，完成或失败后待提交资源归零。迟到 attach、context restore、截图结果与旧 video 事件只处理其自身资源。正常配置/renderer 切换保留 PeerConnection、MediaStream 和音轨；播放元素重建作为独立恢复动作。

SR 故障复用现有 `webgl2_sr → webgl2 USM/CAS → video` 回退能力，各候选先满足源画质下限；增强成本故障优先选择具有有效供帧证据的保真路径。当前请求/实际管线/原因分别投影，用户关闭再开启 SR 的重试能力保留。软压力经过连续本域证据与效果验证；硬 context/shader/FBO 故障及时执行显示回退。SR 输出、真实尺寸重算和源/用户驱动的锐化共用一份 renderer plan；真实源/显示/用户目标变化持有可追踪的待应用目标。

资源释放覆盖 rAF/RVFC、ResizeObserver、DOM 事件、媒体引用、音轨、GPU 对象、SVG/filter 节点、截图 waiter 和订阅。截图超时使用可取消 waiter，结束后从队列移除；stop 后资源计数回到本会话创建前的基线。

### 4. 监督作一次局部决策，连接事务作一次连接提交

browser supervision 继续持有首帧与监测时钟。用同一快照/窗口产生纯决策：输入为事实、能力、用户意图和当前 episode；输出为一个最小动作及原因。统计摘要与 UI 字段从权威状态派生，网络、解码与显示各自保留必要的独立健康事实。

| 新鲜证据/用户意图 | 处理与权限 | 效果验证 |
| --- | --- | --- |
| 只有统计缺失/晚到 | 标记观测不可用，等待当前或下一样本 | 恢复正常采集；独立 Failed/Closed 事件继续处理 |
| 视频/解码持续推进，renderer 硬失败 | PlaybackService 本地回退显示 | 新管线真实 draw/视频可见性，音频与输入连续 |
| 接收活跃，解码/新帧持续停顿 | 限频 keyframe，再按现有预算尝试 playback rebuild | keyFramesDecoded/decode/new frame 的当前代际增量 |
| 网络供给证实拥塞 | 有能力的画质协商；本地显示压力处理由 renderer plan 决定 | 实际协商与远端观测、稳态供给及恢复升档 |
| 短 disconnected、近期媒体持续推进 | 延续当前连接宽限与健康 gate | 连接和新帧进展，宽限结束重新取证 |
| Failed/持续硬断链/控制通道失败且媒体无进展 | 公共授权 → 现有连接 coordinator → negotiation/rebuild | actionCompleted 与 stableMedia 分别结算 |
| stop/远端终态/新 generation | 取消旧操作，当前资源统一释放 | 晚到结果隔离，清理完成，输入释放 |

沿用公共主线当前连接初值：每故障 episode 最多 2 次实际连接动作，总期限 120s，连续稳定新帧 5s、帧间隙上限 1.5s；restart 失败后的 full rebuild 计第二次。首帧保护与局部 keyframe/rebuild 继续使用既有有限预算，等待/效果时序按有效路径 RTT 和本域证据校准。画像、warmup 与软动作驻留按下述体验合同重审；公共合同值变化同步更新公共 RFC。

媒体前后台监督与输入后台策略分别消费公共生命周期事实。页面隐藏/睡眠的节流窗口单列，恢复可见后先取新鲜样本并重新开始宽限；Windows keep-active 的输入意图按既有输入合同保持。音频、视频和控制通道故障各自取证，麦克风失败保持下行游玩可用。

### 5. 2026-10-29 策略方向修订

完整推导、代码因果与后续顺序见[策略重设计](../isu/2026-10-29-browser-policy-stability-and-native-quality.md)。本节为当前实施合同，已有阶段和验证历史继续作为证据记录。

| 合同 | 实施要求 |
| --- | --- |
| 基础计划确定性 | 用户意图、真实源、物理显示需求、能力和当前 renderer 故障状态决定基础计划；网络样本与告警刷新保持该计划恒定 |
| 源像素下限 | SR backing 宽/高均至少为真实源宽/高；ladder、plan/renderer clamp、FBO 分配和截图共同执行。较小视口通过最终显示映射处理，成本超限通过现有保真回退处理 |
| 标准 SR 目标 | 基础档位计算继续承接，放大输出采用 1920×1080、2560×1440、3840×2160 标准尺寸；视口选档与最终 CSS 映射分别执行，源超出目标时成对保留原生尺寸 |
| 等尺寸与锐化 | 1440p 源在 1440p 屏保持等尺寸，跳过 EASU；源/用户决定稳定 RCAS，fit/色彩/锐化保持用户配置。感知质量另用同源图像验证 |
| 网络权限 | 删除 presentAge、callback gap、renderBackpressure 的带宽决策权；码率占请求上限比保留为诊断。传输动作消费持续真实拥塞、有效能力及效果证据 |
| 高 RTT 维稳 | 路径 RTT 基线和持续增量分列；稳定高 RTT 保持画质和供帧。恢复等待覆盖路径响应时间且服从有限期限，显示时钟按本地节拍运行 |
| 新帧节拍 | 同一帧源使用 RVFC metadata 与 rAF 可用帧计数补足 latest；新帧立即尝试提交，必要显示上限用累计截止时间。接收帧率请求保持用户目标，250ms 接收吞吐与源节拍分别表达 |
| 显示事实 | 当前 renderer submitted/age 接入监督；received、decoded、video advancement、callback、submitted 同窗分列，实际 scanout 缺证明确展示 |
| 稳定软动作 | 复用 3s/15s 有界窗口，以真实时间覆盖确认症状和恢复。同域一次动作先完成效果观察，软动作间隔初值至少15s，恢复增强先有健康长窗；硬错误及时走既有回退 |
| 观测与告警 | 前端先判启用/profile再构造payload/IPC；热路径有界计数，已有1s节拍聚合；告警按episode去重。观测/面板变化保持媒体控制事实一致 |
| 代码减法 | 缩窄本域输入类型；收回混合 dynamic overlay、网络驱动 RCAS/fit、重复 ladder 状态及同意图重评估。既有 plan、协商与连接执行链继续唯一 |

网络效果窗口至少覆盖短窗与两次有效路径 RTT，且服从公共 episode 的有界期限；恢复增强的有界试探失败后保留已验证路径，下一次尝试依靠新的用户意图或能力/源上下文。3s/15s 为第一批验证起点，设备成本阈值以真实数据校准。当前 cloud 日志的真实丢包/缓冲积累列入传输分析，显示计划持续独立；home 注册失败列入启动分析。

## Plan

| 阶段 | 改造内容与产物 | 进入下一阶段的门槛 |
| --- | --- | --- |
| P0 基线与合同 | 固化 F1–F12 映射、源集合/构建指纹、配置默认值；补 B1/B2/B4、采样乱序、renderer 晚到、截图关闭的真实边界 fixtures；记录同组合 video/WebGL/SR 运行基线 | 自动测试可分别复现旧风险和成功行为；现有功能矩阵明确；性能比较规则固化 |
| P1 统计单点化 | 合并 StatsService 两条采集；统一解析、样本身份、窗口和缓存；监督/面板/显示接同一快照；去掉无消费的临时统计对象、全局分辨率与重复解析 | 当前 peer 在途≤1；周期原始采集≤4次/s；面板开关的控制事实一致；旧样本隔离；单位/窗口正确 |
| P2 能力与事务 | recvonly sender 能力短路；requested/negotiated/observed 画质分开；keyframe/rebuild 语义校准；画质、配置、mic、restart 共用既有有界队列 | 质量评估与实际画质可独立核验；失败保持有效配置；ICE restart 记账一次；所有用户入口保留 |
| P3 播放与显示资源 | 统一帧事实与 marker；renderer/video 代际交接；集中失败回退、SR plan 和截图 waiter；迁移后清理重复 tracking helper 与资源胶水 | F3/F5–F7/F9/F11 组合通过；高刷新去重复处理；尺寸变化生效；异步 attach/销毁/context loss 资源闭环 |
| P4 策略和状态精简 | 先修源像素下限与新帧节拍，再删除跨域画像/渲染输入、接入真实提交监督、实施稳定软动作与观测热路径收敛；同sample/意图幂等，有效plan才执行 | 源画质/新帧机会、健康计划恒定、单动作效果与全部用户入口通过；健康局部故障连接重建=0；维护指标证明真实减法 |
| P5 整机与收口 | 同组合前后对照、异常/长测、旧配置、跨消费者编译；beta 观察；完成实施报告与任务更新 | F1–F12 每项有证据；自动/trace/目标平台与资源/性能门槛通过，发布阻断项清零 |

每阶段使用独立可回退改动，先改合同与消费链、再清理旧路径。阶段内生产装配保持一条有效路径；旧/新行为对照通过测试或两次独立构建采样完成。公共任务正在验证的生命周期合同加入各阶段回归。

### 维护指标与完成标准

| 指标 | 当前基线 | 目标/证明方式 |
| --- | --- | --- |
| 直接 getStats 生产入口/周期采集时钟 | 2 / 2 | 1 / 1，peer 当前身份与 single-flight 可测 |
| 同一窗口的控制/面板事实 | 两条投影；已存在 B2 偏差 | 同 sampleId 的网络/解码/帧事实一致；格式化只发生在展示投影 |
| 帧事实来源 | Playback + renderer 各自订阅；重复 helper 留存 | 1 个当前 video 事实源；renderer 的绘制调度按自身预算单列 |
| 同一事实的跨域重复可变副本 | runtime 直属 let 129，含摘要和状态 | 权威 owner 逐字段登记；重复可变事实与重复动作判定清零，摘要派生 |
| renderer 更新 | 多输入触发策略重算 | 同一有效 plan 持续不变时 GPU attach/resize/update 次数为0；真实源/视口/用户设置变化可生效 |
| 生产规模/分支/重复实现 | 8文件7,693原始行；相关纯策略/新增文件实施时纳入 | 同范围含新增文件汇总生产语句/分支/状态与原始行数；生产语句/分支至少一项下降，重复状态/路径减少；安全合同增量单列 |
| 功能与稳定性 | 当前自动基线通过，完整实机待验 | F1–F12逐行通过，故障/资源/性能硬门槛通过 |

## Validation

### 自动化场景

| ID | 故障/操作组合 | 必须证明的结果 |
| --- | --- | --- |
| V1 同一采集事实 | 同时触发周期、监督、显示、面板读取；getStats 延迟/超时/乱序/stop 后返回 | 当前原始在途≤1；控制/面板同快照；读快照保持基线；旧结果丢弃 |
| V2 数值与窗口 | peer/SSRC/reportId/candidate pair 改变，计数回退、负 loss 修正、多 inbound video、0/缺失字段、30/60fps | delta 对应身份；网络 jitter 与缓冲延迟分开；实际窗口/单位正确；缺失保留 unknown |
| V3 画质能力与配置 | missingVideoSender、SDP 被服务接受/忽略/拒绝、quality 降档中 mic/配置操作、失败晚到 | 评估/应用/观测分开；旧配置有效；最新意图保留；队列有界；连接预算按真实动作结算 |
| V4 帧与 pacing | RVFC 正常/合批/mediaTime缺失或为0、rAF高刷新、timeupdate、targetFps=0、截图一次绘制 | metadata/首帧/恢复使用当前帧；callback/draw分开；同帧FSR≤1次；兼容功能保留 |
| V5 renderer 交接 | attach pending 时切换/stop/rebuild、SR FBO/shader失败、context lost/restored、连续尺寸变化与显式SR重试 | 一份当前 renderer；旧操作隔离；回退可见；音视频/输入连续；真实尺寸最终应用 |
| V6 资源回收 | 连续launch/stop/retry/reconnect；多个截图超时；mic权限pending后退出；隐藏/唤醒 | media/track/GPU/timer/observer/waiter/订阅回到基线；stop后用户动作与晚到事件隔离 |
| V7 故障权限 | 高RTT健康网络、仅render压力、接收活跃decode停顿、硬Failed、短disconnected、统计stale | 局部修复留在局部；硬断链得到有界连接处理；准确效果；自动恢复预算可结算 |
| V8 用户功能组合 | 菜单/软Home/输入归位、热插拔、keep-active/follow-app、音量、文本、全部显示设置、截图和mic | F1–F12逐项与交叉场景通过；按住交接与正常退出持续卡键=0 |
| V9 源画质硬下限 | 1440p/2160p源 × 小视口/同尺寸/大屏 × DPR1/1.5/2 × 网络/软压力/回退状态 | 各级实际输出保持源下限，硬能力限制得到保真回退；mode与真实backing一致；同源SR开关画质单列 |
| V10 新帧供给 | 30/60fps源、90/144/165Hz屏、32/34ms边界、合批与稀疏回调、源fps未知 | 正常新帧获得显示机会；必要累计限流长期平均准确；每帧实时处理≤1；主动丢帧原因和截图分列 |
| V11 控制域隔离 | 固定用户/源/显示/能力，变化RTT/loss/实际码率/告警；video继续推进而canvas停绘 | 基础render plan相等；网络/渲染故障准确归域；当前renderer提交故障被监督发现 |
| V12 稳定网络与高RTT | 健康低运动低码率、稳定高RTT、RTT基线改变、持续拥塞、字段缺失 | 健康软画质动作为0；unknown保持有效计划；真实拥塞只进入有能力传输动作，效果和有限期限可结算 |
| V13 软动作闭环 | 短尖峰、重复同sample、迟到样本、持续压力、动作无效果、恢复试探 | 连续时间覆盖、单动作、初值15s间隔与长窗恢复成立；真实用户/源/能力变化继续应用 |
| V14 观测扰动 | trace关闭/不同profile、面板开关、告警高频、GPU错误/资源resize | 关闭trace的高频IPC为0；聚合有界；硬故障即时可追踪；开关配对的媒体控制事实一致并测量供帧/CPU长尾 |

新增 tests 写在真实责任模块，覆盖边界和失败时序。`PlaybackService.test.ts`、`PlayerClient.test.ts` 在 P0/P3 建立；profile/runtime 现有测试迁到等价的功能合同，quality assessment 与执行结果分别断言。

每阶段执行相关 vitest；完成前端代码后执行 `pnpm lint:fix`、`pnpm exec vue-tsc --noEmit`、`pnpm build` 和 `git diff --check`。共享 Rust/RPC 改动追加 `cargo fmt`、相关 Rust 测试及 Tauri/iOS 公共消费者编译，完整消费者命令沿公共 RFC 的现有验证入口执行。

规划阶段自动基线：

- `pnpm test`：31文件 / 234项通过。
- `./node_modules/.bin/vitest run src/player/app/stats/StatsService.test.ts src/player/infra/render/Renderers.test.ts src/streaming/runtime/browser-runtime.test.ts src/streaming/runtime/browser-supervision.test.ts src/streaming/runtime/browser-runtime-profile.test.ts src/streaming/runtime/browser-render-policy.test.ts src/streaming/runtime/browser-video-display.test.ts src/streaming/runtime/browser-stats-fallback.test.ts src/streaming/runtime/super-resolution-ladder.test.ts src/streaming/diagnostics.test.ts`：10文件 / 127项通过。
- `pnpm exec vue-tsc --noEmit`：通过。
- 使用 TypeScript AST 统计 runtime 直属可变绑定；使用当前 StatsService 源码转译/隔离执行确认 B1 调用组合。

这些结果固化了规划时自动基线。当前代码交付与新增自动场景见最后一条 Execution Notes 和[代码验证记录](../../dev-docs/validation/2026-10-29-browser-policy-code-validation.md)；P5 整机稳定性继续以用户设备证据验收。

### 实机、性能与 trace 门槛

Windows WebView2（含ROG/FSE）与 macOS WKWebView、home/cloud、直连/TURN分别记录系统/WebView/GPU/输入设备/codec/profile/网络与显示设置。输入后端按当前装配独立记录。每个声明支持的关键组合执行：

1. 50次启动/正常退出，覆盖准备/排队取消、重复启动；客户端误超时、双连接、崩溃、正常退出资源泄漏与持续卡键=0。
2. 100次菜单/触屏/按住交接、20次热插拔、20次前后台/最小化/睡眠恢复；音视频、所有已支持输入/控制与震动行为符合原合同。
3. home/cloud各至少2h持续播放；video/WebGL/SR各取得持续播放与切换样本，核心发行组合追加8h soak。覆盖凭证刷新、码率/分辨率变化、显示设置、mic与截图。
4. 20次受控网络断开/恢复，另覆盖丢包/抖动、TURN、WebGL/SR初始化失败/context loss、mic权限/设备失败、自动播放拒绝和远端终态。每次有动作原因、期限、执行结果、真实新帧与资源清理结果。

P0 固化同机/同服务profile/同网络/同渲染设置的对照基线，至少三轮配对采样并保持相同可见窗口。初始准入规则：启动/首帧/恢复P95/P99劣化上限为 `max(基线×5%, 100ms)`；帧间隔与draw/回调长尾P95/P99上限为 `max(基线×5%, 2ms)`；steady decode/新帧/draw供给下降≤5%；每播放小时冻结总时长与最长停顿保持基线以内；CPU/GPU/稳定内存较基线增长≤5%。小样本P99与运行噪声单列，P0依据实际重复样本固化阈值，后续修改写明测量依据。结构精简的性能目标为持平或改善。

资源数量在每次正常退出后回到基线；2h/8h测试的可达对象、音轨、连接、timer/observer/GPU/waiter数保持有界，heap/RSS趋势结合GC和系统回收解释。真实网络持续不可用按有限恢复/明确失败验收；客户端误关闭健康连接、局部故障误重建和无限恢复为零。

每批fresh trace绑定revision、dirty源码摘要、实际构建、session/operationId、runtime/peer/video/renderer generation、sampleId与配置摘要。当前字段缺项在P1/P3补齐。旧trace用于历史故障fixtures，当前版本完整正常退出与恢复样本用于交付。

| 必报维度 | 浏览器证据 |
| --- | --- |
| 网络健康 | 当前ICE/candidate pair、RTT/loss/jitter、bytes/packets、可用NACK/PLI/FIR增量及字段缺失原因 |
| 恢复链健康 | 同episode的propose/grant/commit/actionCompleted/observed；关键帧/解码/新帧；浏览器内部ReferenceChain等字段的可观测边界 |
| media supply | 接收视频/音频、framesReceived/Decoded、callback/new frame/draw的同窗增量 |
| steady supply | 前台稳态帧率、freeze duration、帧间隔；后台/睡眠单列 |
| submit/present长尾 | RVFC回调、processingDuration/expectedDisplayTime与draw耗时代理量；真实上屏/glass-to-glass追加外部测量 |
| 帧供给delta | 同sample/window的packets→received→decoded→presented advancement→callback→draw；计数单位和合批清楚 |

复用 `.agents/skills/analyze-runtime-logs/scripts/trace_browser_webrtc_behavior_report.py TRACE` 与 `trace_streaming_session_lifecycle_gate.py TRACE --require-cleanup --require-closed`，连接恢复批次追加 `--require-recovery`；构建匹配参数与多文件轮转按[本地验收手册](../../dev-docs/streaming-reliability-local-validation.md)执行。V1/V2/V4/P1/P3新增采样/帧/资源字段，需要在现有浏览器报告补“同样本一致性、乱序/代际、动作能力/效果、绘制去重、资源归零”检查和正负fixtures。现有behavior报告主要证明协商与里程碑；其PASS与新增稳定性门槛分别列结果。

## Risks 与回退

| 风险 | 控制方式 | 回退条件 |
| --- | --- | --- |
| 统一快照后窗口/单位变化影响策略 | P1保留采样精度，P4按当前体验合同修订跨域触发；数值/字符串分层；V1/V2/V9–V14核验 | 统计不一致、源输出下限破坏、健康网络误降档/误恢复或真实故障漏处理 |
| 能力校准改变质量档位解释 | 用户意图/评估/协商/观测逐项映射，保留有效SDP适应；missingVideoSender与服务忽略负样本 | 现有设置/升降档丢失、远端参数错误、mic或连接预算失序 |
| 帧源收敛改变metadata/pacing或截图 | Xbox metadata包与RVFC/rAF两类时钟分别验；高刷新、源30fps、截图与后台组合测试 | metadata错误、重复FSR、画面节奏/截图结果或输入时序劣化 |
| renderer异步交接产生旧资源提交 | 每资源generation、首帧后切换、统一finally/dispose、V5/V6 | 黑屏、旧renderer复活、残留音轨/GPU/waiter、源变化卡在旧尺寸 |
| 只减少门面行数，维护成本转移到新文件 | 同集合含新增文件统计；逐字段owner、重复逻辑/分支与调用链审计 | 重复控制状态增加、并行策略路径增加、功能/性能门槛缺证 |

阶段出现发布阻断项时，停止后续精简，固定失败样本并按该阶段独立提交回退到已验证版本。保留原用户配置和外部数据合同；日志新增字段采用兼容扩展，旧名称迁移完成后清理。正式发布沿既有 release 流程执行。

## Progress

- [x] 核对当前工作树、原审计、统一输入与公共可靠性任务，明确本次完整功能保留边界。
- [x] 完成B1–B8证据、F1–F12清单、P0–P5阶段、V1–V8自动场景及性能/资源/trace回退门槛。
- [x] 运行当前自动基线、类型检查与源码采集组合/状态规模核验。
- [x] P0自动故障fixtures与可复算维护规模基线，新增报告正负/集成 fixtures 已覆盖。
- [ ] P0同组合实机性能基线，至少三轮配对样本与设备阈值固化。
- [x] P1主体与自动回归：250ms单采集/single-flight、数值副本、代际/路径/实际尺寸/帧率窗口隔离、五段帧链、缺证与窗口边界。
- [x] P2主体与组合回归：recvonly能力、画质提交/回滚/效果/有限动作、健康长窗恢复、最新意图合并、协商取消与麦克风晚到释放。
- [x] P3主体与自动回归：共享帧源、5s首帧renderer交接、隐藏预算、硬故障保真回退、截图取消、音频/GPU/DOM/timer资源释放及实际管线投影。
- [x] P4 / S0–S3代码与自动回归：源尺寸下限、新帧累计节拍、控制域隔离、真实提交监督、3s/15s资格与效果、局部预算复用、trace过滤/聚合/队列预算、GPU资源复用。
- [ ] P1–P4目标WebView与完整功能组合的实机复核、像素/感知画质、观测开销配对证据。
- [ ] P5 / S4全部功能/平台/2h与8h长测验收，完成后产出实施Report并收口任务。

## Execution Notes

- Date: 2026-10-28 | Status: planned
- Update: 优化计划落入RFC；原审计和稳定版探索补充本次方向入口，任务登记为Planned。
- Decision: 完整保留浏览器现有功能，优先消除统计事实错位与重复采集，再按能力和资源合同做代码减法。
- Evidence Boundary: 234项全量与127项定向测试、类型检查通过；新故障矩阵、长期资源/性能和Windows/ROG/home/cloud平台证据由实施阶段交付。
- Recommended First Batch: P0与P1。先把监督/面板事实统一，使后续画质、恢复与显示优化具备可信验收基础。

- Date: 2026-10-28 | Status: in-progress
- Update: 已保存实施起点bec3acfb的52个生产文件基线（15,674行/7,039语句/1,137分支）；统计、recvonly画质和麦克风权限回归已落地，41项播放/renderer/runtime定向用例通过。P3资源交接、P4实际减法与P5实机/长测继续推进。

- Date: 2026-10-29 | Status: waiting-for-manual-validation
- Update: 当前全量39文件/273项、lint、vue-tsc、生产构建与diff检查通过；新增浏览器稳定性报告10项正负/集成fixtures通过。`trace_browser_webrtc_behavior_report.py --require-stability`核验共享事实、采样重叠/频率、资源有界/逐client清理、晚到提交与重复绘制。
- Measurement: 同范围包含新增文件：55个生产文件，15,751行/7,027语句/1,239分支，runtime直属可变绑定94（审计基线129）。生产语句减少12；新增分支用于取消、代际、资源与失败时序保护。快照见`dev-docs/validation/browser-runtime-optimization-current.json`；测试与报告工具单列。
- Evidence: 10月28日Mac旧日志绑定02f6db40，与当前实现构建不同；新稳定门槛准确返回MISSING。最新Tauri开发构建已启动并生成空闲宿主日志，实际串流验收继续待验。
- Handoff: 按用户要求停止自动实机运行，Mac + 主机由用户稍后手动测试。Windows/ROG、云游戏、配对性能基线及2h/8h长期验收保留原有待验范围；取得新日志后继续核验F1–F12与稳定性门槛。

- Date: 2026-10-29 | Status: manual-trace-analyzed-validation-pending
- Evidence: 用户手测日志`runtime-trace-1793241116241-1.jsonl`（Mac debug / bec3acfb dirty，476.985s / 5769行）包含2次home创建成功后30s注册等待超时、1次cloud连接/首帧/正常退出。分析记录：[`Mac手动验收`](../../dev-docs/validation/2026-10-29-browser-runtime-manual-analysis.md)。home尚未进入媒体runtime；cloud资源归零、共享样本一致、重复绘制为0。
- Findings: cloud累计包损失5.00%，最后恢复前台24.7s接收28.35fps/解码16.42fps、RTT中位1259ms、jitter buffer平均等待1592ms；持续拥塞与显示/供给长尾保留阻断。P4继续收口14次recvonly评估中的同目标重复、Mac mediaTime=0导致源节拍缺证、55次显示档位和25次FPS预算变化。原稳定报告4项FAIL中3项由RPC到达乱序引起，重算后剩5次采样跨995ms的边界容差问题。
- Handoff: 本轮仅分析日志并更新验收记录；自动实机运行保持暂停。当前样本支持部分资源/生命周期合同，P0/P4/P5及完整功能、主机媒体、平台/长测验收继续待验。

- Date: 2026-10-29 | Status: render-policy-correctness-findings
- User Focus: 用户要求以代码策略正确性、画面保真、接收→解码→呈现→实际绘制差值以及采集/告警成本为主要验收对象。网络统计作为归因上下文。
- Evidence: 同一前台72.152s窗口接收29.92fps、解码28.37fps、SR提交14.21次/s；生产限流复算144Hz/目标60fps上界48fps。L2可将1440p源缩为1080p再放大到1440p窗口，DPR1.5场景还可输出960p；真实Mac日志3次1440p→1080p降采样。审计：[`显示策略正确性`](../../dev-docs/validation/2026-10-29-browser-render-policy-correctness-audit.md)。
- Findings: 本轮fps字段投影与既有显式内容FPS策略存在合同冲突；SR收紧新帧/时间双门槛存在高刷新率节拍风险。video呈现事实与canvas提交/显示监督错位，临时性能预算与真实显示需求共用D，RCAS与请求码率比值耦合，高频trace和GPU查询需收敛。共享样本/资源释放证据保持有效，策略运行效果待修复。
- Next Batch: 优先修正像素保留、实际尺寸/mode投影、内容帧率合同和累计节拍；再接入当前renderer提交监督，收敛自动画质与trace热路径，保留全部用户功能和故障回退。自动实机运行保持暂停。

- Date: 2026-10-29 | Status: policy-direction-redesigned
- User Decision: 用户要求整体策略以稳定游玩为目标，1440p源开启SR在各情况下保持至少源画质的信息量；网络相关与本地渲染/观测各自控制。
- Evidence: 源码复核确认presentAge进入带宽分类、实际码率占请求上限比单项判拥塞、高RTT overlay收紧告警阈值、网络标签驱动本地锐化/fit，形成跨域连锁。此前像素与节拍复算继续有效。
- Update: 完成[整体策略重设计](../isu/2026-10-29-browser-policy-stability-and-native-quality.md)，同步修订F7/F8、核心合同、P4与V9–V14；后续按源下限/节拍→删除跨域输入→提交事实→稳定软动作和观测开销→用户手测的顺序实施。
- Boundary: 本轮修改设计和任务文档，生产源码保持现状；当前策略修复和实机效果继续待验，任务保持In Progress。

- Date: 2026-10-29 | Status: code-complete-manual-validation-pending
- Delivered: 按已确认策略完成 S0–S3。实际源尺寸贯穿 plan/backing/分配与执行检查；FramePacer 使用累计截止时间，共享 latest marker 驱动真实 renderer；网络、解码、显示与观测各有证据和动作归属。StatsService 提供 received/decoded/video advancement/callback/submitted 五段窗口，按 peer/video/renderer、路径、帧率和实际尺寸隔离。
- Stability: 软动作要求完整3s症状覆盖与15s间隔；网络/关键帧效果等待至少3s与两次有效RTT，超过15s期限结算unknown；playback rebuild以新播放代际与本地3s证据结算。隐藏/stale失效隔离晚到完成；健康长窗关闭局部episode并让后续故障获得新预算。增强恢复采用一次试探及效果回退。
- Resources: PlaybackService候选5s首帧期限按隐藏预算暂停，旧renderer同步暂停并统一清理timer；共享帧源/截图队列与资源代际保持有界。uniform、texture/FBO复用，GPU能力/完整性检查绑定资源变化，运行期错误检查保持周期预算；源GPU能力失败直接采用原生video。
- Observability: 面板、诊断和trace分列五段帧率、实际源/输出、CPU提交成本、提交年龄、覆盖/限流/失败/截图计数、RTT基线/增量及requested/negotiated/observed码率。GPU时间与真实scanout标为unavailable。trace入口先过滤后构造，最多4个IPC在途与32个排队，observedAtMs提供单调观测时间；报告同时检查周期资源快照与生命周期资源事件。
- Validation: `pnpm test`（44文件 / 297项）、`pnpm lint:fix`、`pnpm exec vue-tsc --noEmit`、`pnpm build`、报告fixtures（16项）与`git diff --check`通过。新增集成验证覆盖旧局部预算耗尽→新代际健康长窗关单→新故障预算恢复，以及健康高RTT/renderer压力下trace off/dev媒体决策一致。详细合同与命令见[代码改造验证记录](../../dev-docs/validation/2026-10-29-browser-policy-code-validation.md)。
- Measurement: 同范围包含新增生产文件：62文件，15,126原始行 / 6,836语句 / 1,263分支；相对bec3acfb基线减少548行与203语句，分支增加126。runtime直属可变绑定74（审计129，本轮起点94）。正式[当前快照](../../dev-docs/validation/browser-runtime-optimization-current.json)包含逐文件源码SHA-256，测试与报告工具单列。
- Handoff: 自动实机运行保持暂停；当前源码构建的Mac/home/cloud、Windows/ROG、高刷新/DPR与SR同源画质、配对性能、完整功能及2h/8h长测继续待用户设备证据。历史Mac同窗帧差与旧构建trace继续作为整改依据。任务维持In Progress，完整验收后写最终Report。

- Date: 2026-10-29 | Status: fresh-build-trace-partially-validated
- Evidence: 用户新日志`runtime-trace-1793255066737-1.jsonl`为Mac debug/dev、1次cloud串流、341.822s宿主/130.869s媒体采样；另1份44.361s空闲日志。两份构建源码摘要均为`fac1d02f956893c249f1833739e7a009222075d7337d070a2b517be0366202f8`，与当前726个源文件完全匹配。详见[新构建日志验证](../../dev-docs/validation/2026-10-29-browser-policy-fresh-trace-validation.md)与[结构化数值](../../dev-docs/validation/2026-10-29-browser-policy-fresh-trace-validation.metrics.json)。
- Passed: 基础browser behavior与公共完整生命周期门槛PASS；实际1440p backing保持2560×1440，实际源随后变为720p、输出1600×900。用户确认频繁切换窗口：45次FPS预算变化为初始60及22次隐藏/22次恢复，前台恒60。48次plan变更由生命周期、实际源与几何输入解释；应用层画质/renderer软恢复/连接重建动作0。同一renderer在恢复后135ms提交首帧，正常退出所有资源计数归零；trace在途≤2、排队/丢失0，重复绘制/提交失败/截图额外绘制0。
- Supply: 前台共41.111s、后台89.758s，最长连续前台9.319s。14个五段均完整3s覆盖的720p/SR前台窗口中位fps为received29.45→decoded28.76→video25.98→callback15.28→submitted13.48；同窗video→callback差中位11.94fps，callback→submitted差1.80fps。主要缺口继续在RVFC交付/合批，提交年龄采样P95约498ms；CPU提交间隔均值中位6.25ms，逐帧CPU/GPU/真实上屏成本继续补证。
- Network: RTT中位234ms/P95 281ms、jitter中位13ms/P95 32ms、媒体增量丢包7.09%、jitter buffer平均delay约324ms；PLI+3/NACK+7334/重传包+6496各自报告，浏览器NACK有效率/ReferenceChain内部字段unavailable。30个candidate pair缺失采样与约2.75s路径覆盖如实保留；本批软动作与15s健康关单效果待补证。
- Gate Boundary: 严格browser稳定性为MISSING，唯一缺项`sharedSamples`，内部failures=0；面板consumer事件0。连续前台15s段0，SR关/开同源、观测开关、主机/Windows/ROG及2h/8h完整验收待继续。
- Tooling: 报告补充分离采集间隔帧率与3s策略窗均值/覆盖，避免混用时窗；Python正负/集成fixtures扩至17项并全部通过。媒体生产源码保持本批匹配状态。
- Next Evidence: 用户手测沿用原约定；优先连续前台60–120s，打开诊断面板，固定同源/网络/显示配置完成SR关/开配对及正常退出，继续核验帧交付与提交长尾。

- Date: 2026-10-29 | Status: standard-output-latest-frame-code-verified
- User Decision: 基础 SR 档判断承接现有计算，目标采用1080p/1440p/2160p标准尺寸；本地调度优先处理最新帧，接收帧率目标保持用户设定。
- Delivered: 标准目标尺寸贯穿 ladder、plan 与实际 SR backing；真实源超过目标时成对保留原生宽高。共享 VideoFrameSource 同时持有 RVFC 与一个 rAF 计数轮询入口，按浏览器播放推进交付最新帧，并用已交付增量抵扣晚到回调；首次零计数、计数缺证后恢复、原始 RTP metadata 与旧代际取消均有边界回归。标准 WebGL 与 SR 收到新帧后立即尝试提交，累计节拍执行明确的用户显示上限，隐藏预算继续暂停本地绘制。网络适应保持既有有限码率请求，SDP 帧率、source/codec 与 sender 的帧率权限分别验证。
- Validation: `pnpm test`（44文件 / 326项）、`pnpm lint:fix`、`pnpm exec vue-tsc --noEmit`、`pnpm build`（266模块）、Python报告fixtures（18项）与`git diff --check`通过。RVFC 15fps、实际源30/60fps、60/144Hz、mediaTime=0的10s受控用例提交与供给首尾差至多2帧，重复实时处理为0；GPU采用mock。详见[标准输出与最新帧专项验证](../../dev-docs/validation/2026-10-29-browser-standard-output-and-latest-frame-validation.md)和[验证数据](../../dev-docs/validation/2026-10-29-browser-standard-output-and-latest-frame-validation.metrics.json)。
- Measurement: 当前726个源码输入路径指纹为`4f1e7668766be3f1e6329f0b732dfa0d07a0de78f2fd4a5cdff13626bca38800`；同范围62个生产文件、15,222行、6,898语句、1,297分支，较基线减少452行/141语句，runtime直属可变绑定74。此前297项、548行及`fac1d02f…`指纹保留各阶段历史含义；新日志中的非标准1600×900输出和帧交付缺口作为修复前基线。
- Handoff: 本轮两项代码修正及自动验证完成，设备运行效果继续由用户新构建手测取证。连续前台、面板共享样本、SR开关配对、平台/完整功能与2h/8h长测仍按S4验收；自动实机运行保持暂停，整体任务维持In Progress。


- Date: 2026-10-29 | Status: frame-display-diagnostics-code-verified
- User Decision: 用户授权继续补齐帧身份与显示状态日志；应用操作保持暂停，调度参数沿原值执行。
- Delivered: 现有dev `renderTelemetryObserved.mediaDiagnostics`记录原始RVFC/rAF计数、接受/拒绝原因、presentedFrames/mediaTime/RTP身份、播放计数与双向抵扣前后值；绑定当前/候选renderer的DOM样式、覆盖矩形与实际WebGL上下文属性。屏幕上屏事实继续标记unavailable，DOM布局证据独立解释。
- Budget: 惰性payload首次读取启用采集，接受/拒绝各最多2Hz、最近8条证据、最多4层祖先；250ms统计与基本资源读取沿原数值路径。off/production诊断读取0次，dev采集与其他模式媒体决策一致。
- Validation: 七文件115项定向及45文件336项全量、lint/类型/生产构建通过；九组离线供给与绘制路径复现继续通过。源码摘要与字段读法见[验证记录](../../dev-docs/validation/2026-10-29-sr-frame-diagnostics-validation.md)。历史trace保持原构建边界，实际输入/可见输出由新dev日志继续验收。
