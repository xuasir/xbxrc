# better-xcloud 六方向源码借鉴审计

日期：2026-11-10。任务类型：simple 源码审计；研究完成，候选项供后续实施决策。

## 核查范围与证据

- better-xcloud：本地 `typescript` 分支，HEAD `76db611e610c45ea996f6d35a9df5f980e8f0f85`；对照 `v6.7.12..v6.7.13` 与 Clarity Boost 提交 `aa346f9`。
- xbxrc：当前 `main`，HEAD `50a4d27d8102f84867290a586e6e03ab8bf595be`；对照 Browser 媒体、公共输入和 Rust 认证/会话/信令/配置。
- 判断依据为已跟踪源码、Git 版本差异和现有 RFC/Report。better-xcloud 工作区中的三份本地总结文档作为目录线索，结论以实现复核。
- 本轮证据确认功能、算法、调用路径和接入缺口。图像观感、GPU 成本、网络收益与物理上屏进入目标设备实测。

最值得吸收的增量是主机跨区域发现、主机媒体地址到 ICE 的实际消费、按游戏覆盖设置、鼠标模拟摇杆、游戏专属触控布局。现有 USM/CAS/SR、4ms 公共采样、发送积压保护、Rust 生命周期与认证继续作为接入基础。

## 1. 串流质量

### 1.1 分辨率与 1080p HQ：已有覆盖，补强实际结果展示

better-xcloud 将 `720p/1080p/1080p HQ` 映射为 Android/Windows/Tizen 的请求设备画像，并同时修改请求头与 `settings.osName`。HQ 是服务端请求画像；本地清晰度处理使用另一组设置。

- 依据：[设备画像映射](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/network.ts:340)、[云串流请求](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/xcloud-interceptor.ts:156)。
- xbxrc 已有相同画像映射，额外支持 1440p，xHome/xCloud 档位独立：[Rust 设备计划](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/src/policy/session/compiler.rs:158)。
- 借鉴方向：界面与导出摘要同时呈现“请求档位、实际源尺寸、处理后尺寸”，继续从远端 SDP 与实际入站视频验证请求结果。现有请求宽高合同和逐帧供给继续复用。

### 1.2 Codec：借鉴能力驱动的选项展示，保持准确命名

better-xcloud 用 `RTCRtpReceiver.getCapabilities('video')` 动态构造可选编码档位，随后调整 SDP 的 payload 顺序。其 UI 名称 `high` 对应 profile 前缀 `4d`，实际为 H.264 Main；H.264 High 的 family 是 `64`。

- 依据：[可选档位构造](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/settings-storages/global-settings-storage.ts:25)、[SDP 编码偏好](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/sdp.ts:1)。
- xbxrc 已区分 `H264Main/4d` 与 `H264High/64`，实际协商也检查浏览器能力：[Rust 映射](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/src/policy/negotiation/compiler.rs:55)、[Browser 协商](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/webrtc/SdpManipulator.ts:33)。设置 schema 当前采用静态 `CODEC_OPTIONS`。
- 借鉴方向：把运行能力投影到设置选项和有效状态，保留用户偏好，标明实际选中的 profile、解码器和硬件解码能力。能力缺项采用 unknown，浏览器可协商、硬件高效解码、服务端实际选择分别记录。

### 1.3 码率：复用现有合同，清楚呈现上限与实际码率

better-xcloud 的自定义码率通过 SDP `b=AS` 表达接收上限。最大设置值被转换为“默认/无限制”意图，代码仅在用户选择更小数值时插入上限。其设置界面按 `1024*1000` 显示单位，SDP 写入按 `/1000` 转换，迁移时应使用统一十进制 kbps/Mbps 合同。

- 依据：[码率设置](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/settings-storages/global-settings-storage.ts:307)、[设置本地 SDP](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/monkey-patches.ts:95)。
- xbxrc 已有云/主机/音频独立 Auto/Custom 和分档预算：[Rust 码率计划](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/src/policy/negotiation/compiler.rs:90)。我们的 SDP 修改按媒体 section 处理 AS/TIAS，能覆盖最后一个媒体段：[SDP 实现](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/webrtc/SdpManipulator.ts:5)。
- 借鉴方向：设置继续表达请求上限，诊断另列实际视频 Mbps、远端声明上限及拥塞事实。更高请求值的收益由实际服务结果确认。

