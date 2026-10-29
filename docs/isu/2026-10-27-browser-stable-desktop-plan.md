# 桌面稳定版：浏览器输入、WebRTC 直出与 WebGL 后处理方案

日期：2026-10-27。状态：方案，待进入实施任务。代码基线：`78e82a5a`。

2026-10-28 后续方向：用户明确要求浏览器方案精简逻辑、完整保留现有功能并保障稳定性，具体计划见[浏览器功能保留与精简 RFC](../rfcs/2026-10-28-browser-runtime-simplification-and-stability.md)。该计划基于当前统一输入和公共可靠性改造，保留 USM/CAS、FSR1、麦克风、截图与动态适应；本文的首版功能范围、策略移出及发行依赖隔离作为历史探索选项保存，当前优化按新 RFC 的功能保留合同推进。

## 1. 问题界定

推荐把浏览器路径确立为桌面稳定发行的唯一输入和媒体路径：Tauri + Vue 保留产品形态，Rust 继续承担认证、远端会话、信令与系统集成，WebView 承担输入采样、WebRTC 收发、音视频播放和可选 WebGL 后处理。Rust 媒体引擎保留为默认关闭的实验能力，后续以稳定版为参照验证收益。

手柄架构采用统一 GamepadBackend 抽象，Browser 与 Rust 是两种独立实现；稳定发行默认装配 Browser，输入后端选择与媒体运行时选择分别处理。2026-10-27 后续细化明确：Windows 当前默认“后台保持输入”，公共接口与本机工作继续推进，Windows/ROG 兼容性单独待验。

稳定性的交付依据是明确的支持矩阵、有界的故障处理、可取消的会话生命周期和实机验收。浏览器接管媒体处理可以显著收缩项目自建接收、解码和显示链的维护范围；WebView 版本、GPU 驱动、设备映射和前后台行为仍需逐项验证。

本方案承接[实现复杂度分析](/Users/guo.xu/Documents/code/games/xbxrc/docs/isu/2026-10-27-desktop-implementation-complexity-audit.md)，根据用户随后提出的稳定版方向，更新其中“共享原生输入、继续 Rust 媒体主线”的推荐。当前交付只包含方案文档。

## 2. 当前约束与可复用资产

| 当前事实 | 对实施的影响 |
| --- | --- |
| 配置默认已是 `webrtc-direct`，浏览器播放器与 Xbox DataChannel 已存在 | 沿当前代码收敛，复用经过互通的协议与业务逻辑 |
| 浏览器手柄仍消费 SDL 原生快照 | 浏览器输入迁移须覆盖导航、串流、键盘桥、震动和设置页 |
| `browser-runtime.ts` 约 3,926 行，混合连接、恢复、画质、显示、统计 | 通过削减策略和收回状态所有权降低行为复杂度 |
| Rust `SessionFlowService` 已管理远端会话与 SDP/ICE，且由其他平台复用 | 保留共享业务服务，稳定版只投影实际支持的浏览器参数 |
| Tauri 默认依赖原生引擎、SDL、图形库，构建脚本和安装包包含 FFmpeg | 发布改造须同时覆盖依赖、启动装配、脚本、资源和 CI |

关键依据：

- [浏览器运行时](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/runtime/browser-runtime.ts)、[播放器组合入口](/Users/guo.xu/Documents/code/games/xbxrc/src/player/api/PlayerClient.ts)、[会话服务](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/src/session/flow.rs:330)。
- [应用输入接线](/Users/guo.xu/Documents/code/games/xbxrc/src/main.ts:45)、[导航原生快照消费](/Users/guo.xu/Documents/code/games/xbxrc/src/navigation/core/gamepad-listener.ts:73)、[浏览器输入驱动](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/input/GamepadDriver.ts)。
- [原生依赖](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/Cargo.toml:76)、[AppState 强制原生字段](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/shell/state.rs:21)、[安装资源](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/tauri.conf.json:44)、[构建包装脚本](/Users/guo.xu/Documents/code/games/xbxrc/scripts/with-ffmpeg-sdk.mjs)。

历史工作继续提供验收案例：[FSE 输入稳定化](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-10-26-gamepad-fse-input-stabilization.md)、[恢复领域边界](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-06-02-webrtc-recovery-boundary-redesign.md)、[Rust H264 浏览器 profile 回退](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-07-06-rust-h264-browser-profile-fallback.md)。它们分别约束触屏/焦点交接、故障处理权限和协商互通；当前稳定版的输入与媒体实现按本方案调整。

