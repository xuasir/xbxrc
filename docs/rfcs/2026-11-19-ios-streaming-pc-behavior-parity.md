# iOS 串流行为与设置消费对齐 PC

> 2026-11-20 输入范围更新：iOS 游戏输入统一为触屏与外接手柄，PC 键盘模拟从移动端清理。本文中的键盘迁移记录属于此前实现；当前边界见 [iOS 设计规范](../../iosapp/DESIGN.md)及 [本轮验收](../../dev-docs/validation/2026-11-20-ios-gamepad-input-only/README.md)。

## Status

- Completion: 实施与自动验收完成；实机验收待用户
- Current State: validation-pending
- Owner: 主窗口，单窗口直接执行
- Last Updated: 2026-11-20
- 用户目标：保留 iOS libwebrtc + Metal 媒体链，其他串流流程对齐 PC main，设置项真实作用于运行行为。
- 执行阶段：用户已确认“按计划全部实施”，主窗口完成 M0～M6 代码与本地自动验收；真实串流按实机清单继续验收。

## Background

[当前源码审计](../../dev-docs/validation/2026-11-19-ios-pc-streaming-main-flow/README.md)确认两端共享 `SessionFlowService/SessionScheduler`，差异集中在宿主认证、主机上下文、候选适配、媒体监督、帧反馈、输入设备归属、音频操作与生命周期。

[既有设置 RFC](2026-11-16-ios-settings-parity.md)与[页面迁移 RFC](2026-11-12-ios-ipad-pc-capability-migration.md)覆盖页面、持久化和部分消费者。本任务将每项串流能力追到配置保存、会话快照、公共 Plan、平台执行、实际状态与回退原因。

当前全部画面比例已进入 `StreamingVideoContentMode`，输入偏好通过闭包逐次更新，颜色/锐化经现有 Metal shader 消费，性能显示直接读取偏好。以上能力进入回归验收；新增实现优先覆盖已定位的行为差异。

## Goal and Scope

1. 认证续期、主机发现、启动/排队/取消、SDP/ICE、通道握手、恢复、输入归属、串流菜单、远端操作和清理与 PC main 的当前行为合同一致。
2. 串流相关设置具有可验证的实际消费点、生效时机和真实能力状态。
3. 公共认证、Xbox API、session/signaling、候选绑定和业务策略保留 Rust；Swift 提供原生事实、libwebrtc API、生命周期及 SwiftUI 状态投影。
4. iOS 保持现有唯一 libwebrtc 解码、音频生命周期与 Metal mailbox/present 管线；效果参数通过既有原生入口消费。原生超分使用 MetalFX，Browser 使用现有 FSR。
5. 平台观测保留来源：Browser rVFC/renderer；iOS decoder ingress/decoder completion/GPU completion。比较采用相同事实含义与稳定窗口。

整体界面风格、目录与个人数据沿已有任务。当前工作区同时存在其他编辑，每阶段采用局部改动，保留它们的内容和暂存状态。

## 主窗口责任

| 模块 | 责任 |
| --- | --- |
| `crates/xbox-ios-bridge/src/cloud_access.rs`、`streaming.rs`与子模块 | access/凭据代次、主机区域、共享flow、设置投影、恢复/信令DTO |
| `crates/xbox-streaming/src/session/`、`policy/` | 扩充原公共事实/决策接口，复用发现与候选策略，保持PC合同 |
| `crates/xbox-streaming-protocol/` | 原input Metadata与Xbox通道公共字节合同 |
| `iosapp/XBXRC/Platform/Streaming/` | 原生事实、动作串行、设备归属、反馈发送、菜单与生命周期 |
| `iosapp/XBXRC/Features/Settings/`、`Features/Controllers/`、`App/` | 设置消费时机、旧值恢复、能力状态与原生交互 |
| `src/streaming/`、`src/player/`、`src-tauri/src/mods/streaming/` | 公共策略调整所需的PC适配与回归，保持当前Browser行为 |
| 原Rust/Swift/Browser测试、iOS trace gate | 正常/失败/取消、设置双向值与代次验证、证据归档 |

## 设置真实消费合同

PC基准取实际消费者、公共compiler、范围与测试：[设置schema](../../src/pages/settings/setting-page-schema.ts)、[字段定义](../../src/shared/config/domain-definition.ts)、[默认值](../../src-tauri/src/mods/config/defaults.rs)、[PC运行时](../../src/streaming/runtime/browser-runtime.ts)。历史字段按当前实际入口和行为确认能力归属。