### 1.4 “防止分辨率下降”：保留真实帧时间

better-xcloud 的该开关默认关闭。实现向 Xbox 客户端 `onVideoFrame` 注入代码，固定 `frameDecodedTimeMs = frameSubmittedTimeMs + 10`，再改写 `frameRenderedTimeMs/expectedDisplayTime`；`if (true || ...)` 使它对每帧执行。这是对下游客户端判断输入的改写。

- 依据：[metadata 修改](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/patcher/patcher.ts:1256)。
- xbxrc 建议：保持原始帧时间与时钟校准证据；解码、处理、提交、上屏分别观测。画质请求变更由真实拥塞、能力和效果反馈决定。
- 后续规划补充：当前 [InputService](/Users/guo.xu/Documents/code/games/xbxrc/src/player/app/input/InputService.ts:373) 已无条件将大于 10ms 的 decode 反馈限幅。用户选定将其配置化为体验开关，原始 observation 与上报策略各自记录；修改计划见[四项改进 RFC](../rfcs/2026-11-10-better-xcloud-selected-streaming-improvements.md)。

## 2. 渲染与画面后处理

### 2.1 6.7.13 WebGL2：吸收精度和资源细节

该版本对已有 Clarity Boost 做了以下改良：float/sampler 使用 highp；CAS 的分母添加黑色像素保护；性能档使用 headroom 1、质量档使用 2；饱和度采用 Rec.709 系数；CPU 预计算 texelSize；Y 翻转移入 shader；`gl_VertexID` 生成全屏三角形；缓存 uniform；更新 viewport；链接后删除 shader，销毁时先释放资源再 loseContext。

- 依据：[fragment shader](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/player/webgl2/shaders/clarity-boost.fs:3)、[vertex shader](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/player/webgl2/shaders/clarity-boost.vert:3)、[资源实现](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/player/webgl2/webgl2-player.ts:100)。
- xbxrc 已有 highp float、uniform 缓存、viewport 更新、desynchronized、完整三角形，以及 USM/CAS 的抑噪、增益上限、色度与过冲保护：[当前 shader](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/Renderers.ts:434)。当前采样器精度沿 GLSL 默认值、亮度系数为 0.299/0.587/0.114、三角形使用 vertex buffer、上传开启 Y 翻转。
- 优先候选：显式 sampler 精度、与 VideoFrame/browser 色彩合同一致的亮度系数、顶点与 shader 资源简化。texelSize 预计算与 Y 翻转移动作为成本实验；源尺寸和输出尺寸继续独立，SR/FBO/截图的坐标共同验证。
- 资源边界：我们的 EASU 按需创建 program 时复用现有顶点 shader；释放时机需要覆盖所有 program 的最后一次链接。全屏三角形当前已经使用三个顶点，gl_VertexID 的收益主要是简化顶点资源。
- 质量约束：保留我们的增益上限、噪声门限与过冲保护；颜色系数变动需要同步核对参考数学和已有风格观感。

### 2.2 逐帧驱动：保持当前 VideoFrame 供给

better-xcloud Canvas 播放器通过 RVFC 驱动，接口缺省时走 rAF；WebGL2 每次从 HTMLVideoElement 上传纹理。xbxrc 已改为独立解码 VideoFrame、Worker 最新槽和单帧在途预算。

- 依据：[better-xcloud 调度](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/player/base-canvas-player.ts:25)、[视频纹理上传](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/player/webgl2/webgl2-player.ts:55)、[我们当前上传](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/Renderers.ts:720)。
- 借鉴结论：GPU 局部优化接入现有逐帧引擎，继续用独立帧身份、时钟、ACK、关闭与提交事实验证新鲜度。

### 2.3 WebGPU 与基础 video 滤镜：保留研究身份

better-xcloud 的 WebGPU 通过 `importExternalTexture({source: video})` 使用外部纹理。其 WGSL 仍使用旧亮度系数、CAS 黑色分母和始终执行的 3×3 读取；6.7.13 的 WebGL2 修正集中在另一条实现。基础 video 使用 SVG 卷积锐化与 CSS 色彩，属于浏览器合成路径的后处理。

