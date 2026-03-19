# webrtc-rs 串流分层开发指导

## 1. 目标与范围

本文定义当前 `webrtc-rs` 主线的视频串流分层模型，用于统一后续开发时的模块边界、交互方式和架构判断口径。

本文只解决三类问题：

1. 我们当前的视频主链应如何分层。
2. 每层的目标、输入输出 contract、执行模型应该如何定义。
3. Moonlight 哪些设计可以直接借鉴，哪些必须按当前主线改写。

本文只覆盖视频主链：

- `XbxNegotiationBackend -> XbxActiveMediaStack`
- `transport/webrtc -> transport/adapter -> media/video`
- `diagnostics/stats -> runtime_state`

横切但不单独成层的部分：

- `audio`
- `control/data-channel`
- `input`

本文的非目标：

1. 不定义 `runtime.tick()`、启停、重连、host bridge 这类会话编排。
2. 不把 Moonlight 的 RTSP/RTP/FEC 栈当成当前系统事实。
3. 不讨论具体参数调优值，例如 NACK 窗口、队列长度、阈值毫秒数。

## 2. 两条总原则

### 2.1 以当前主线为准

所有分层都必须贴合当前实现，不允许为了“像 Moonlight”而重写问题表述。

当前主线的组合根是：

- [`crates/xbxengine/core/src/transport/webrtc/backend.rs`](../crates/xbxengine/core/src/transport/webrtc/backend.rs)
- [`crates/xbxengine/core/src/transport/webrtc/stack.rs`](../crates/xbxengine/core/src/transport/webrtc/stack.rs)

它们明确了当前事实：

1. `XbxNegotiationBackend` 只是 backend 外壳。
2. `XbxActiveMediaStack` 负责组装 transport、data-channel、video、control。
3. 视频主链当前是 `adapter/ingress/decode/pacer/render`。

### 2.2 借鉴 Moonlight 的控制哲学，不照搬其网络结构

Moonlight 可以直接借鉴的是：

1. 网络、解码、渲染必须执行隔离。
2. 使用小队列、受控丢帧和快速恢复保持低时延。
3. 恢复动作由观测驱动，而不是等下游彻底堵死。

Moonlight 不能直接照搬的是：

1. RTSP/RTP/FEC/MTU/NAT64 这类网络 ownership。
2. SDL 主循环式的顶层结构描述。
3. 它的 depacketizer / decode-unit queue 具体对象模型。

## 3. 六层总览

| 层 | 一句话职责 | 一句话目标 |
| --- | --- | --- |
| 网络接入层 | 建立并维持 WebRTC 媒体接入，产出 transport 事实与连续媒体输入 | 把不稳定 transport 尽量变成可继续处理的媒体来源 |
| 组帧与准入层 | 把 transport 输入整理成允许送解的编码帧 | 把“收到了的数据”变成“可安全解码的数据” |
| 解码层 | 把已准入编码帧转换成可呈现表面 | 稳定产出可渲染表面，并快速收敛坏参考链/坏数据 |
| 渲染呈现层 | 按 host timing 将解码结果低延迟呈现 | 用小队列和主动丢帧换低排队延迟 |
| 观测汇总与投影层 | 汇总跨层事实并投影给 runtime trace/UI | 把分散局部事实变成稳定可解释系统状态 |
| 实时调度策略层 | 基于观测统一跨层恢复与实时决策语义 | 决定何时丢、等、拉 keyframe、reset、cooldown |

约束：

- 第六层是“实时策略语义”，不是 `runtime.tick()` 的会话编排。
- 六层围绕视频主链定义，`audio / control / input` 仅作为横切约束。

## 4. 六层详细定义

### 4.1 网络接入层

### 职责边界

负责：

1. `RTCPeerConnection / transceiver / interceptor / stats` 这一层的媒体接入。
2. track 挂载、transport state、RTT/loss/TWCC/path 等事实采集。
3. control/data-channel 可用性事实的暴露。

不负责：

1. 把样本直接定义成“可安全解码帧”。
2. 解码恢复与渲染背压。
3. 会话级启停、重连编排。

### 核心目标

把不稳定的 WebRTC transport 尽量变成：

1. 可持续消费的媒体样本来源。
2. 可供上层决策的 transport 事实。
3. 可触发恢复动作的 feedback 上下文。

### 输入 contract

上游输入包括：

1. SDP / ICE / PeerConnection 生命周期。
2. remote video/audio track。
3. RTCP/TWCC/NACK 等反馈信号。
4. data-channel/control channel 状态。