新增通用字段采用PC默认值；用户已保存的有效显式值保留。现有v1连接设置与v2移动偏好使用原持久化入口逐字段恢复，凭据保留原安全存储边界。

| 设置/能力 | 消费终点与生效时机 | 验收证据 |
| --- | --- | --- |
| 云/主机分辨率：720/1080/1080HQ/1440 | 下次连接：公共device/settings/header、H.264 frame limit与dimensionschanged | cloud/home请求档位、SDP、控制消息、实际源尺寸 |
| H.264档位/自动 | 下次连接：公共codec与原生offer投影 | 各档profile-level-id、RTX apt、远端接受与实际codec |
| 云/主机视频码率Auto/Custom，0～200Mb/s | 下次连接：公共negotiation与同Peer SDP，0采用公共自动语义 | 自动/自定义/0、独立目标、旧值保留与实际上限 |
| 音频码率Auto/Custom，0～200kb/s | 下次连接：公共音频上限与Opus SDP | 自动128kb/s、自定义/0、音频m-line |
| 防止分辨率下降 | 下次连接：常规反馈持续有效，开启后只处理wire副本 | 两模式都有Metadata；原始差值、10ms限幅、队列/背压 |
| 授权地区/云服务器地区/游戏语言 | 授权地区下次授权；服务器/游戏语言下次连接 | 用途独立、失效选择回退、真实origin/locale |
| 优先IPv6 | 下次连接：公共候选策略与native adapter | v4/v6/混合、兼容候选、selected pair |
| xHome TURN fallback、自定义TURN | 下次连接：公共TURN解析/来源与RTCIceServer | 开关/失效/导入移除、host/srflx/relay、实际路径与原因 |
| `power_on`主机唤醒偏好 | 下次连接：公共wake/require-ready/create重试 | 待机主机分别验证开关、电源与远程管理事实 |
| 画面比例全部选项 | 设置页定义下次默认；串流菜单当前会话立即生效 | 每种实际几何、菜单往返、下一连接恢复 |
| 颜色/锐化、兼容显示、原生超分 | 同Metal renderer即时消费；设置页保存下次默认 | 参数/像素变化、保留增强偏好、设备能力与回退 |
| 显示音量控制、初始静音 | 会话快照确定显示与初始值；菜单操作立即作用播放 | 显隐、实际音轨/系统音量状态、失败与重连 |
| 麦克风 | 用户操作后，同Peer采集/方向/聊天SDP协商 | 权限、sender、上行bytes/packets、关闭/取消回滚 |
| 性能显示、诊断模式/轻量采样 | 即时消费原writer与显示状态 | 实际采样、预算、schema、录制/导出和缺值 |
| 手柄选择、映射、死区/Y反转 | 下一次采样：同一设备快照决定输入、公告、震动 | 两设备选择、映射输出、切换归零、断开/背键/轴 |
| 键盘映射/输入来源 | 原生键盘进入既有Xbox逻辑状态与归属 | PC默认映射/组合、菜单录入、按下/释放、失焦归零 |
| 四路震动/总强度/UI反馈 | 即时消费所选设备与原曲线 | 正确设备、四路强度、delay/duration/repeat、关闭释放 |
| 串流/诊断快捷键 | 映射后状态进入同一触发和抑制合同 | 0.6秒、单次/全释放、重叠/空绑定、菜单归属 |
| 触屏显示/透明度、已有数据布局 | 即时消费原虚拟手柄输入和显示入口 | 显隐/透明度、短按/多指/摇杆/扳机、释放与发送 |
| 保持屏幕常亮 | 活动串流期间应用平台idle timer | 保存值、活动窗口、退出/失败/取消恢复原状态 |
| 失活/后台与前台恢复 | 输入/震动先释放，逻辑会话按PC保留语义和系统能力恢复 | 失活/后台/锁屏、远端terminal、续期、恢复后新帧 |

能力状态由宿主真实结果决定。设备不支持或系统拒绝时，控件显示具体可用性与原因；诊断保留请求值、实际值和回退。iOS后台连续执行取决于系统调度，前台恢复主动核对远端会话和当前凭据。

`background_keepalive`当前在PC配置中保留，macOS窗口隐藏由shell直接执行，其配置消费者按代码核实。iOS会话保留和前台恢复对齐实际PC行为；系统后台音频能力按真实音频播放与公开API验证。

## Plan

### M0：合同与设置清单

