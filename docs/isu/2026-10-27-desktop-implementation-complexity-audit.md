# 桌面端实现与复杂度分析

日期：2026-10-27。代码基线：`78e82a5a`。范围：当前 Tauri + Vue 3 + TypeScript + Rust 桌面实现。本轮产物为分析文档，产品代码保持原状。

后续方向更新：用户随后明确优先交付浏览器输入与媒体稳定版，推荐路线见[浏览器稳定版方案](/Users/guo.xu/Documents/code/games/xbxrc/docs/isu/2026-10-27-browser-stable-desktop-plan.md)。本文保留当时的实现分析与证据。

2026-10-28 串流公共主线细化：可靠性与代码精简计划见[串流公共主线 RFC](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-10-28-streaming-common-reliability-simplification.md)。计划基于当前 `02f6db40` 重新核对播放期超时、启动取消、恢复去重与清理路径，按故障回归、生命周期闭环、恢复收敛、结构精简、实机验收推进，并完整保留现有功能。

2026-10-28 浏览器方案细化：用户要求精简逻辑并完整保留功能，执行计划见[浏览器功能保留与精简 RFC](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-10-28-browser-runtime-simplification-and-stability.md)。计划承接当前协商/监督/连接事务改造，按功能基线、单点统计、动作能力、播放显示资源、策略状态、整机验收六阶段推进；USM/CAS、FSR1、麦克风、截图和动态适应均纳入保留与回归清单。当前交付为优化计划，实施与实机验收分别记录。

## 问题界定

当前复杂度主要来自三个方面：Rust 自建媒体接收与恢复闭环；浏览器运行时集中维护大量相互影响的策略状态；公共控制层与具体 runtime 之间存在重复决策和契约错位。手柄底层已经完成一轮实质性收敛，输入消费端仍有遗留的不一致。

四部分的关系可以表述为：共享的会话业务主线 + 共享的手柄服务 + 两种媒体执行实现。`webrtc-direct` 和 `rust-owned` 是同一个桌面应用的两种执行模式；当前配置默认值为 `webrtc-direct`。

本分析区分三类证据：源码直接确定的结构、源码最小复现确认的问题、需要实机进一步测量的效果。代码行数表示维护面的规模，性能与稳定性结论需要运行数据支持。

## 当前结构和约束

```text
Vue 串流页面
  -> useStreamExecution：启动、退出、页面状态
      -> Tauri StreamingService
          -> xbox-streaming：策略编译、远端会话、信令、keepalive
              -> xbox-webapi / auth / data providers
      -> runtime-host：选择运行时、连接事件、TURN 重试、诊断投影
          -> browser-runtime -> PlayerClient -> WebView RTCPeerConnection
          -> xbxengine-runtime -> Tauri XbxEngineService -> xbxengine

SDL3 物理采样 -> ohmygamepad core -> 共享 GamepadRuntimeHost
  -> Tauri 快照事件 -> Vue 几何导航
  -> Tauri 快照事件 -> Browser GamepadDriver -> JS 输入编码 -> DataChannel
  -> ArcSwap 最新状态 -> Rust 输入环 -> Rust 输入编码 -> DataChannel
```

这里有两种有价值的共享：会话协议与业务策略复用；设备发现、映射、采样和震动复用。远端 HTTP 会话、PeerConnection、媒体恢复和页面呈现各自拥有独立生命周期，保留这些领域区别有助于正确管理资源。

主要代码入口：

- [串流执行入口](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/useStreamExecution.ts:96)
- [运行时选择](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/runtime/runtime-host.ts:111)
- [公共策略编译](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/src/policy/compiler.rs:33)
- [默认执行模式](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/config/defaults.rs:76)

### 规模快照

统计 `.ts/.vue/.rs` 文件原始行数，包含测试、注释、空行及目录内暂未接线的实现；各行目录互不重叠。它们用于定位维护成本，不能直接解释运行时负载。