- 依据：[外部纹理](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/player/webgpu/webgpu-player.ts:112)、[WGSL](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/player/webgpu/shaders/clarity-boost.wgsl:36)、[video 滤镜](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/player/video/video-player.ts:73)。
- 借鉴方向：WebGPU 先做相同逐帧供给、相同像素质量下的导入/GPU/上屏成本对照；正式装配沿共享引擎和 RFC。渲染降级继续承担降低后处理成本的目的。

### 2.4 比例、位置与音频：按实际体验选择小实验

better-xcloud 提供多种画面比例及居中/靠上/靠下位置，增强播放器变化后更新原生触摸尺寸；还提供合并音视频媒体元素的实验开关、GainNode 音量与 AudioContext latencyHint 调整。

- 依据：[比例与位置](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/stream-player-manager.ts:32)、[AudioContext](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/monkey-patches.ts:131)、[合并媒体](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/patcher/patcher.ts:387)。
- xbxrc 已有 Contain/Stretch/Zoom、GainNode 与失败回退：[画面适配](/Users/guo.xu/Documents/code/games/xbxrc/src/player/app/media/PlaybackService.ts:504)、[音频效果](/Users/guo.xu/Documents/code/games/xbxrc/src/player/app/media/AudioEffectsService.ts:15)。
- 借鉴方向：为触控预留画面安全区域和位置选择；映射使用实际 content rect，处理黑边、缩放与裁切。音频低延迟 hint 与媒体合并仅在音画同步、输出延迟、功耗和生命周期对照中推进。

## 3. 网络与服务器

### 3.1 区域：借鉴动态列表与明确的实际服务器身份

better-xcloud 从登录响应读取地区列表，补国家/大洲/名称，选中地区后将该地区传回 Xbox 客户端；实际 play URL 同时用于服务器 badge。此路径实现显式选择与展示。

- 依据：[区域列表与选择](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/xcloud-interceptor.ts:76)、[实际请求地区](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/xcloud-interceptor.ts:135)。
- xbxrc Rust 已支持 preferred_region_name、默认区域及有效 base URI 选择：[区域策略](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/src/policy/session/compiler.rs:96)。
- 借鉴方向：服务返回的有效区域列表驱动 UI，串流时显示实际区域和连接路径。按区域积累成功率、RTT、抖动和冻结画像属于我们可扩展的产品策略，需要样本时效与网络环境区分。
- 区域登录、控制面 host 和实际媒体 ICE 路径分别记录；用户选择由会话创建消费。

### 3.2 IPv6 与 ICE：保留标准字段，评估实际选中路径

better-xcloud 的重写器仅匹配 foundation 为数字、component 为数字、transport 为大写 UDP 的字符串。它可把 IPv6 排在前面，并将第一项 priority 设为 2130706431、其他项设为 1，再重新编号 foundation、固定 mid/component。

- 依据：[候选重写](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/network.ts:43)。
- xbxrc Rust 已按候选类型/地址族稳定排序并保留 priority 等字段，xHome 支持 Teredo IPv4 派生；Browser 另有地址族匹配和策略诊断：[Rust ICE](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/src/session/signaling/ice.rs:34)、[网关启用范围](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/src/session/api/signaling.rs:35)、[Browser 策略](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/runtime/ice-candidate-policy.ts:3)。
- 借鉴结论：用户级 IPv6 偏好继续沿现有合同；候选 origin、type、transport、地址族与选中路径必须可复盘。实际 ICE 连接由浏览器检查结果形成，列表排序只表达偏好。

### 3.3 网络与画面诊断：复用差分方法，准确命名

better-xcloud 用 bytes delta 算码率、decode time delta/decoded frames 算平均解码时间。它标为 Jitter 的值实际是 jitterBufferDelay delta/emittedCount，表示平均抖动缓冲驻留时间；FPS 读取 inbound-rtp framesPerSecond；selectedCandidatePairId 首次确定后缓存。

- 依据：[统计计算](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/stream-stats-collector.ts:227)。
- xbxrc 已分别输出 networkJitterMs 与 jitterBufferMs，并按当前 transport 解析选中路径：[分层网络指标](/Users/guo.xu/Documents/code/games/xbxrc/src/player/app/stats/browser-stats.ts:182)、[当前候选对解析](/Users/guo.xu/Documents/code/games/xbxrc/src/player/app/stats/StatsService.ts:579)。
- 借鉴方向：简洁可读的即时指标与按需展开；网络 jitter、buffer 驻留、接收/解码 FPS、提交 FPS、呈现观察 FPS 分别命名。路径改变后更新关联，counter 重置及缺字段保持明确语义。
- WebRTC 拥塞、重传和反馈继续由 Browser WebRTC 与现有公共监督处理；源码借鉴重点在控制参数与诊断呈现。