## 3. 选项与推荐

| 选项 | 收益 | 代价与适用性 |
| --- | --- | --- |
| **浏览器输入 + 浏览器媒体，单一稳定发行路径** | 输入与媒体集中在 WebView；可移出原生采样、FFmpeg、原生呈现依赖 | 接受浏览器设备与触觉能力范围；推荐作为当前交付方向 |
| SDL 输入 + 浏览器媒体 | 保留原生设备映射、采样和高级触觉 | 持续承担跨进程输入归属、原生装配与 FSE 路由成本 |
| 双媒体运行时继续共同发布 | 保留运行时选择与原生调试入口 | 每项输入、生命周期、诊断与构建修改都需要双路径验收；适用于后续实验阶段 |

实施顺序以用户当前选择的第一项为主。先完成公共接口和可执行的本机验证，使用真实安装包逐步补齐目标平台能力；Windows/ROG 当前缺少环境，其兼容性独立记录为待验证。

## 4. 推荐方向

### 4.1 运行架构与职责

```text
Vue：登录、主机/云游戏列表、会话进度、串流菜单、设置
  │
  ├─ Rust：认证/API、远端 Session、排队/keepalive、SDP/ICE HTTP 信令
  │
  ├─ GamepadBackend：Browser / Rust 选一实现 → 公共 InputController
  │                                           ├─ 几何导航
  │                                           └─ 当前串流输入发送端
  │
  └─ Browser runtime：一个当前播放连接、一个恢复执行入口
       ├─ 统一逻辑输入 / DOM 输入 → Xbox DataChannel
       ├─ RTCPeerConnection → MediaStream → 浏览器解码 → video / audio
       │                                                   └─ 可选 WebGL2 后处理
       └─ 单点 getStats + 帧回调 → 诊断快照、有限恢复决策
```

保留现有 `PlayerClient` 下的 Session/Input/Media/StatsService 边界。`browser-runtime` 收缩成这些组件的装配与生命周期入口；`runtime-host` 在稳定版只负责装配浏览器运行时和投影 UI。组件数量按实际资源所有权确定，优先改造现有模块。

远端会话由 Rust 服务持有，前端负责当前播放连接与页面执行；双方使用同一 operationId 和会话标识关联。菜单、统计面板、后处理设置消费当前会话状态，资源创建和销毁统一由串流执行入口触发。

### 4.2 手柄与其他输入

后续细化见[手柄统一接入与后台策略](/Users/guo.xu/Documents/code/games/xbxrc/docs/isu/2026-10-27-browser-gamepad-rog-fse-design.md)。该文明确统一后端接口、两种后台策略，以及当前无法验证 Windows 时的推进方式。

**物理源：** GamepadBackend 统一提供启动/停止、设备与状态快照、基础震动。Browser 实现用 `requestAnimationFrame` 调用 `navigator.getGamepads()`；Rust 实现复用原生采样和现有桥。同一时刻只装配一个实现，稳定版默认 Browser。标准映射优先；设备身份按各后端本次连接表达，公共层维护活动设备与逻辑槽位。

**消费端：** UI 导航与串流共用同一份逻辑输入。继续使用当前 geometric pathfinding engine、按键 repeat 和焦点恢复机制。键盘 DOM 事件直接进入同一输入归属规则；现有键盘映射为手柄的功能按实际需要复用。鼠标和触屏承担界面操作，原生键鼠直传按现有服务能力另行验收。

**输入归属：** 公共 InputController 单一 owner 表达 `ui / stream / blocked`，两种后端共用。后台策略为 `follow-app / keep-active`：前者消费应用生命周期的单一可靠前后台来源，明确后台时 blocked；后者保持输入。状态缺失按允许消费处理。允许消费时，菜单/UI占用或串流未就绪则为 ui，否则为 stream。Windows 当前默认 keep-active，前台识别与业务路由退出具体手柄后端。

关键行为合同：