### 输出 contract

本层对下游承诺输出：

1. 可消费的媒体样本来源。
2. transport path、RTT、loss、TWCC、bitrate、track state。
3. channel availability 等恢复可执行性事实。

本层不承诺输出“已安全可解码帧”。

### 执行/交互模型

1. 本层不能等待 decode/render 才继续接收媒体。
2. 它只能向下游移交媒体样本与 transport 事实，不能被渲染卡死。
3. 对当前主线而言，这一层主要运行在 `webrtc-rs` callback/task + stats loop 语义上，而不是 Moonlight 的裸 UDP/RTP 线程。

### Moonlight 可直接借鉴

1. 网络接入必须持续暴露 transport 诊断事实，而不是只有“连上/没连上”。
2. feedback 与恢复触发要尽量靠近 transport 事实，而不是晚到 decoder/render 之后才补救。
3. 网络层不得被下游同步阻塞。

### Moonlight 必须改写

1. Moonlight 的 RTP/FEC/MTU ownership 不属于当前层现状。
2. 当前主线没有自管 UDP/RTP receive queue 这一层对象模型。
3. 这里必须用 WebRTC transport 接入来描述，而不是“裸 RTP 网络层”。

### 当前主线落点

- `transport/webrtc/*`
- [`crates/xbxengine/core/src/transport/webrtc/backend.rs`](../crates/xbxengine/core/src/transport/webrtc/backend.rs)
- [`crates/xbxengine/core/src/transport/webrtc/stack.rs`](../crates/xbxengine/core/src/transport/webrtc/stack.rs)

### 4.2 组帧与准入层

### 职责边界

负责：

1. sample builder / H.264 inspection / ingress admission。
2. 把 transport 样本提升为允许送解的 `EncodedFrame`。
3. 在 decode 前处理 bootstrap、坏 AU、等待 keyframe、reconfigure。

不负责：

1. 具体硬件/软件解码执行。
2. 最终呈现。
3. 会话级重连。

### 核心目标

把“收到了的数据”变成“可安全解码的数据”，避免 decoder 被脏数据、坏参考链、错误 backlog 长期污染。

### 输入 contract

1. packet/sample 级媒体输入。
2. packet gap/loss、NACK/recovery signal 等 transport 事实。
3. 当前流的 codec/参数集变化信息。

### 输出 contract

1. 已完成准入判定的 `EncodedFrame`。
2. `WaitKeyframe / Reconfigure / DropLate / DropBacklog` 等准入结论。
3. recovery signal，例如 sample loss、await recovery keyframe。

### 执行/交互模型

1. 本层是 decode 前闸门。
2. 网络层交来的数据必须先在这里完成组装与裁决，再送给 decode。
3. backlog、bad AU、bootstrap gating 应优先在这里解决，而不是让 decoder 自己清洗输入。

### Moonlight 可直接借鉴

1. depacketizer 之后应有明确的 decode admission 阶段。
2. 等待 IDR / keyframe、坏参考链前拦截、必要时清队列并请求刷新，这些语义可以直接借鉴。
3. 低价值旧帧不应继续占用进入 decoder 的资格。

### Moonlight 必须改写

1. Moonlight 的 depacketizer + decode unit queue 不能直接映射到当前类型设计。
2. 当前主线已经拆成 `adapter` 与 `ingress` 两段，因此这里要写成“组帧与准入层”。
3. 当前层不能假设拥有 Moonlight 那种 packet/FEC 级完整控制权。

### 当前主线落点

- [`crates/xbxengine/core/src/transport/adapter/source.rs`](../crates/xbxengine/core/src/transport/adapter/source.rs)
- [`crates/xbxengine/core/src/media/video/ingress/scheduler.rs`](../crates/xbxengine/core/src/media/video/ingress/scheduler.rs)

### 4.3 解码层

### 职责边界

负责：

1. decoder session 生命周期。
2. 把已准入 `EncodedFrame` 转换为可呈现表面。
3. 在坏数据、坏参考链、session 污染时执行局部自愈。

不负责：

1. transport 恢复 ownership。
2. 呈现时钟与最终 present。
3. 会话层状态机。

### 核心目标

稳定产出可呈现表面，并在出现坏数据时快速收敛，不让 decoder 长期吞脏输入。

### 输入 contract

1. 已经通过准入的 `EncodedFrame`。
2. 局部 `flush` / `reset` 请求。
3. 新的 recovery keyframe。

### 输出 contract