## 4. 输入链路

### 4.1 采样与设置：已有主线能力，扩展成本受控

better-xcloud 默认 4ms 轮询，可选至 60ms，通过修改 Xbox 客户端定时器实现。手柄键位、触发范围、摇杆死区与震动参数在设置刷新时转换，游戏输入消费转换后的配置。

- 依据：[轮询设置](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/settings-storages/stream-settings-storage.ts:45)、[定时器接入](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/patcher/patcher.ts:140)、[预计算](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/stream-settings.ts:103)。
- xbxrc 已有同源 rAF+4ms 补采、状态变化发布与心跳，以及 queue age/bufferedAmount/释放保护：[Browser backend](/Users/guo.xu/Documents/code/games/xbxrc/src/shared/gamepad/browser-backend.ts:135)、[发送服务](/Users/guo.xu/Documents/code/games/xbxrc/src/player/app/input/InputService.ts:12)。
- 借鉴方向：配置变化时完成映射预计算，诊断显示实际 poll/publish Hz 和 sample-to-send；节电采样档需独立验证短按捕获、主线程成本与后台调度。

### 4.2 鼠标模拟摇杆：明确功能增量

better-xcloud 支持鼠标映射左/右摇杆、X/Y 独立灵敏度、低幅移动死区补偿、径向上限，以及停动 50ms 后摇杆归零。滚轮映射游戏按键，20ms 后释放；Pointer Lock 优先请求 unadjustedMovement。键鼠按游戏预设选择。

- 依据：[鼠标映射](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/mkb/mkb-handler.ts:355)、[滚轮短按与捕获](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/mkb/mkb-handler.ts:389)、[参数](/Users/guo.xu/Documents/code/games/better-xcloud/src/enums/mkb.ts:1)。
- xbxrc 当前有键盘模拟桥、虚拟输入接口和 Mouse/Pointer 编码基础，实际 InputService 发送的是 metadata/gamepad 路径：[键盘桥](/Users/guo.xu/Documents/code/games/xbxrc/src/shared/gamepad/keyboard-fallback-bridge.ts:81)、[虚拟状态](/Users/guo.xu/Documents/code/games/xbxrc/src/shared/gamepad/input-controller.ts:197)、[编码结构](/Users/guo.xu/Documents/code/games/xbxrc/src/player/protocol/input/InputPacketEncoder.ts:48)。
- 接入建议：先通过 setVirtualState 与现有 owner 发送链实现“鼠标模拟手柄”，保留共享释放保护。50ms/20ms 是参考参数，使用实际事件时间和定时抖动验证；UI 录入、弹层、失焦、Pointer Lock 退出统一释放。

### 4.3 原生键鼠与本地合作：分开形成执行合同

better-xcloud 的原生键鼠控制 enableKeyboardInput/enableMouseInput，发送 relative X/Y、Buttons、WheelX/Y；包装应用通过 AppInterface 与 localhost WebSocket 采集鼠标。其本地合作绕过 Xbox 客户端固定槽位，提供多个实体/虚拟输入槽位和支持游戏列表。

- 依据：[原生鼠标输入](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/mkb/native-mkb-handler.ts:193)、[宿主鼠标来源](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/mkb/pointer-client.ts:28)、[本地合作](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/patcher/patches/src/local-co-op-enable.ts:6)、[槽位设置](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/settings-storages/stream-settings-storage.ts:115)。
- 接入建议：原生键鼠以 title 能力、runtime 能力、控制配置和发送消费闭环为前提，宿主能力沿 Tauri/Rust 现有桥扩展。本地合作沿公共输入增加玩家槽位、每槽位 neutral/热插拔/owner 与震动路由；服务端支持列表和实际双玩家结果共同验证。
- 当前公共链和 rumble 接收以主玩家为核心，协作功能需先落跨层合同与 RFC。

## 5. 触控和震动

### 5.1 触控：通用布局 + 游戏专属数据