| 目录 | 文件数 | 原始行数 | 含义 |
| --- | ---: | ---: | --- |
| `crates/ohmygamepad` | 44 | 9,978 | 手柄采样、映射、协议、宿主、平台触觉 |
| `src/shared/gamepad` | 7 | 1,111 | 前端输入合同、owner、键盘桥 |
| `src/navigation` | 12 | 1,170 | 几何导航及输入意图 |
| `crates/xbox-streaming` | 34 | 8,412 | 公共策略与会话业务 |
| `src-tauri/src/mods/streaming` | 6 | 3,483 | Tauri 会话适配与 DTO |
| `src/player` | 40 | 10,243 | 浏览器播放器底层及测试 |
| `src/streaming` | 34 | 15,486 | 页面执行、runtime、策略、诊断及测试 |
| `crates/xbxengine` | 260 | 120,816 | Rust 媒体引擎及大量测试 |
| `src-tauri/src/mods/xbxengine` | 9 | 11,277 | Rust 引擎宿主桥、trace 投影及测试 |
| `src-tauri/src/mods/native_video` | 10 | 12,361 | 原生视频宿主、平台 presenter、调度及测试 |

值得关注的单文件：`browser-runtime.ts` 3,926 行；Rust `session/policy.rs` 3,273 行；`video_scheduling_owner.rs` 2,757 行；`api/runtime/lifecycle.rs` 2,630 行；Tauri `native_video/mod.rs` 3,400 行；`trace_projection.rs` 3,536 行。`browser-runtime` 初始化区直接声明了 134 个 `let` 闭包变量，覆盖多个行为域。

## 一、手柄支持

### 当前实际实现

物理输入采用 SDL3。`source.rs` 持有设备句柄，处理热插拔并每 4ms 读取全量状态；core runner 再进行映射、deadzone/filter、逻辑槽位采样和发布。默认 Balanced 配置为 backend/logical 250Hz、UI 60Hz，串流 push 使用 on-change。source 的实际物理轮询与上层采样配置属于不同节拍。

Windows Xbox 类输入通过 SDL 内置 XInput 驱动接入，配置关闭 RawInput、WGI、HIDAPI-Xbox；PS 等设备继续使用 SDL 的相应驱动。10 月 26 日改动已经移除旧自研 XInput 输入旁路与采样自愈逻辑，采样常驻运行。source 仍每秒进行设备重枚举，用于设备发现；这项行为应与故障推断后的设备重开分别理解。

`GamepadRuntimeHost::shared()` 使用进程级 `OnceLock`，Tauri 与 xbxengine 共享同一个服务。Rust 串流输入环每 8ms 读取 route 和 `ArcSwap` 最新逻辑状态，在 Stream 离开时补发保留原槽位的中性帧。Vue 通过 Tauri 事件接收状态；浏览器播放器同样读取原生快照，然后在 JS 侧编码 Xbox 输入包。

输入与震动的后端选择独立：Windows 当前接入 WinRT/XInput 震动 provider，macOS 当前 host 选择 SDL3 rumble。独立 `macos-gccontroller-haptics` 目录有实现内容，当前 host 装配没有引用它；`hid-dualsense` 保留占位实现。`crates/README.md` 对 Windows 触觉等能力仍使用早期占位描述。

输入路由由三项事实产生：窗口可接收业务输入、stream forwarding、UI capture。输出为 `Blocked / Ui / Stream`。Windows 按最小化状态判定窗口，macOS 额外使用可见性。Vue 的菜单、诊断、弹层通过 route controller 管理 capture 集合、异步队列和按键归位等待；几何导航负责把标准输入意图映射到 DOM 焦点。

证据入口：