1. 菜单打开、follow-app 下明确进入后台、手柄断开、退出时，尽力向仍可发送的连接补发中性输入并停止震动；交接时仅屏蔽仍按住的控件，各自归位后解除屏蔽，其余控件立即可用。keep-active 下前后台变化保持原owner，实际菜单与会话动作仍正常生效。
2. 默认锁定一个活动手柄。断开后显式选择或经新交互确认另一个设备，并等待归位；重连后的浏览器 index 可以变化。
3. DataChannel 背压时保留最新待发状态，优先交付中性释放，限制缓冲并记录丢弃；恢复后发送当前状态。快速按下/松开的边沿交付需要单独测试。协议 keepalive、时间戳和帧 metadata 按协议节拍发送，独立于手柄数值是否变化。
4. 震动复用现有 Xbox 包解析，经 GamepadBackend 调用当前设备；Browser 实现接 `vibrationActuator`，Rust 实现接现有触觉能力。基础双震动按能力开放，高级扳机震动后置。API 缺失或调用失败时，继续输入和播放。
5. 设置页按“设备与行为、操控、震动、测试与高级”四组重做，详细清单与迁移见[手柄设置整体设计](/Users/guo.xu/Documents/code/games/xbxrc/docs/isu/2026-10-27-browser-gamepad-rog-fse-design.md)。后台策略与触觉保存成功后立即应用；游戏布局、左右死区、右摇杆反转和键盘绑定使用草稿应用；游戏自定义保持UI标准导航。Browser/Rust共用用户偏好，接入方式放高级设置并在下次启动生效。旧配置完整保留并按语义迁移，Windows默认后台保持输入。采样频率只在高级详情显示实测值。

释放中性帧采用尽力交付语义：进程冻结、网络中断或通道关闭时，远端接收受实际运行环境限制。重新交接时先发送中性状态，再交付新输入；弱网断开与系统睡眠纳入卡键验收。

FSE 前后台信号的可靠性当前尚未确认。Windows 默认 keep-active，覆盖 FSE 使用场景，省去运行时识别和焦点补偿；其明确代价是后台游戏仍可能响应手柄。该策略只保证应用自己的路由持续允许输入，WebView 挂起与 Gamepad API 的供给能力仍受运行时约束。系统 Home 键可被系统接管，串流菜单提供软 Home 入口。

涉及现有入口：[输入归属](/Users/guo.xu/Documents/code/games/xbxrc/src/shared/gamepad/business-input-arbiter.ts)、[菜单路由](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/stream/stream-input-route-controller.ts)、[键盘桥](/Users/guo.xu/Documents/code/games/xbxrc/src/shared/gamepad/keyboard-fallback-bridge.ts)、[串流震动](/Users/guo.xu/Documents/code/games/xbxrc/src/player/app/input/RumbleService.ts)、[导航震动](/Users/guo.xu/Documents/code/games/xbxrc/src/navigation/core/haptics.ts)、[手柄设置](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/settings/SettingGamepadSection.vue)。

### 4.3 WebRTC 直出

沿用 `WebRtcTransport`、Xbox input/control/chat/message 通道、握手顺序和现有信令服务。浏览器承担 RTP/RTX、抖动缓冲、解码、音视频同步以及协商支持的拥塞控制和反馈机制。项目保留必要的 Xbox 协议适配与经过测试的 SDP 补丁。

首版以 H264、SDR、立体声和保守画质 profile 为基线，建议先验收 1080p60 与 720p60 两档，最终能力以目标服务协商为准。画质在连接前明确选择，播放中依靠浏览器标准 WebRTC 机制适应网络。分辨率、帧率、码率展示实际协商与观测结果。

稳定路径移除 receiver 场景的本地 video sender 限速、动态 quality ladder、display ladder 与反复 SDP 降档决策。保留必需的 TURN 支持、ICE 状态处理及服务端协商要求；TURN 尝试和连接重试共用一次预算。纯音频异常独立诊断，麦克风权限和上行失败保持下行播放可用。

相关实现：[WebRTC transport](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/webrtc/WebRtcTransport.ts:175)、[通道装配](/Users/guo.xu/Documents/code/games/xbxrc/src/player/app/session/SessionService.ts)、[播放服务](/Users/guo.xu/Documents/code/games/xbxrc/src/player/app/media/PlaybackService.ts)。

### 4.4 WebGL 后处理与退化路径

默认使用 video 直出，用户可开启 WebGL2 基础后处理。首版保留一种锐化算法，建议沿用 CAS，并提供亮度、对比度和饱和度调节。FSR 超分、自动档位、多级画质恢复留到稳定基线之后。

视频始终由同一 MediaStream 和浏览器解码。WebGL 从 video 取当前帧渲染到 canvas；共用一个视频帧回调分发渲染和 metadata，优先 `requestVideoFrameCallback`，缺失时采用 rAF 与可用的帧进展信息。应用侧按最新帧绘制，限制输出尺寸与 GPU 负载。