better-xcloud 区分 NativeTouch、GenericTouch、CustomTouchOverlay；通用触控扩展所有游戏，官方支持布局沿 title 能力，社区布局按十进制 Xbox TitleId 下载，并可选择多个布局、默认项与作者。message 通道 `/titleinfo` 更新当前 title，随后响应 touchcontrols 消息选择布局。

- 依据：[输入类型](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/bx-exposed.ts:125)、[布局加载](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/touch-controller.ts:132)、[title 切换](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/touch-controller.ts:330)。
- 实际布局渲染依赖 Xbox 网页 TAK/Babylon：[manager 暴露](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/patcher/patcher.ts:276)。陀螺仪部分是布局包含该字段时开放既有传感器入口：[布局应用](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/touch-controller.ts:245)。
- xbxrc 当前桌面 touch_surface=false，消息初始化也发送 false；领域层和编码器已有基础：[桌面能力](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/streaming/context.rs:268)、[消息初始化](/Users/guo.xu/Documents/code/games/xbxrc/src/player/protocol/channels/MessageChannel.ts:87)。
- 接入建议：Vue 通用手柄叠层先接公共虚拟状态；再支持版本化布局数据、titleId/productId 映射、多布局、透明度、安全区域与记忆选择。原生触摸路径单独验证服务端能力和 pointer 坐标。传感器入口以宿主真实能力为依据。
- 稳定性要求：多点绑定、pointercancel、菜单打开、窗口隐藏、实体手柄接管、退出与重连均释放有效状态。布局下载使用受控并发、schema 校验、会话代次、缓存和 fallback。
- 实现提醒：better-xcloud 使用 async forEach 和固定 1s 等待布局加载；我们的加载闭环采用显式完成信号和当前代次校验。

### 5.2 触控资源：借鉴按需渲染

better-xcloud 对 TAK 的 WebGL context 关闭抗锯齿、选择低功耗偏好，键鼠启用时允许 dispose TAK renderer。连接实体手柄时的自动隐藏选项在 title 能力处理阶段检查 getGamepads。

- 依据：[触控 context](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/monkey-patches.ts:224)、[TAK 资源停止](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/patcher/patcher.ts:293)、[实体手柄检测](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/bx-exposed.ts:128)。
- 借鉴方向：触控叠层采用事件驱动状态，隐藏时停止动画和传感器，优先使用 Vue/CSS 控件；自动模式根据当前实际输入来源更新，保留显式显示选择。

### 5.3 震动：按真实 actuator 路由

better-xcloud 为手柄四电机报文统一乘强度；设备震动另外从 input 通道解析报文，优先交给包装应用 AppInterface。Browser navigator.vibrate 使用 200ms 开关脉冲模拟强度，Auto 模式在实体手柄缺省时启用。

- 依据：[手柄强度](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/patcher/patches/src/vibration-adjust.ts:10)、[设备震动](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/device-vibration-manager.ts:65)、[Auto 条件](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/stream-settings.ts:142)。
- xbxrc 已有报文兼容、强度曲线、owner 隔离和 stop；当前 Browser backend 消费 dual-rumble，DTO 保存 trigger 分量：[报文解析](/Users/guo.xu/Documents/code/games/xbxrc/src/player/app/input/RumbleService.ts:18)、[公共强度](/Users/guo.xu/Documents/code/games/xbxrc/src/shared/gamepad/rumble.ts:11)、[Browser actuator](/Users/guo.xu/Documents/code/games/xbxrc/src/shared/gamepad/browser-backend.ts:264)。
- 借鉴方向：按真实设备列出 dual/trigger/设备震动能力，保持偏好与有效状态区分。触控设备的自动震动可作为宿主能力扩展；Windows 掌机与 macOS 的设备马达以实测/API 能力确认。扳机字段、浏览器 actuator 支持、物理反馈分别验证。
- 多玩家接续每槽位震动路由。设备脉冲是节奏模拟，200ms 常量需要在游戏事件持续时间和宿主调度下评估。

## 6. 主机串流

### 6.1 跨区域查找主机：优先借鉴

better-xcloud 把 xHome 登录响应中的默认 region 放到前面，依次请求各区域 `/v6/servers/home?mr=50`，跳过空列表和失败，找到主机后保存可用 server。