- [SDL 轮询与设备更新](/Users/guo.xu/Documents/code/games/xbxrc/crates/ohmygamepad/backends/sdl3/src/source.rs:329)
- [Windows SDL 驱动约束](/Users/guo.xu/Documents/code/games/xbxrc/crates/ohmygamepad/backends/sdl3/src/source.rs:382)
- [共享 host](/Users/guo.xu/Documents/code/games/xbxrc/crates/ohmygamepad/bridge/host/src/lib.rs:57)
- [后端路由派生](/Users/guo.xu/Documents/code/games/xbxrc/crates/ohmygamepad/backends/sdl3/src/service.rs:104)
- [Rust 输入环](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack/input_loop.rs:121)
- [Vue 输入路由](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/stream/stream-input-route-controller.ts:32)

### 当前复杂度判断

SDL3 跨平台采样、逻辑槽位、映射、震动 provider、几何导航都具有清楚的职责。主要可削减部分在输入所有权的传播：后端 route、前端乐观 route、adapter 启停、`stream-ui-input-mode` DOM 事件共同控制同一业务输入。

已确认具体冲突：

1. 浏览器 `createBrowserPlayerStreamInputAdapter()` 的 activate/deactivate 都为空。
2. route controller 激活后仅在前端写 `backendRoute='stream'`。
3. 后端以 `stream_pad_forwarding=false` 继续派生 `Ui`。
4. arbiter 收到 `gamepad.runtimeSnapshot` 后覆盖前端 route；`deriveBusinessInputOwner()` 只看该 route。
5. Browser `GamepadDriver` 按 `owner==='stream'` 决定发送物理输入。

通过当前 TS 源码、实际 arbiter/controller/adapter 和模拟后端快照做最小复现，得到：`before=stream, after=ui, streamActive=true, rpcCalls=[]`。该证据确认路由契约冲突；具体设备上的触发时机需要实机验收。

相关代码：[浏览器空 adapter](/Users/guo.xu/Documents/code/games/xbxrc/src/shared/gamepad/stream-input-consumer-adapters.ts:29)、[唯一 owner 派生](/Users/guo.xu/Documents/code/games/xbxrc/src/shared/gamepad/business-input-arbiter.ts:53)、[事件覆盖](/Users/guo.xu/Documents/code/games/xbxrc/src/shared/gamepad/business-input-arbiter.ts:109)、[输入消费门槛](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/input/GamepadDriver.ts:163)。

简化方向：明确“会话输入归属”和“具体 transport 是否执行输入发送”的区别，两个 runtime 共用一套归属合同；采样继续常驻；菜单归位等待集中在一个交接入口。当前 source 4ms、MaxPrecision 标称 1000Hz backend polling、Rust input 8ms 各有含义，设置页应展示实际可兑现的节拍。

## 二、串流公共主线

### 从点击开始到首帧

1. `Stream.vue` 使用 `useStreamExecution`，加载配置并创建 startup attempt。
2. Tauri `StreamingService` 获取认证、主机/游戏能力、remote-play 上下文、TURN，映射为领域配置。
3. `xbox-streaming::policy` 将 `Config + Context` 编译为 session/runtime/negotiation/input/render plan；projection 输出给宿主和页面。
4. `SessionFlowService` 创建远端会话、等待 ready、管理状态监控和 keepalive，并提供 SDP/ICE 交换。
5. 前端获得 execution/progress，依据 `runtimeLaunchState=ready` 产生 launch spec，runtime-host 选择具体 runtime。
6. 浏览器 runtime 通过 RPC 访问公共信令；Rust runtime 通过 `TauriXbxEngineHostBridge` 访问同一会话服务。
7. runtime 的 connected/mediaReady/frame 事件经共享 host 投影给页面；关闭时执行本地 runtime 与远端 session 清理。

`xbox-streaming` 已被桌面与 `xbox-ios-bridge` 引用，公共领域层具有现实复用价值。其泛型 provider 将认证和宿主能力留在适配层，也有明确作用。

### 复杂度集中在哪里