开启后处理时先初始化并完成首帧绘制，再切换显示；切回直出保持原视频元素、音轨、PeerConnection 和会话。WebGL 创建失败、context lost、shader/纹理上传失败时立即露出原 video，并在本次会话中保持直出。单次短时耗时尖峰只进入统计；持续超出验收确定的 GPU 预算才触发性能退化。

声画由浏览器媒体播放负责，实测确认当前独立 audio/video 元素的同步表现、音量、静音、自动播放权限和输出设备变化。浏览器拒绝自动播放时提供明确的播放手势入口。被 canvas 覆盖的视频元素必须持续产帧，目标 WebView 上验证其隐藏方式与节流行为。

现有 [Renderers](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/Renderers.ts:365) 已有 WebGL context 事件入口，重点是统一失败后的视频可见性和资源释放。[resetVideoDecoder](/Users/guo.xu/Documents/code/games/xbxrc/src/player/app/media/PlaybackService.ts:74) 应按真实动作命名为播放元素重建，效果以解码和显示恢复证据判定。

### 4.5 会话生命周期和有限恢复

页面主状态保留 `idle → preparing → connecting → playing → stopping → ended`，连接故障进入 `recovering / error`。云排队作为 preparing 的业务进度，独立显示排队信息和取消操作。状态分别包含远端会话 ID、当前播放 generation 和正在执行的动作。

`start / stop / retry` 通过一个执行入口串行化。新 start 或 stop 取消旧 generation 的异步任务；晚到的 ready、SDP、ICE 和媒体事件只处理所属 generation。Rust 对已取消创建请求的晚到会话 ID 执行清理。关闭幂等，统一释放 rAF、订阅、定时器、媒体元素、PeerConnection、输入占用与 Rust 会话任务。

远端终态由业务层结束当前会话。正常退出显式 stop；断网退出记录待清理结果并做有界补偿，远端服务的租约/超时语义需要验证。进程崩溃和服务失联时，远端会话回收时间以实测与服务契约为准。

恢复动作由浏览器会话控制器单点决策和执行，所有尝试记录动作、原因、generation、耗时、执行结果和恢复效果。

| 证据 | 优先动作 | 停止条件 |
| --- | --- | --- |
| 解码/视频持续推进，WebGL 失败 | 切 video 直出 | 本会话粘滞直出，保留音频与输入 |
| 视频接收活跃，解码或视频帧持续停顿 | 一次限频 Xbox 关键帧请求，观察实际帧进展；必要时一次播放元素重建 | 在统一预算内仍无进展时进入连接级处理 |
| 短时 disconnected | 短宽限观察连接与媒体进展 | 超时或 failed 进入连接级处理 |
| 持续断连或媒体失败，远端会话有效 | 一次受控重连；复用远端会话须先通过服务重新协商验证 | 超时显示明确错误和手动重试入口 |
| 远端终态、认证永久失败、取消或退出 | 停止当前执行并清理 | 后台恢复任务全部结束 |

建议初始实验参数：前台视频无进展 2 秒进入检查，关键帧动作后观察 3 秒，transient disconnect 宽限 3 秒，单次连接重建上限 12 秒，单轮自动恢复总预算 20 秒；每会话最多 2 轮自动连接恢复。音频单独记录进展，后台/睡眠期间暂停前台停顿判定。首帧超时从远端 ready 和媒体连接阶段分别计时，云排队有独立策略。

这些参数是待测初值，发布前依据冷启动、弱网、丢包和恢复样本校准。重连能力按 home/cloud 分别验证；缺少可复用会话的协商证据时，错误页提供显式重新开始。控制消息发送成功、元素重建成功、视频重新出帧分别记录，恢复完成以实际播放进展为准。

### 4.6 Rust 与发行边界

Rust 稳定保留：认证与令牌刷新、Xbox API、会话生命周期、排队、SDP/ICE HTTP 信令、keepalive、配置持久化、窗口与系统集成、更新、诊断写盘。`xbox-streaming` 对 `xbxengine-protocol` 的轻量 DTO 依赖可先保留，后续按实际归属整理。

默认构建退出：`xbxengine` 媒体执行、原生视频宿主、SDL 手柄服务、原生震动、FFmpeg 与它们专用的图形依赖。可采用一个默认关闭的 `native-streaming-experimental` Cargo feature 隔离实验装配，同步控制依赖、模块、RPC 注册、AppState 和后台任务。前端通过构建入口区分稳定与实验装配，稳定用户设置只呈现浏览器路线。

