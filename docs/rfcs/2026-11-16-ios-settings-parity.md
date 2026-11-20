# iOS 设置能力对齐 PC

## 状态与授权

2026-11-17，能力范围、移动交互、原生导航和视觉验证已完成。用户通过 /goal 授权全面迁移 PC 设置能力，并要求遵循 iosapp/DESIGN.md、设置操作覆盖无键盘场景；本任务承接连续执行授权，由主窗口单窗口完成。

设置页、控制器工具与串流内设置共用现有 Swift 状态、UniFFI、Rust session/compiler、libwebrtc 和 Metal 渲染链。main 的开发构建入口保持暂停，本任务执行本地验证。

## 核对来源

- [页面展示合同](../../src/pages/settings/setting-page-schema.ts)
- [字段选项与范围](../../src/shared/config/domain-definition.ts)
- [PC 默认值](../../src-tauri/src/mods/config/defaults.rs)
- [颜色、锐化与兼容模式](../../src/shared/config/render-controls.ts)
- [设备映射](../../src/shared/gamepad/device-mapping.ts)与[控制器图示工具](../../src/pages/controller/SettingGamepadEditor.vue)
- [诊断流程](../../src/pages/settings/SettingDiagnosticsSection.vue)

## 能力矩阵

默认值列记述 iOS 新安装状态；平台适配项同时说明 PC 语义。选项、范围与真实消费者一起作为验收依据。