公共主线已经把“启动计划”和“实际执行”分开。额外复杂度出现在运行期恢复：runtime-host 本地去重，RPC 请求 session 决定重连，具体 runtime 再执行；browser-runtime 有自己的卡顿识别、预算和退避；Rust engine 同样管理连接、媒体恢复与重建。一个失败事实需要跨多处通过资格判定。

同时存在多个状态投影：后端 session phase、runtimeLaunchState、runtime phase、presentation milestone、前端 sessionUiPhase/isLoading/isConnected。前端 reducer 是有价值的收敛入口，后续应把相同事实的展示字段作为派生值，并保留不同资源的生命周期边界。

已确认恢复去重的组合错误：`runtime-host.ts` 第一次 `decideRecoveryArbiter()` 产生包含当前 `factKey` 的 `nextGate`；RPC 返回允许重连后，第二次调用又用同一个 `factKey` 和该 `nextGate`，在 6 秒窗口内得到 `suppressedBy=factWindow`。browser-runtime 的 mediaHealth 重连也采用相同结构。

源码最小复现：第一次在 t=1000ms 返回 allowed=true；第二次在 t=1001ms 加入 reason 后返回 allowed=false。结果适用于这条仲裁分支；TURN 重试、显式重连、Rust 内部重建分别拥有其他入口。

证据：[共享 host 两阶段检查](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/runtime/runtime-host.ts:237)、[去重规则](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/runtime/runtime-host-policy.ts:129)、[浏览器同类调用](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/runtime/browser-runtime.ts:2987)。

建议公共主线集中管理远端会话资源、启动取消和整体退出；每个 runtime 拥有其连接恢复执行；恢复预算在真正提交动作时更新一次。TURN fallback 保留一个明确 owner，避免它与连接重建各自维护相互重叠的尝试生命周期。

## 三、浏览器方案

### 真实能力边界

这里的浏览器方案是 Tauri WebView 内的 `RTCPeerConnection`。其媒体接收、标准网络反馈、内部 jitter/decode 等由 WebView 的 WebRTC 实现承担。项目负责 Xbox 特有信令适配、SDP 调整、四个 DataChannel、手柄包编码、音频/麦克风、画面后处理和运行期监督。

`PlayerClient` 组合 SessionService、InputService、MediaService、StatsService；SessionService 装配 input/control/chat/message channels。视频 track 交给 PlaybackService，创建 video 元素，再选择 `video / webgl2 / webgl2_sr`。普通 WebGL 支持 USM/CAS，实验超分使用 FSR1 EASU/RCAS；帧回调用于呈现观测和 Xbox metadata 回传。

### 主要复杂度

`browser-runtime.ts` 同时处理：SDP/ICE 协商、连接与麦克风重建、首帧保护、媒体卡顿、3s/15s 窗口、带宽状态、画质 L0/L1/L2、恢复预算、显示降级、刷新率估算、全屏几何变化、SR 冻结/重算/失败回退、trace 和 UI stats 合并。

已经抽出的 `browser-render-policy`、`browser-runtime-profile`、`super-resolution-ladder` 提供了局部纯函数边界；runtime 仍持有大量状态并跨域编排。拆文件只能改善阅读，减少互相影响的策略和明确执行能力才能降低行为复杂度。

三个具体例子：

- **统计存在重复采集。** StatsService 每秒 snapshot、每 250ms 专项采样；stall checker 每秒再次 snapshot；面板开启后 host 每两秒 snapshot。多个入口会调用 getStats，并共享增量样本基线。统一采集时钟、向其他模块分发最新快照能减少时间窗口和数据来源组合。
- **画质策略操作本地 sender。** `applyVideoSenderPolicy()` 查找本地 video sender；接收串流通常没有本地 video track，返回 `missingVideoSender`。quality ladder 明确接受该结果并推进逻辑档位；另一条 `applySdpDownshift()` 则可回退到重新协商。因此逻辑档位、实际远端码率和协商动作应分别记录。
- **decoder reset 的动作语义过强。** `resetVideoDecoder()` 实际销毁 video/renderer，再把原 MediaStream 挂到新 video。源码能证明播放接收端重建，底层 WebRTC decoder 的重置效果需要平台证据。当前 boolean 返回值只证明重建函数执行。`requestKeyframe` 的 success rate 同样统计控制消息发送成功，解码恢复要另读接收和呈现进展。