1. 可呈现 `DecodedFrame` / render surface。
2. 解码失败与 decoder health 状态。
3. reset 后等待新的 recovery keyframe 的语义状态。

### 执行/交互模型

1. 解码必须是独立执行单元，与网络接入层隔离。
2. 当前主线现状是 actor + bounded push mailbox，不是 Moonlight 式 pull decoder。
3. 后续对齐方向可以是更强的 budget-aware handoff，但不能把未来方向误写成“现在已经是 pull”。

### Moonlight 可直接借鉴

1. decoder 与网络接入解耦。
2. 解码失败后快速拉 keyframe / reset。
3. reset 后等待新的 recovery keyframe 再恢复正常送解。

### Moonlight 必须改写

1. `LiWaitForNextVideoFrame()` 对应的是 Moonlight 的 pull 输入模型，不是当前主线现状。
2. 当前主线应先保留 actor 结构，只对齐“失败即自愈”和“bootstrap gating”语义。
3. 不能为了模仿 Moonlight 而让网络层直接控制 decoder 生命周期。

### 当前主线落点

- [`crates/xbxengine/core/src/media/video/decode/actor.rs`](../crates/xbxengine/core/src/media/video/decode/actor.rs)
- `media/video/decode/*`

### 4.4 渲染呈现层

### 职责边界

负责：

1. pacer、renderer、host timing 对齐。
2. 最终 present 与呈现统计。

不负责：

1. transport 恢复。
2. 组帧准入。
3. 会话调度。

### 核心目标

把可呈现表面转成低排队延迟的最终画面，优先保持“最新可见帧”而不是忠实积压所有帧。

### 输入 contract

1. `DecodedFrame` / render surface。
2. host display interval。
3. host frame age budget。

### 输出 contract

1. 最终可见画面。
2. submit/drop/present/overwrite 等呈现统计。

本层不承诺 FIFO 忠实回放。

### 执行/交互模型

1. decode 与 render 之间只能通过 bounded queue / pacing 语义交互。
2. render stall 不允许回压到网络层。
3. 小队列、主动 drop、按 host timing 追实时是本层常态，不是异常。

### Moonlight 可直接借鉴

1. pacer 是低延迟链路的核心。
2. render queue 必须很小，必要时主动丢帧。
3. 显示链路不能反向拖死解码与网络。

### Moonlight 必须改写

1. Moonlight 的 SDL/vsync/device 细节不应直接搬到当前实现。
2. 可以借鉴的是“pacer + bounded queue + controlled drop”的抽象语义。
3. 当前层要服从 Tauri/宿主 present 方式，而不是复刻 Moonlight renderer 拓扑。

### 当前主线落点

- [`crates/xbxengine/core/src/media/video/pacer/actor.rs`](../crates/xbxengine/core/src/media/video/pacer/actor.rs)
- [`crates/xbxengine/core/src/media/video/render/actor.rs`](../crates/xbxengine/core/src/media/video/render/actor.rs)
- `media/video/render/*`

### 4.5 观测汇总与投影层

### 职责边界

负责：

1. 把 transport/media/runtime 各层事实汇总为系统状态。
2. 输出 health、diagnosis、summary、trace snapshot、UI 投影。

不负责：

1. 决定具体媒体调度动作。
2. 执行网络/解码/渲染热路径逻辑。

### 核心目标

把分散局部事实变成稳定、可解释、可投影的系统状态，让上层知道“卡在哪层、为什么、系统正在做什么”。

### 输入 contract

1. transport/media/runtime 各层 stats 与 observation。
2. recovery 动作与结果。
3. 宿主侧 host timing / present 统计。

### 输出 contract

1. health、diagnosis、summary、issue chain。
2. trace snapshot 与 UI 可消费投影。
3. 稳定的运行状态摘要，而不是重复局部日志。

### 执行/交互模型

1. 本层必须是旁路消费，不得阻塞热路径。
2. 聚合与投影可以有节流，但不能丢失关键因果链。
3. 宿主投影层不应该反向持有媒体调度 ownership。

### Moonlight 可直接借鉴

1. 观测面是系统能力的一部分，不是可有可无的日志。
2. 网络、解码、渲染、drop、latency 必须能串成因果链。

### Moonlight 必须改写

1. Moonlight 更偏局部统计与日志；当前主线已经有统一聚合与宿主投影，因此必须写成“汇总与投影层”。
2. 不能退化成各线程自己打日志、靠人工拼图。

### 当前主线落点