| PC 字段或能力 | iOS 选项与范围 | 默认值 | 实际消费与时机 |
| --- | --- | --- | --- |
| locale | 中文、English、跟随系统 | 跟随系统；PC 为 English | AppRootView 的 locale、string catalog、动态组合文本与标题测量；立即 |
| theme | 浅色、深色、跟随系统 | 跟随系统；PC 为深色 | AppRootView 的 preferredColorScheme 与公共画布；立即 |
| fullscreen / background_keepalive | Apple 全屏串流与场景生命周期；常亮开关 | 常亮开启；PC 启动全屏与后台保活关闭 | 进入串流设置 isIdleTimerDisabled，退出恢复；后台沿原会话暂停、释放输入与触觉 |
| ui_haptics / ui_audio | 独立开关与反馈试听 | 触觉关闭、音效开启 | MobileInteractionFeedback 与公共设置动作、开关、选择、返回；立即 |
| resolution / xhome_resolution | 云与主机分别支持 1440、1081（1080p HQ）、1080、720 | 云 720、主机 1080 | AppSettingsStore snapshot → UniFFI XboxStreamSettings → Rust plan；下一连接 |
| codec | 自动、H.264 High/Main/Constrained Baseline/Baseline（超高/高/标准/低） | 自动 | Rust codec 投影与 libwebrtc SDP 协商；下一连接 |
| xhome_bitrate_mode / xcloud_bitrate_mode | 自动、手动；自动保留手动数值 | 自动 | Rust negotiation compiler；下一连接 |
| xhome_bitrate / xcloud_bitrate | 0～200 Mb/s，步长 1；滑杆、预设、Stepper | 各 20 | 0 使用自动协商，正值作为手动上限；下一连接 |
| audio_bitrate_mode / audio_bitrate | 自动、手动；0～200 kb/s，步长 1 | 自动，手动值 20 | Rust plan → LibWebRtcRuntime 的音频 SDP 投影；下一连接 |
| prevent_resolution_drops | 防止分辨率下降开关，采用 PC 清晰度优先策略 | 关闭 | 设置快照 → Rust input plan → 原生解码观测与 Metal 呈现 → 原 input DataChannel；下一连接 |
| video_format | 完整显示、拉伸、填满、16:10、18:9、21:9、4:3；另提供 16:9 | 完整显示 | StreamingFeatureStore → StreamingMetalRenderer 的几何投影；新会话默认与会话内调整 |
| display_options | 饱和度/对比度/亮度各 0～200%，步长 1；标准/清晰/柔和预设 | 各 100% | 原 Metal shader；当前会话立即，设置页作为下次默认 |
| 锐化 | 关闭/柔和/清晰，数值 0/1/3 | 关闭 | 原 Metal 渲染链；与 PC 强度数值一致 |
| render_compatibility_mode / super_resolution_experimental | 兼容显示、设备原生 MetalFX 超分；放大时最多 2× | 两项关闭 | 同一 renderer、纹理导入、mailbox、commandBuffer 与 present；设备能力决定可用性 |
| enable_audio_control | 系统音量控制显示开关；另提供初始静音 | 音量控制关闭、初始静音关闭 | StreamingFeatureStore、串流菜单系统音量组件与原音频生命周期；会话快照 |
| performance_style | 紧凑性能显示开关 | 关闭 | StreamingPlayerView 的实时统计浮层；立即 |
| stream_menu_shortcuts | 串流/诊断独立组合；17 个标准 Xbox 按键，最多 4 键，含 LT/RT | View + Menu；诊断未绑定 | StreamInputSession 使用映射后状态；长按单次触发、全部释放重新待命、重叠组合按长度优先 |
| 快捷操作录入 | 页内实体 A 启动，释放激活键后采集；组合全部释放后自动保存退出；清空、恢复默认与冲突校验 | 长按 0.60 秒；iOS 可调 0.40～2.00 秒 | MobileControllerCaptureSession 与同一个输入会话；即时保存 |
| vibration / vibration_strength / vibration_channels | 开关、真实/增强/全强；四个握把/扳机通道独立选择，总强度 0～100% | 开启、四路真实、总强度 100% | PC 同值曲线 → 原触觉路由；图片选择部位并在当前页测试 |
| gamepad_game_controls | 图片选择映射目标；原始按钮、附加背键、组合、摇杆及全范围轴；布局配置最多 64 份 | 标准映射；左右死区 10%，Y 反转关闭 | 同一个设备采样与输入输出；死区 0～30%，步长 1%，左右独立 |
| 桌面输入来源与键盘录入 | 移动端使用实体手柄、触屏手柄和原生选择/粘贴；触屏显示自动/始终/隐藏，透明度 20～100% | 自动、透明度 65% | GameController 与现有 TouchGamepadOverlay / StreamInputSession；场景失活释放接管 |
| force_region_ip | 当前网络与澳大利亚/巴西/欧洲/日本/韩国/美国/南印度/中印度，9 项 | 当前网络 | 与 PC 相同的地区 IP，刷新已有 Xbox 授权后重新载入目录 |
| xcloud_region_name | 自动与当前账号可用服务器；刷新列表，保存选择 | 自动 | Rust cloudServerRegion 投影；下一连接 |
| preferred_game_language | 与 PC 相同的 28 项代码，包含平台默认 | en-US | 设置快照 → Rust plan；与应用语言独立，下一连接 |
| 目录市场与语言 | 11 个市场与完整语言选择 | US / en-US | CloudLibraryStore 的请求与缓存 scope；返回目录页重新载入 |
| ipv6 | 优先 IPv6 开关 | 关闭 | LibWebRtcRuntime 按 Rust plan.preferIPv6 排序候选；下一连接 |
| xhome_turn_fallback | 主机 TURN fallback 独立开关 | 关闭 | 共享 Rust compiler 在显式开启时提供 relay；下一连接 |
| server_url / server_username / server_credential | JSON 文件或 PasteButton 导入，16 KiB 上限、URI/字段校验、预览确认、移除确认 | 空 | Keychain 保存 → 现有 snapshot → Rust compiler 解析自定义 TURN；下一连接 |
| diagnostics_lightweight / runtime_trace_mode | 常规/轻量后台记录；关闭/生产/开发级别；手动录制、停止导出、问题记录、最近记录与分享 | 轻量关闭；Debug 开发、Release 生产 | 同一 IOSRuntimeTrace writer；Release 将开发级别投影为生产；立即 |
| 诊断预算与开发工具 | 手动录制最多 5 分钟/32 MiB，最近 3 份保留 7 天；当前/全部 Trace 导出与确认清理；开发分析沿 Xcode 与原生导出 | 原 Trace 保留策略 | 原写入队列、脱敏、文件预算和系统分享 |
| 关于与更新 | 版本、项目主页、发行说明和安装分发渠道 | 当前构建版本 | 原生版本信息、项目发布页、TestFlight/系统分发渠道 |