证据：[统计时钟](/Users/guo.xu/Documents/code/games/xbxrc/src/player/app/stats/StatsService.ts:199)、[本地 sender 参数](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/webrtc/WebRtcTransport.ts:175)、[quality ladder 结果处理](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/runtime/browser-runtime.ts:2283)、[SDP 降档路径](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/runtime/browser-runtime.ts:2361)、[video 重建](/Users/guo.xu/Documents/code/games/xbxrc/src/player/app/media/PlaybackService.ts:74)、[关键帧请求计数](/Users/guo.xu/Documents/code/games/xbxrc/src/player/protocol/channels/ControlChannel.ts:61)。

另一个维护问题是公共 runtime projection 携带 `bweMode/forcedRembKbps/videoPipeline` 等参数，而浏览器 PlayerClient 装配主要消费 SDP 码率、分辨率、codec 和输入/音频参数。浏览器闭环内部参数应按实际支持能力表达，防止公共配置制造“参数已应用”的错觉。

建议浏览器层保留四个明确职责：连接协商、播放与显示、单一统计采集、有限恢复动作。SR/锐化作为显示模块的可选能力。先把动作与效果分开，再决定哪些画质/恢复启发式值得保留。

## 四、Rust 方案

### 实际执行链

```text
Tauri XbxEngineService / 16ms runtime tick
  -> XbxEngineRuntime：启动、协商、连接生命周期、宿主接口
  -> XbxActiveMediaStack
      -> rtc 0.9 sans-io PeerConnection + 项目 I/O / TURN / feedback glue
      -> RTP / RTX ingress
      -> packet buffer / NACK / H264 bootstrap / RTP 组帧
      -> ReceiveRecoveryLedger / ReferenceChain / insert/decode gate
      -> VideoIngress / DecodeIngressAdapter
      -> FFmpeg decode actor
      -> decoded mailbox -> pacer -> renderer actor
      -> Tauri NativeVideoRegistry -> presenter mailbox -> 原生上屏

音频：接收 -> Opus -> CPAL；输入：共享 GamepadRuntimeHost -> DataChannel
```

`rtc` 提供 WebRTC 协议基础，应用负责驱动 I/O 和大量媒体接收行为。`ReceiveEngine` 已把 PacketBuffer、NackRequester、KeyframeRequester、H264BootstrapTracker、RtpFrameAssembler 聚合到一起；项目承担的是媒体引擎级别的维护面。

平台执行如下：Windows 优先 FFmpeg D3D11VA，解码纹理走 D3D11 native presenter；macOS 优先 FFmpeg VideoToolbox，CVPixelBuffer 走原生 layer presenter；软件解码的 CPU surface 走 WGPU。原生纹理直出和 CPU 上传分支有清晰的平台依据。

当前 viewport 固定解析到 `MainWindow`，与主窗口 Web UI 融合。历史双窗口 RFC 提供了探索背景，当前 active 代码使用主窗口宿主。此实现运行在 Tauri 进程内；文档和注释中的 sidecar/winit 旧名称需要按现有装配校准。

证据：[媒体栈装配](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack.rs:117)、[接收引擎](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/receive/engine.rs:16)、[解码后端选择](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/backend.rs:197)、[presenter 选择](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/native_video/native_video_policy.rs:6)、[主窗口归属](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/native_video/mod.rs:487)。

### 复杂度最大的部分

**恢复事实存在多层表达和消费。** ReceiveRecoveryLedger、ReferenceChain、VideoSchedulingOwner、RecoveryCoordinator、session policy、runtime lifecycle、host display recovery 都在回答某一部分“当前是否恢复/是否可继续”。它们涉及不同领域，同时通过共享 stats、字符串标签和时间戳交叉读取。每个新异常容易要求给多处判定增加例外。