- 依据：[查找过程](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/remote-play-manager.ts:175)。
- xbxrc 当前创建一次 home query gateway，由 Rust compiler 选定一个区域，再查询列表：[查询入口](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/services/streaming_query_service.rs:31)、[gateway 创建](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/services/streaming_query_service.rs:182)。
- 候选实现：在 Rust 查询层对登录响应的有效 region 做有界尝试，默认优先、空列表/可恢复网络失败接续；认证失败走现有 token recovery。找到主机后绑定本次发现区域与后续 home 会话，设置缓存时效和账户隔离。
- 验证：默认空列表+其他区命中、首区故障、全部空、401、取消、重复主机、旧缓存、跨账户。区域扫描负责发现可用控制面，媒体直连由 ICE 结果确认。

### 6.2 主机地址补候选：当前最值得核对的消费缺口

better-xcloud 从当前 session configuration 的 serverDetails 读取 ipAddress/port、ipV4Address/ipV4Port、ipV6Address/ipV6Port，为每个地址添加服务端端口及 9002，再追加 host ICE candidate。

- 依据：[当前配置提取](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/xhome-interceptor.ts:35)、[追加候选](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/network.ts:88)。
- xbxrc 已从主机列表的已知路径提取 console_addrs，并在 negotiation plan 声明 inject_console_addrs：[地址提取](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/services/streaming_query_service.rs:239)、[policy 声明](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/src/policy/negotiation/compiler.rs:35)。
- 消费核查：当前 WebApiSignalingGateway 构造仅消费 prefer_ipv6 与 xHome Teredo 开关，normalize 接收远端候选后调用 IcePolicy；该路径的状态和处理尚只包含这两项：[构造](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/src/session/api/signaling.rs:29)、[候选处理](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/src/session/api/signaling.rs:97)。Browser 候选策略也以远端 candidates 和地址族为输入。
- 后续规划补充：get_configuration 已有 API 定义，当前会话主线尚缺调用与配置消费；resolve_remote_play_context 中的 configuration_resolved=true 来自主机列表匹配。实施时将 console-list 与 session-configuration 来源明确区分，并验证当前配置、补候选和 ICE 代次的配对。
- 优先建议：补齐“当前 configuration 地址事实 → 公共 negotiation/信令 → 浏览器 addIceCandidate”的端到端合同。对已知字段显式解析、去重、会话代次和地址族校验；已有远端候选与 TURN 继续保留。9002 作为明确的兼容补候选策略，用真实主机和连接结果验证端口可用性。
- 验证覆盖 LAN、家庭 NAT、异地、IPv4/IPv6、Teredo、TURN、空/晚到 ICE、重复配置、退出取消。记录新增候选来源、选中路径、连接/首帧耗时及失败原因。

### 6.3 保活与生命周期：复用 Rust 现有闭环

better-xcloud 捕获主机串流 WarningForBeingIdle 并调用 Xbox 客户端 sendKeepAlive；其 xHome token/list 在 manager 内缓存。xbxrc 已有 Rust 定时 keepalive、取消令牌、15s 操作期限、401/404 终止处理、wake/ready、close 和认证续期。

- 依据：[空闲警告处理](/Users/guo.xu/Documents/code/games/better-xcloud/src/modules/patcher/patches/src/remote-play-keep-alive.ts:4)、[Rust 保活](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/src/session/scheduler.rs:279)、[主机控制](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/services/streaming_query_service.rs:100)。
- 借鉴方向：服务端空闲告警作为当前会话诊断事实，并通过既有 Rust 调度器处理有界补保活。token/list 缓存沿现有账户隔离、TTL 与统一续期合同。
- xHome 专属触控布局沿当前 titleInfo/inputconfigs 接续，菜单/游戏 title 切换后恢复默认与自定义布局需要配对验证。

## 7. 横向复用：按游戏配置

better-xcloud 的 GameSettingsStorage 按 Xbox TitleId 建立存储，字段级覆盖全局设置，读到与全局相同的值会清除该覆盖。画面、键鼠预设和其他 StreamPref 因而可按游戏选择。