枚举PC当前字段、范围、默认值、生效时机和消费者，记录iOS对应关系与工作区基线。统一请求配置、实际状态和回退的安全诊断投影。每阶段同步相应设置消费者。

### M1：认证与xHome上下文

iOS provider实现公共 `recover_streaming_token`，复用原AuthFlow/access registry，按目标/账户/凭据代次合并续期并隔离迟到响应。复用 `discover_home`，合并Smartglass身份/电源与串流注册区域，沿同一startup scope异步解析、编译Plan。`power_on`显式进入设置与公共schedule，唤醒/注册/创建重试保持原flow。

### M2：信令、候选与阶段状态

远端ICE消费公共flow结果，收敛bridge固定mid=0的额外地址注入。保留当前configuration、BUNDLE/media/ufrag绑定、source、EOC和晚到配置完成状态。iOS继续使用Rust-owned batch接口，native boundary只执行SDK格式和候选应用。对齐connected/controlReady/mediaReady/steadySupply、取消/失败/远端terminal与唯一cleanup语义，原生呈现保留其观测事实。

### M3：网络/首帧/冻结恢复

用PC现有测试固定恢复动作、阈值、统计新鲜度与效果窗口。在现有公共Rust恢复模块扩充纯事实/决策合同，两端共同消费；平台提供接收/解码/呈现/控制通道/可见性事实，执行现有API动作。覆盖4秒首帧guard、约1秒监督、关键帧请求、局部播放资源恢复、网络压力下质量协商/恢复及公共arbiter重连。保留6秒arbiter窗口、当前episode动作预算与120秒事务期限，恢复以真实新帧观察收口，结果区分improved/neutral/degraded/unknown。Peer重建、设置协商、麦克风与退出共用串行/代次边界。

### M4：常规与清晰度反馈

常规模式持续观测有效帧并发送规范化原始Metadata；分辨率保持模式复用同队列与公共wire限幅。沿默认decoder转发、RTP键、Metal完成回调和原input包，保留128观测/32待发/500ms过期等预算。编码中硬编码开启值改为本次策略，输入/中立释放拥有发送余量。生产trace有界记录模式、可用性、原始/发送差值、观测来源及丢弃原因。

### M5：输入、菜单、音频与生命周期

所选手柄快照共同决定输入、gamepadChanged和震动，复用原映射/抑制。原生键盘接入原Xbox逻辑输入，消费PC默认映射与归属；已有TouchLayout接入原虚拟手柄。麦克风权限/采集/sender/聊天offer沿当前Peer及原音频生命周期，关闭/拒绝/取消/迟到权限响应收口回滚。失活先释放输入/触觉并暂停呈现、失效统计，前台核对terminal/凭据后恢复同一会话。文字发送、Home、关机、重试、菜单返回和诊断导出逐项回归PC结果与失败语义。

### M6：设置端到端与收口

完整覆盖开关/枚举/数值保存、重启、当前会话和下次连接。回归已有比例/shader/输入映射/快捷键/超分能力/轻量诊断。完成Rust/Swift/受影响PC测试、bindings、设备构建和手机/iPad实际入口验证。自动验证与实机待验分别记录，最终Report逐项关联消费者与证据。

## Validation

- [x] Rust：cloud/home Plan、401续期、账户代次、跨区发现、configuration/EOC/ufrag、取消与cleanup，3个crate共253项通过。
- [x] 相同事实回放：6组186条事实同时通过PC原策略与公共Rust判定；预算、冷却、覆盖和陈旧/缺失观测得到相同结果。
- [x] Swift：首帧、connected冻结、failed/disconnected、重建、前台恢复、设备归属、输入释放、麦克风和迟到动作的指定回归通过。
- [x] Metadata：两种模式、RTP键、时钟/回绕、队列/背压/旧Peer和原始值通过7项反馈回归。
- [x] 设置保存、旧值、新会话快照、枚举与数值边界完成回归；19组消费者核对到公共Plan或原生执行入口。
- [x] 画面参数归一化、比例选项、Metal shader编译、音轨静音/麦克风串行协商与重建状态通过自动检查。
- [x] 会话边界、UniFFI、Swift编译、模拟器测试和iPhoneOS设备构建；手机/iPad各178项单元与2项设置入口UI测试通过。
- [x] 受影响PC 94项Vitest、Rust 253项、cargo fmt、pnpm lint:fix/typecheck/build及Tauri check通过。
- [x] 手机/iPad设置选项和手柄图示内联录入真实UI交互通过，最终4张截图与两台模拟器trace结构门已归档。
- [ ] 实机画面几何/像素/MetalFX能力回退、音频播放/上行、串流菜单、中英深浅色、触屏和外接设备交互。
- [ ] fresh xCloud/xHome trace：网络健康、恢复链、media supply、steady supply、submit/present P95/P99、帧供给delta。
- [x] 证据归档、进程结束核对、本轮临时目录清理及空间验证；回收1.76 GiB，23份xcresult及本任务系统临时锁已清理。