**共享 stats 已承担部分控制状态。** `XbxEngineMediaRuntimeStats` 有 298 个公开字段，包含计数、时间、连接状态、recovery generation、decoder debt、owner 和诊断标签。RecoveryCoordinator 通过 RuntimeStatsSink 读取它并作决策，多个 `*_from_stats` 合同函数用于控制。外部 `XbxEngineStatsDto` 还有 264 个公开字段，再经 Tauri trace、JS stats、diagnostics、面板投影传播。

**解码后存在多级候选和调度。** decoded mailbox、pacer、renderer、host mailbox 分别处理 latest-only、过期、供给和上屏机会。保留有界候选对于低延迟有价值；同一帧在多层被独立解释 deadline、恢复价值和 generation 会增加推理负担。适合把最终显示时机/丢弃决策集中到明确 owner，其余环节执行有界交接。

**协议反馈与自建接收逻辑需要持续核对。** builder 同时装配 rtc NACK interceptor，receive 自有 NACK 请求和 feedback arbiter。源码确认两类机制存在；重复反馈是否实际发生取决于协议执行路径和抓包证据，本轮将其列为后续边界审计项。

证据：[共享运行状态](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/backend.rs:513)、[恢复协调器读写 stats](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs:99)、[session policy 的跨域输入](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs:1)、[视频 owner 状态](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs:25)。

### 历史决策如何影响当前判断

- 4 月多轮工作引入/收敛 latest-only、恢复锚点和显示供给控制。
- 5 月 19 日库重评区分 transport/API 替换与完整媒体接收闭环替换；5 月 20 日工作明确保留 rtc，并把接收逻辑收向 receiver-local ownership。
- 6 月 2 日 RFC 明确 receive/decode 闭合媒体恢复、display 独立、session 只处理连接与 receive terminal、projection 用于观测。
- 6 月后续任务持续处理 stale anchor、host retained frame、no-pending reset、误重连等交叉状态问题。
- 7 月加入 H264 profile 会话内 fallback，进一步体现互操作性与恢复策略之间的联系。

这些历史支持继续落实职责边界。当前代码还有显著的 stats 控制耦合；后续简化应逐个恢复域收回权威事实与动作权限。更换 transport 库能够改变协议胶水成本，完整 receiver/decode/present 的维护责任需要单独评估。

参考：[receive 重构 RFC](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-05-20-receive-side-loop-rearchitecture.md)、[恢复边界 RFC](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-06-02-webrtc-recovery-boundary-redesign.md)、[H264 fallback RFC](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-07-06-rust-h264-browser-profile-fallback.md)。

## 可选方向

| 方向 | 收益 | 成本与适用范围 |
| --- | --- | --- |
| 按现有边界删减重复决策 | 保留已积累的协议和平台能力，直接修复组合冲突 | 需要明确 owner 并覆盖交接场景；适合现在开始 |
| 浏览器建立最小稳定基线 | 便于衡量应用策略和显示增强的实际收益 | WebView 控制能力有边界；作为测量基线具有价值 |
| 重新评估完整原生媒体栈 | 有机会转移部分自建接收/恢复维护责任 | 涉及 FFI、分发和技术边界，属于独立 RFC 决策 |

推荐当前采用第一项，并用第二项提供对照。当前仓库明确保留 Rust transport 主线，技术路线调整应在独立方案中论证。

## 推荐收敛次序

1. 修复并锁定输入归属和两阶段恢复仲裁的组合契约，形成“事实 -> 一次决策 -> 一次执行 -> 结果”的明确路径。
2. 浏览器统一 stats 采集，核准 sender policy、video 重建、关键帧请求的能力和效果语义，删减无法兑现的策略状态。
3. 公共 session 专注业务会话；runtime 负责自身媒体执行与恢复；页面消费少量稳定事件和派生状态。
4. Rust 按 receive/decoder/presentation/connection 四个领域逐步收回状态和动作权限，诊断从权威状态单向产生。
5. 清理已被后续实现取代的任务状态与 README，形成一份 current architecture 索引。