配套工作包括：

1. 旧 `rust-owned` 配置迁移到浏览器；保留仍有效的用户画质与键位偏好，对能力缺失给出清晰提示。
2. 稳定构建命令直接调用 Tauri，FFmpeg SDK 包装脚本只供实验构建使用；CI 与安装资源按构建目标选择。
3. 稳定安装包审查动态依赖和资源；用干净机器确认安装、启动、串流、更新全过程独立于 FFmpeg/SDL 开发环境。
4. 实施时同步更新 AGENTS 技术边界和 current architecture 文档，将本次用户指定的浏览器稳定发行方向写入当前约定；Rust 实验能力保留自己的验证说明。

参考：[package 构建命令](/Users/guo.xu/Documents/code/games/xbxrc/package.json:15)、[构建 CI](/Users/guo.xu/Documents/code/games/xbxrc/.github/workflows/build-tauri.yml:59)、[beta CI](/Users/guo.xu/Documents/code/games/xbxrc/.github/workflows/release-beta.yml:67)。

### 4.7 首版功能范围

| 稳定版必需 | 后续实验能力 |
| --- | --- |
| 登录/刷新、主机与云游戏列表、启动与排队取消 | 新增媒体栈、Rust 自建接收与解码 |
| 音视频、标准手柄、几何导航、菜单与软 Home、必要键盘映射 | 高级触觉、多设备合并、复杂采样/映射策略 |
| 全屏、音量/静音、退出、明确错误与手动重试 | HDR、超高分辨率、复杂动态码率与显示梯级 |
| 可选 CAS 与基础色彩调节、WebGL 故障回到直出 | FSR 超分、自动画质优化 |
| 单点统计、关键生命周期日志、故障导出 | 麦克风按平台单独验收后开放，首版默认关闭 |

### 4.8 稳定性验收与可观测性

以下是计划目标，当前尚无本方案的实机完成证据。Windows WebView2 与 macOS WKWebView 分开记录系统、WebView 版本、GPU/驱动、手柄型号/连接方式、服务类型与网络路径。Windows 掌机 FSE 因近期问题优先验收；macOS 保持独立准入门槛。

**发布硬门槛：** 对每个声明支持的关键组合完成 50 次启动/正常退出，100 次菜单与触屏交接，20 次手柄热插拔，20 次最小化/前后台恢复；home 与 cloud 各进行至少 2 小时持续播放。设备组合至少覆盖目标掌机内置手柄、Xbox USB/蓝牙；其他品牌按实测加入。长时间测试同时覆盖 video 与 WebGL 模式。

故障矩阵覆盖 WebGL context loss、初始化失败、网络断开与恢复、受控丢包/抖动、TURN、睡眠唤醒、音频设备切换、stop/start 竞态、排队取消、令牌过期、远端结束。受支持环境中，客户端崩溃、持续卡键、应用正常可通信时泄漏远端会话、无限重试和重复 PeerConnection 均阻断发布。网络失联与强制终止场景另验远端超时回收。

WebGL context loss 注入在可用扩展下进行；同时通过初始化/上传错误模拟覆盖公共回退合同。目标是下一次可用 video 呈现机会恢复直出，并在实机记录故障到可见帧的耗时。2 小时运行检查内存是否持续增长、是否累积连接/定时器/音轨及输入延迟。50 次成功只作为该批验收结果，长期可靠率由后续 beta 样本估计。

统一 StatsService 每秒最多一个进行中的 getStats 采样，面板、日志与恢复读取相同快照。基线随 PeerConnection generation 重置，采用真实时间差计算速率。帧回调累计轻量计数，按相同窗口汇总；停顿判断结合采样时间和直接媒体进展，避免统计延迟误触发恢复。

| 验证维度 | 浏览器可取证数据与解释边界 |
| --- | --- |
| 网络健康 | ICE/selected candidate、RTT、bytes/packets/loss/jitter；字段按当前实现可用性标注，TURN 路径独立记录 |
| 恢复链健康 | 动作时间线、可用的 nack/pli/fir/keyFramesDecoded 增量、解码及视频帧恢复；浏览器内部 ReferenceChain/RecoveryLedger 标为不可观测 |
| media supply | 接收字节/帧、framesDecoded、视频帧回调与音频进展，区分有网络、能解码、能播放 |
| steady supply | 固定窗口解码与回调帧率、帧间隔、freeze/stall 时长；目标帧率按实际协商值 |
| submit/present 长尾 | WebGL draw 耗时、回调间隔、可用的 expectedDisplayTime/processingDuration；这些属于浏览器可观测代理量，真实屏幕 scanout 需外部测量 |
| 帧供给 delta | 同窗 packets/framesReceived/framesDecoded/video callback/WebGL draw 增量，定位在哪一段开始分叉 |