## Risks

- 公共恢复合同调整影响PC时序，用既有回放固定默认值、决策与生产诊断预算。
- 原生到达边界和Browser代理不同，显式记录来源/覆盖，网络与清晰度收益通过同条件设备记录确认。
- 后台调度、音频路由、权限、TURN和主机电源属于宿主/服务事实，UI和trace保留真实状态、原因与恢复结果。

## Progress and Execution Notes

- [x] 用户目标、PC基准、当前消费者和主窗口阶段责任已核对。
- [x] 设置矩阵、依赖顺序和验证条件已形成。
- [x] 用户确认“按计划全部实施”，持续目标 active。
- [x] M0～M2：合同、认证、上下文与信令。
- [x] M3～M4：恢复与反馈。
- [x] M5：输入、音频与生命周期。
- [x] M6：设置消费者、自动验收、交付Report与证据归档。
- [ ] 实机清单：真实xCloud/xHome网络、恢复、输入、音频和六维媒体表现，责任人为用户。

2026-11-19，确认设备选择缺口：`StreamInputSession.currentController`读取所选设备，`XboxStreamDataChannels.currentGamepadController`读取系统current/首个设备，M5统一来源。画面比例与逐次输入偏好已核实存在消费者，进入回归验证。

基础证据沿2026-11-19审计：会话边界和14项trace分析器测试通过。本轮整理RFC与账本，持久临时构建/日志/截图/结果包为0；后续轮次按任务/阶段/轮次登记和回收产物。

2026-11-19，实施与自动验收交付：公共Rust负责3s/15s连续观测、解码压力、网络质量和连接恢复预算；PC与iOS生产入口共用判定。临时disconnected保留WebRTC自恢复，failed/closed进入公共恢复；xHome采用直连优先、15s初始连接期限与一次TURN回退。认证更新、静音、麦克风和有效码率在Peer重建及前台恢复后保持当前会话状态。

最终验证为Rust 253项、PC 94项、手机/iPad各180项及iPhoneOS设备构建通过。两项资料模块旧断言按当前分区请求模型单独记录，最终指定串流回归显式排除这两项。关键8份源码哈希一致，81份源文件快照均早于最终手机测试；后续新增内容集中在证据与交付文档。

[交付Report](../reports/2026-11-19-ios-streaming-pc-behavior-parity.md)、[自动验收证据](../../dev-docs/validation/2026-11-19-ios-streaming-pc-behavior-parity/README.md)和[实机验收清单](../../dev-docs/validation/2026-11-19-ios-streaming-pc-behavior-parity/device-acceptance.md)构成收尾入口。模拟器trace门覆盖测试夹具的结构、脱敏、预算及事件配对，真实串流性能按实机证据继续判定。

2026-11-19，续接收尾完成：81份验证源码与当前工作区一致，归档日志和截图哈希通过核验。相关构建、测试和导出均已结束；M1/M4临时目录、23份xcresult、独立DerivedData与克隆包、重复导出及系统临时锁已回收，占用1.76 GiB，清理后可用空间60.27 GiB。最终证据和共享缓存保留，详见[清理记录](../../dev-docs/validation/2026-11-19-ios-streaming-pc-behavior-parity/cleanup.json)。

2026-11-20，启动闪退接续：SDP诊断和UniFFI入口构造栈修复后，针对本机Home截图补充公共取消/超时包装的堆上固定。凭据包装future由24944降至816B，Home上下文由28960降至1568B，三个启动阶段满足8KiB体积预算。11个小栈测试覆盖20个544KiB案例，实际FFI验证Home/Cloud控制流程、认证续期错误、取消和释放；Rust264项、Debug iPhoneOS构建及PC Tauri编译通过。[操作轮询交付](../../dev-docs/validation/2026-11-20-ios-streaming-startup-crash/home-auth-stack-fix/README.md)记录源码/产物对应关系。当前源码的本机Home复测、实机首帧和六维trace继续沿原设备清单验收。