- [`crates/xbxengine/core/src/diagnostics/stats.rs`](../crates/xbxengine/core/src/diagnostics/stats.rs)
- [`src-tauri/src/mods/xbxengine/runtime_state.rs`](../src-tauri/src/mods/xbxengine/runtime_state.rs)

### 4.6 实时调度策略层

### 职责边界

负责：

1. 定义跨网络/组帧/解码/渲染的实时决策语义。
2. 根据健康状态、deadline、queue depth、loss/recovery 事实生成动作。

不负责：

1. 会话启停、重连、host bridge 这类外层编排。
2. 吞并各层内部算法实现。

### 核心目标

统一决定：

1. 什么时候等。
2. 什么时候丢。
3. 什么时候请求 keyframe/IDR。
4. 什么时候 decoder reset。
5. 什么时候 retry、cooldown、escalation。

### 输入 contract

1. 各层 health、deadline、queue depth。
2. transport loss/recovery 事实。
3. decoder/render 状态与 host 预算。

### 输出 contract

1. `drop / wait / keyframe / reset / retry / cooldown / escalation` 决策。
2. 决策原因与节流状态。

本层只输出策略决策，不直接持有媒体数据。

### 执行/交互模型

1. 本层是跨层策略语义，不是单独的“大循环线程”。
2. 当前主线由 media supervisor、recovery coordinator 等组合承载这层语义。
3. 本层消费事实并发出决策，不直接替代 transport、decode、render 的局部实现。

### Moonlight 可直接借鉴

1. 恢复优先级与升级链。
2. `drop-vs-wait` 的实时权衡。
3. 避免长期积压和重复触发恢复。

### Moonlight 必须改写

1. 不能把 Moonlight 的 SDL 主循环式调度写成当前结构事实。
2. 当前主线应该表达为“跨层策略语义 + 分散执行点”，不是“一根中央主线程”。

### 当前主线落点

- `transport/webrtc/recovery/*`
- [`crates/xbxengine/core/src/transport/webrtc/stack.rs`](../crates/xbxengine/core/src/transport/webrtc/stack.rs)

## 5. 线程模型与层交互

这一节定义我们与 Moonlight 对照时最容易被误读的点。

### 5.1 Moonlight 的关键隔离模式

从 Moonlight 代码可确认三件事：

1. 网络输入先进入 depacketizer / decode-unit queue，而不是直接把网络线程绑到 decoder。
2. decoder 通过 `LiWaitForNextVideoFrame()` 等待下一帧，本质是 pull handoff。
3. pacer/render 通过小队列与 decode 隔离，`PACER_MAX_OUTSTANDING_FRAMES` 明确限制积压上限。

关键代码证据：

- `session.cpp` 中当 decoder 支持 `CAPABILITY_PULL_RENDERER` 时，不再提供 push callback。
- `VideoStream.c` 的 `VideoDecoderThreadProc()` 使用 `LiWaitForNextVideoFrame()` 阻塞等待。
- `ffmpeg.cpp` 的 decoder thread 在没有新输入时同样阻塞等待下一帧。
- `pacer.h` 通过 `PACER_MAX_OUTSTANDING_FRAMES (3 + 1 + 1)` 显式封顶。

### 5.2 当前主线的对应表达

当前主线不是同一套线程拓扑，但应保持同样的隔离目标：

1. 网络接入层不能等待 decode/render。
2. 组帧与准入层是 decode 前闸门。
3. decode 是独立执行单元，但现状仍是 actor + bounded push mailbox。
4. pacer/render 继续使用小队列和主动 drop。
5. observability 必须旁路，不阻塞热路径。

### 5.3 现状与对齐方向

必须分开写：

现状：

1. `decode/pacer/render` 已经是独立线程。
2. `decode -> pacer -> renderer` 当前仍是 bounded push。
3. `RendererActorHandle` 队列上限为 `1`，`PacerActorHandle` 和 `DecodeActorHandle` 队列上限为 `2`。

对齐方向：

1. 进一步强调 budget-aware handoff。
2. 把更有价值的 backlog 控制前移到组帧与准入层。
3. 必要时向更强的 pull 语义靠近，但不能把这件事表述成“当前已经 pull 化”。

### 5.4 层间交互规则

1. `网络接入层 -> 组帧与准入层`
   - 只交媒体样本与 transport 事实。
   - 不等待 decoder/render。
2. `组帧与准入层 -> 解码层`
   - 只交允许送解的编码帧。
   - backlog、坏 AU、等待 keyframe 必须优先在这一步解决。
3. `解码层 -> 渲染呈现层`
   - 只能通过 bounded queue / pacing handoff。
   - render stall 不能直接向上回压。