标准颜色预设为 100/100/100，清晰为 105/105/100，柔和为 96/96/102。四路震动曲线与 PC 同值；旧 v2 记录沿线性强度恢复，保持已有保存值的手感。手机与平板使用同一完整选项集合。

## 交互与存储合同

1. 选择、开关、动作和导航项整行响应，内边距属于点击区域，最小触屏目标 44pt。
2. 数值使用 Slider、预设和 Stepper；凭据使用文件导入或系统粘贴。设置、控制器设置与串流文字发送覆盖无键盘输入。
3. 录入复用原设备采样。A 激活后等待释放；快捷操作保存映射后的 Xbox 目标，映射工具保存原始物理来源；附加轴保留静止基准，完成录入后自动退出。
4. 录入、测试、菜单和生命周期采用独立抑制原因；断开、设备切换、页面失活和后台取消当前接管，恢复前同时检查原始按钮、映射后按钮及摇杆归零。
5. 顶部画布遮挡滚动内容；VStack 的底栏独占高度，中部按实际剩余高度裁剪，末行完整可达。各一级页面保留原生返回栈，边缘返回和触摸可见性共用原生栈定位规则。
6. v1 连接设置与 v2 移动偏好保持既有持久化入口；逐字段解码、有限值归一化与启动写回保留旧用户记录。中英可见文字、无障碍标签和值同步，长按单位为秒。

## 验收与证据

### 2026-11-18 分辨率开关接续

用户最新批注要求将只读策略改为可操作开关。偏好沿现有 v2 逐字段解码保存，旧记录恢复为关闭；下一连接将值送入共享 Rust `Config.input.prevent_resolution_drops`，并由既有 WebRTC plan 投影到 iOS。

开启时，在现有默认 libwebrtc 解码器外记录 RTP 90kHz 键、提交与回调时间；默认解码器、缓冲区和回调继续原样转发。原 Metal commandBuffer 完成回调补齐绘制完成时间，按 RTP 键匹配后送入当前 peer 的有界反馈队列。共享 Rust 协议编码在原 input 包中追加 Xbox 逐帧 Metadata；沿 PC 同值规则将发送副本的解码耗时压至 10ms，并等量修正绘制时间。真实时间观测保留用于诊断。关闭时沿 SDK 默认反馈。

SDK 的公开 `RTCEncodedImage` 提供 RTP 键，接收边界采用完整编码帧到达解码器的本机时刻。该边界明确记为 `encodedFrameIngress`；网络到包时间继续以 libwebrtc 统计和标准 RTCP 为依据。该策略作用于设备解码压力引起的画质选择，实际清晰度继续由网络和服务策略决定，设备压力下可能增加卡顿。UI 将这项取舍贴附开关。

反馈队列按 peer 生命周期隔离，原生解码观测最多 128 帧、待发送最多 32 帧，500ms 过期，单包最多 29 帧；背压时保留原输入发送门禁与中立释放余量。只对 GPU 已完成绘制且时序有效的帧发送反馈，旧 peer 的完成回调由生命周期门禁丢弃。原生绘制时间明确记为 `commandBufferCompletion`，沿当前 Metal 生命周期的完成边界；物理上屏的时刻由后续实机显示健康验收确认。标准 TWCC、NACK、PLI/FIR、REMB/GCC、ICE/TURN 沿原链路。

验证覆盖旧设置恢复、快照与 Rust plan、协议字节、10ms 阈值、长时间时钟回绕、帧键匹配、过期与队列预算、背压和会话退出。模拟器确认控件和消费链；实机画质、网络健康、恢复链、media/steady supply、submit/present 长尾与帧 delta 继续按现有串流验收合同验证。

能力矩阵、真实消费点、旧设置兼容、Rust plan、实体录入状态机、手机/平板 UI、设备专用 MetalFX 构建以及布局截图共同形成验收证据。最终结果与实机步骤记录在 [交付报告](../reports/2026-11-16-ios-settings-parity.md)，视觉装配遵循 [移动端风格合同](../../iosapp/DESIGN.md)。