## 待补证据与后续决策

目前源码可以确认结构和上述组合问题。实机输入稳定性、浏览器重建 video 对解码器的影响、两种 runtime 的延迟与能耗、Windows/macOS 的平台差异仍需匹配当前构建的样本。

后续串流验收应同时覆盖：

| 维度 | 要回答的问题 |
| --- | --- |
| 网络健康 | ICE/TURN 路径、RTT、loss、TWCC、反馈目标是否正常 |
| 恢复链健康 | NACK/PLI/FIR 到 usable IDR、decoded anchor、generation 的闭环是否完成 |
| media supply | RTP/AU/decode/renderer/host 各阶段是否持续得到输入 |
| steady supply | 稳态出帧与内容帧率、显示刷新是否匹配 |
| submit/present 长尾 | packet-to-decode、decode-to-submit、submit-to-present 的分位数与最长停顿 |
| 帧供给 delta | AU、decode emit、pacer submit、host enqueue、present 的增量在哪里分叉 |

这些指标用于验证后续改动的收益。本轮证据支持复杂度和契约分析，当前性能排名保持待测。

## 候选任务

| 任务 | 分类 | 验收重点 |
| --- | --- | --- |
| 修正两阶段恢复仲裁对同一事实的自抑制 | simple | failed/closed/mediaHealth 在一次完整流程中正确消费预算 |
| 统一两种 runtime 的输入归属合同 | complex | 启动、后端快照、菜单 capture、neutral release、最小化、退出、多槽输入 |
| 浏览器 stats 单点采集与动作能力校准 | complex | 面板开关保持相同采样语义；每项动作标清实际执行与观测效果 |
| 公共 session/runtime 恢复权限收敛 | complex | 业务重建、transport 重连、TURN fallback、媒体局部修复职责明确 |
| Rust 控制事实和诊断投影分离 | complex | receive/decode/presentation/connection 单向事实合同与全维 trace 验收 |
| 当前架构文档与被替代任务状态校准 | simple | README、代码入口、任务状态一致 |

以上为候选开发任务，本轮停留在分析阶段。复杂实现按 task-run 进入 RFC 流程。

## 本轮验证与完成核对

- 阅读当前工作树的入口、运行时装配、手柄采样与消费、会话流、浏览器媒体适配、Rust 接收/解码/显示，以及相关近期 RFC/任务。
- 使用 TypeScript transpile + VM 执行当前输入 arbiter/controller/adapter；模拟后端 Ui 快照，确认 owner 从 Stream 回到 Ui。
- 执行当前恢复仲裁函数，按两个调用点的时序传入 nextGate，确认第二次被 factWindow 抑制。
- 执行 `pnpm exec vitest run src/pages/stream/stream-input-route-controller.test.ts src/streaming/runtime/runtime-host-policy.test.ts src/streaming/runtime/browser-runtime.test.ts`：3 个文件、39 项测试通过。现有单元测试与上述组合问题可以同时成立。
- 手柄支持：已覆盖采样、映射、路由、两种消费者、震动、导航及当前冲突。
- 串流公共主线：已覆盖配置/能力编译、远端会话、信令、runtime 启动、恢复与关闭边界。
- 浏览器方案：已覆盖 WebRTC、协议通道、媒体元素、显示增强、统计、恢复及实际能力限制。
- Rust 方案：已覆盖协议基础、自建接收闭环、解码、调度、平台上屏、状态与诊断耦合。

建议从输入归属与恢复仲裁两个已有复现的交接点开始收敛，再以浏览器单点统计和 Rust 分域状态合同推进后续减法。