4. `观测汇总与投影层`
   - 始终旁路消费，不得成为同步依赖。
5. `实时调度策略层`
   - 不直接持有媒体数据，只消费事实并发出决策。

## 6. Moonlight 对照规则

为了避免后续实现者误把“借鉴”理解成“复制”，这里固定使用以下判断口径。

### 6.1 可直接借鉴

1. `bounded queue`
2. `controlled drop`
3. `quick recovery / keyframe / reset escalation`
4. 观测驱动调度
5. 网络、解码、渲染三段执行隔离

### 6.2 必须改写

1. RTSP/RTP/FEC/MTU/NAT64 等 Moonlight 网络 ownership
2. SDL 主循环式顶层调度表述
3. depacketizer / decode-unit queue 的具体对象模型
4. Moonlight 的 renderer/vsync/device 细节

### 6.3 判断准则

能借鉴的是：

1. 目标
2. 语义
3. 交互原则
4. 背压隔离方式

必须改写的是：

1. 协议结构
2. 线程拓扑命名
3. 当前代码边界
4. ownership 归属

## 7. 横切约束

### 7.1 audio

`audio` 不单独成层，但会影响：

1. stall diagnosis
2. health 判断

### 7.2 control/data-channel

`control/data-channel` 不单独成层，但会影响：

1. recovery action 是否可执行
2. 网络接入层暴露的 channel availability
3. 实时调度策略层的动作落地能力

### 7.3 会话编排外层

以下内容不属于六层正文：

1. `runtime.tick()`
2. 启停、重连
3. host bridge 协商
4. runtime 与应用态编排

它们应作为会话编排外层单独讨论。

## 8. 实施使用规则

后续开发时，默认按下面规则使用本文。

1. 新逻辑先判断属于哪一层，再决定模块落点。
2. 涉及跨层状态与恢复动作时，优先放到“实时调度策略层”的语义里表达。
3. 新观测先进入“观测汇总与投影层”，再决定是否投影到宿主/UI。
4. 任何让下游阻塞回压上游热路径的设计，都应视为反模式。
5. 任何把 Moonlight 的网络结构直接写成当前事实的设计，都应视为反模式。
6. 任何把未来方向写成当前现状的设计，都应视为反模式。

## 9. 证据基线

### 9.1 当前主线

- [`crates/xbxengine/core/src/transport/webrtc/backend.rs`](../crates/xbxengine/core/src/transport/webrtc/backend.rs)
- [`crates/xbxengine/core/src/transport/webrtc/stack.rs`](../crates/xbxengine/core/src/transport/webrtc/stack.rs)
- [`crates/xbxengine/core/src/transport/adapter/source.rs`](../crates/xbxengine/core/src/transport/adapter/source.rs)
- [`crates/xbxengine/core/src/media/video/ingress/scheduler.rs`](../crates/xbxengine/core/src/media/video/ingress/scheduler.rs)
- [`crates/xbxengine/core/src/media/video/decode/actor.rs`](../crates/xbxengine/core/src/media/video/decode/actor.rs)
- [`crates/xbxengine/core/src/media/video/pacer/actor.rs`](../crates/xbxengine/core/src/media/video/pacer/actor.rs)
- [`crates/xbxengine/core/src/media/video/render/actor.rs`](../crates/xbxengine/core/src/media/video/render/actor.rs)
- [`crates/xbxengine/core/src/diagnostics/stats.rs`](../crates/xbxengine/core/src/diagnostics/stats.rs)
- [`src-tauri/src/mods/xbxengine/runtime_state.rs`](../src-tauri/src/mods/xbxengine/runtime_state.rs)

### 9.2 Moonlight 参考代码

- `/Users/guo.xu/Documents/code/games/moonlight-qt/app/streaming/session.cpp`
- `/Users/guo.xu/Documents/code/games/moonlight-qt/moonlight-common-c/moonlight-common-c/src/VideoStream.c`
- `/Users/guo.xu/Documents/code/games/moonlight-qt/app/streaming/video/ffmpeg.cpp`
- `/Users/guo.xu/Documents/code/games/moonlight-qt/app/streaming/video/ffmpeg-renderers/pacer/pacer.h`

## 10. 一句话落地标准

如果一个新设计不能同时回答下面四个问题，就说明它还没有对齐本文：

1. 它属于哪一层。
2. 它解决哪类目标问题。
3. 它的输入输出 contract 是什么。
4. 它会不会让下游反向阻塞上游热路径。