所有可选字段缺失时显示 unknown。真实 glass-to-glass、输入到屏幕延迟以及绝对呈现长尾通过实机外部测量补证。WebGL 开启/关闭使用同机、同源 profile、同网络对照，记录 P50/P95/P99、最长停顿和资源占用；性能发布阈值在阶段 0 基线测量后固化进 RFC。

日志只保留关键生命周期事件、错误与滚动统计。导出包括构建与能力信息，屏蔽 token、账户标识、完整含敏感参数 URL 和原始 SDP/ICE；采用有界文件与保留期限。诊断快照从权威状态单向生成。

## 5. 待补证据

| 待验证事项 | 决策影响 | 获取方式 |
| --- | --- | --- |
| 目标 Tauri WebView 的 Gamepad 暴露条件、安全上下文、FSE 操作与后台节流 | 补齐设备兼容性与实际供给范围；当前Windows采用keep-active | 具备目标平台环境后用真实安装包验证 |
| WebView2/WKWebView 的 H264、音频同步、自动播放与基础震动能力 | 确定各平台基线及能力提示 | 同一 profile、同一控制器按平台记录 |
| home/cloud 对同一远端 session 重协商的支持、远端失联回收时间 | 决定自动重连与清理策略 | 实际服务运行和故障注入 |
| WebGL 对帧节奏、能耗和温度的影响 | 决定推荐后处理配置、GPU 预算与准入阈值 | 长时间 video/CAS 对照 |
| 现有会话字段与浏览器支持参数的真实交集 | 决定可删投影、配置迁移与共享 DTO 边界 | 对照创建请求、协商结果与 PlayerClient 消费点 |

上述证据通过实施与后续平台补验取得。当前方案按 Windows 掌机重点支持、macOS 独立验证、单活动手柄、Windows后台保持输入、默认 video 直出且 WebGL 可选的范围规划。

## 6. 候选后续任务与实施顺序

| 阶段 | 任务与改动边界 | 验收出口 | 分类 |
| --- | --- | --- | --- |
| 0：公共合同与本机基线 | 定义GamepadBackend、两种后台策略，验证公共路由与本机可执行的播放器能力 | 合同测试通过；平台能力矩阵记录已验证/待验证 | complex |
| 1：输入统一 | 默认接Browser后端，导航/串流/震动消费抽象；Rust实现对齐相同合同；设置按四组重做，区分全局偏好/运行时设备/高级接入方式 | 菜单、设备、策略、配置迁移与中性释放合同通过；Windows默认keep-active | complex |
| 2：会话和直出闭环 | 收敛 start/stop/cancel、单点恢复与统计，移除失效 sender policy 和动态梯级 | home/cloud 从启动到退出可验证；恢复与资源始终有界 | complex |
| 3：后处理 | 保留基础 CAS/色彩调节，统一帧回调、尺寸变化与 video 回退 | WebGL 开关、故障、全屏与长时间运行通过 | complex |
| 4：稳定发行装配 | 隔离原生 feature，迁移设置，精简依赖/资源/CI，更新当前架构约定 | 干净机器安装运行；默认构建与运行完整独立于实验媒体 | complex |
| 5：发布验收 | 自动化关键合同、实机故障矩阵、soak、beta 观察和安装升级 | 硬门槛通过，性能达到阶段 0 固化阈值；已知限制清楚 | complex |

每个阶段沿现有主线实施，公共合同与本机验证先行，平台补验按可用环境推进。关键自动化覆盖后端接口、后台策略、输入归属/边沿/中性帧、取消后晚到事件、幂等关闭、背压、恢复预算和 WebGL 原地回退；整机手柄、实际解码和驱动行为由实机测试承担。实现阶段通过 task-run 建立有依赖顺序的 RFC/任务，完成相应格式化、类型检查、测试与打包检查。

推荐先完成统一手柄接口与后台策略，再按输入实现、会话直出、后处理、发行装配的顺序收敛。Windows/ROG兼容性单独待验，当前工作继续推进。稳定版本取得可重复验收结果后，Rust 媒体以独立实验构建与这条基线比较性能、资源和恢复收益。