- 依据：[按游戏覆盖](/Users/guo.xu/Documents/code/games/better-xcloud/src/utils/settings-storages/stream-settings-storage.ts:425)。
- xbxrc 已区分全局默认、本次应用、预览/A/B 临时状态；建议增加 Rust 持久化的“保存到当前游戏”，复用现有共享控件、配置投影和代次。
- 意图合同：明确“继承全局”与“固定此值”。当前值等于全局的固定选择仍应保持用户意图；恢复继承通过明确动作完成。
- identity 合同：titleId/productId/云 catalog ID/主机当前游戏 ID 形成明确映射，云/主机同游戏的共享范围由产品选择。首先覆盖锐化、色彩、画面位置、鼠标/触控布局，网络和设备偏好维持各自范围。

## 8. 推荐推进顺序

| 批次 | 候选 | 接入与验收重点 |
| --- | --- | --- |
| 近期 1 | xHome 跨区域发现与主机地址消费审计 | 先确认查询/当前 configuration/plan/信令的缺口，形成公共 Rust 合同和最小回归 |
| 近期 2 | WebGL2 精度、色彩与资源细节 | 同源像素对照、源/输出尺寸、SR/FBO/截图、GPU 完成与上屏成本 |
| 近期 3 | 按游戏保存设置与 Codec 能力展示 | 配置继承/固定意图、title identity、预览/本次/默认/游戏范围 |
| 功能 1 | 鼠标模拟摇杆与滚轮短按 | 公共虚拟输入、Pointer Lock、短按/释放、菜单交接、实际 sample-to-send |
| 功能 2 | 通用触控与游戏专属布局 | Vue 叠层、布局 schema/缓存、安全区域、多点取消、输入接管 |
| 功能 3 | 本地合作与 actuator 能力 | 玩家槽位/owner、热插拔、释放、每玩家震动、服务端支持结果 |
| 研究 | WebGPU 外部纹理、A/V 合并、音频延迟 hint | 相同供给和质量下的目标设备成本实验，后续 RFC 决定正式装配 |

## 9. 历史约束与验证复用

相关历史按结构相似度复用：

1. [统一手柄报告](/Users/guo.xu/Documents/code/games/xbxrc/docs/reports/2026-10-27-unified-gamepad-input.md)：公共 owner、标准 UI 输入、游戏映射一次应用、热插拔/中性释放/震动。
2. [CAS 颗粒控制 RFC](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-10-23-cas-sharpening-grain-control.md)：增益上限、噪声门限、邻域约束和源尺寸锐化；是吸收 shader 改动时的图像保护基础。
3. [Browser 可靠性 RFC](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-10-28-browser-runtime-simplification-and-stability.md)：真实拥塞决定有能力的网络动作，显示压力归本地；home/cloud 生命周期与候选结束规则分列。
4. [当前直通引擎报告](/Users/guo.xu/Documents/code/games/xbxrc/docs/reports/2026-11-10-default-passthrough-render-engine.md)：SR/锐化/风格共享引擎和逐帧资源闭环。
5. [帧时钟与诊断报告](/Users/guo.xu/Documents/code/games/xbxrc/docs/reports/2026-11-10-browser-frame-freshness-clock-and-diagnostics.md)：原始时序、帧身份和低干扰 GPU 完成代理。

媒体/网络策略的验收按同一稳定时间窗覆盖：

| 维度 | 必备事实 |
| --- | --- |
| 网络健康 | RTT、network jitter、loss/NACK、接收 bytes/packets delta、jitter buffer、选中候选路径 |
| 恢复链健康 | 冻结/恢复事件、当前会话/轨身份、关键帧与恢复动作、恢复完成和连续画面证据 |
| media supply | 接收、解码、VideoFrame reader 的同窗供给 |
| steady supply | reader/Worker/latest-slot、在途/ACK、实际提交间隔与持续供帧 |
| submit/present 长尾 | read-to-submit、GPU 完成代理、RVFC 呈现观察；物理上屏另取设备证据 |
| 帧供给 delta | 接收→解码→reader→submit→呈现的差分，覆盖/陈旧/本地释放各自配平 |

输入额外采集 poll/publish Hz、sample-to-send、队列年龄、bufferedAmount、短按/释放结果及 owner；触控额外验证 content rect、多点与取消；震动额外验证 actuator 实际完成与停止。

文档检查：88 个本地源码/文档链接全部存在，带行号链接均在有效范围；任务登记唯一，git diff --check 通过。包含后续四项改进规划补充。

本轮交付为源码审计、版本差异核查和候选接入建议。建议先推进 xHome 查询与候选消费闭环，再接续低风险渲染细节和按游戏配置。
