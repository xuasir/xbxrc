# iOS 串流行为与设置消费对齐 PC Report

> 2026-11-20 输入范围更新：iOS 游戏输入统一为触屏与外接手柄，PC 键盘模拟从移动端清理。本文中的键盘迁移记录属于此前实现；当前边界见 [iOS 设计规范](../../iosapp/DESIGN.md)及 [本轮验收](../../dev-docs/validation/2026-11-20-ios-gamepad-input-only/README.md)。

## Summary

完成[已确认RFC](../rfcs/2026-11-19-ios-streaming-pc-behavior-parity.md)的M0～M6代码交付与本地自动验收。iOS沿唯一libwebrtc解码、原音频生命周期和Metal mailbox/present管线运行；认证、Xbox API、会话/信令与公共行为判定由Rust持有。任务状态为`Validation Pending`，真实xCloud/xHome的设备验收由用户继续执行。

## Delivered

- 会话内401续期复用现有认证流程，按账户、目标和凭据代次合并请求；刷新结果经原Keychain串行入口保存，退出登录和迟到保存具有代次隔离。
- xHome复用跨区发现与公共启动作用域；唤醒偏好独立保存。远端ICE消费公共configuration、候选来源、mid/ufrag/BUNDLE绑定和EOC；主机先直连，路径耗尽后按15秒连接期限执行一次TURN回退。
- PC与iOS生产入口共用Rust监督合同：连续3秒/15秒观测、首帧保护、解码压力、质量降档/恢复、动作效果、6秒仲裁和120秒连接恢复期限。临时断连沿WebRTC自恢复，硬连接失败和媒体证据进入既有恢复动作。
- 常规与分辨率保持模式持续发送同一Metadata链；原始观测保留，发送副本按策略限幅。RTP关联、旧Peer隔离、队列、过期和输入释放余量沿原通道执行。
- 所选设备共同决定输入、设备公告和震动。原生键盘与已有数据触屏进入同一Xbox逻辑状态；麦克风、质量和Peer重建共用串行协商边界。后台保留逻辑会话并释放输入/触觉，前台核对凭据与远端状态后等待新帧。
- 19组设置按保存、会话快照、公共Plan、原生执行和能力结果建立[消费者矩阵](../../dev-docs/validation/2026-11-19-ios-streaming-pc-behavior-parity/settings-consumer-matrix.json)。静音、麦克风、有效码率及进入前的idle timer状态在恢复与退出时得到相应投影。

## Changes

| 层次 | 主要实现入口 |
| --- | --- |
| 公共监督与恢复 | [supervision.rs](../../crates/xbox-streaming/src/session/recovery/supervision.rs)、[quality.rs](../../crates/xbox-streaming/src/session/recovery/quality.rs)、[observation.rs](../../crates/xbox-streaming/src/session/recovery/observation.rs) |
| iOS Rust桥接 | [streaming.rs](../../crates/xbox-ios-bridge/src/streaming.rs)、[credentials.rs](../../crates/xbox-ios-bridge/src/streaming/credentials.rs)、[home_context.rs](../../crates/xbox-ios-bridge/src/streaming/home_context.rs) |
| 原生动作与状态 | [StreamSessionActor.swift](../../iosapp/XBXRC/Platform/Streaming/StreamSessionActor.swift)、[LibWebRtcRuntime.swift](../../iosapp/XBXRC/Platform/Streaming/LibWebRtcRuntime.swift)、[StreamingRuntime.swift](../../iosapp/XBXRC/Platform/Streaming/StreamingRuntime.swift) |
| 输入与反馈 | [StreamInputSession.swift](../../iosapp/XBXRC/Platform/Streaming/StreamInputSession.swift)、[StreamingFrameFeedback.swift](../../iosapp/XBXRC/Platform/Streaming/StreamingFrameFeedback.swift) |
| PC适配 | [browser-runtime.ts](../../src/streaming/runtime/browser-runtime.ts)、[RPC类型](../../src/shared/rpc/streaming.ts)、[Tauri服务](../../src-tauri/src/mods/streaming/service.rs) |

## Validation

[完整摘要](../../dev-docs/validation/2026-11-19-ios-streaming-pc-behavior-parity/test-summary.json)关联最终日志、Xcode摘要、源码哈希与截图。

| 验证 | 最终结果 |
| --- | --- |
| Rust | xbox-ios-bridge 67、xbox-streaming 176、xbox-streaming-protocol 10，共253项通过 |
| PC | 5个受影响测试文件，共94项通过 |
| 相同事实回放 | 6组186条事实同时通过PC原策略和公共Rust监督 |
| iPhone 17e / iPad Pro 13-inch (M5) | 各178项指定单元测试与2项设置入口UI测试通过 |
| 设备构建 | generic/platform=iOS，关闭签名构建成功 |
| 配套检查 | UniFFI生成、会话边界、cargo fmt、pnpm lint:fix/typecheck/build、Tauri check通过 |
| 模拟器trace | 手机2802行、iPad4573行；schema、序列、配对、脱敏和物理文件/留存预算门通过 |

最终指定单元回归明确排除`testInteractiveLoginPersistsSessionAndProfile`和`testXboxDataStoreKeepsLibraryWhenPlaytimeFails`。两项旧断言分别期待一次聚合profile请求和聚合warning文案，当前资料模块采用四源请求与分区状态；[失败诊断](../../dev-docs/validation/2026-11-19-ios-streaming-pc-behavior-parity/baseline-failures.json)保留实际值和归属。全仓暂存区另有3处其他验收文本的末尾空行诊断，记录于测试摘要。

本轮续接核对8份关键源码哈希，81份源码快照的修改时间均早于最终手机测试。最终4张设置/录入截图经视觉核对，源文件、日志和原始模拟器trace已归档。trace采用`--require-flow none`检验测试宿主与设置交互的结构合同，设备串流表现沿下列清单继续验收。

## Risks

- TURN、待机唤醒、音频权限/路由、麦克风上行和后台调度依赖设备与服务事实，验收需要真实会话。
- iOS解码/GPU完成与PC rVFC/renderer保留各自观测来源；延迟和供帧比较使用相同档位、连续窗口与明确计时边界。
- MetalFX能力回退、实际画面像素、外接多设备输入、四路触觉及中英深浅色串流菜单继续由实机证据确认。

## Follow-up

按[实机验收清单](../../dev-docs/validation/2026-11-19-ios-streaming-pc-behavior-parity/device-acceptance.md)采集xCloud与xHome各一份fresh trace，逐项核对网络健康、恢复链、media supply、steady supply、submit/present长尾与帧供给delta，并将结论回写RFC和任务账本。资料模块接续更新已记录的两项旧断言。

## 2026-11-20 实机闪退接续

用户反馈xCloud/xHome均在出画面前闪退。本轮已复现并修复SDP诊断将跨media section的同号payload放入唯一键字典导致的Swift致命错误；补充真实Peer创建、H.264首帧解码、后台回调与NV12 Metal呈现等12项回归，iPhoneOS构建及trace门通过。[接续证据](../../dev-docs/validation/2026-11-20-ios-streaming-startup-crash/README.md)保留崩溃前后生产函数复现与当前源码哈希。

随后收到的11月19日[实机报告分析](../../dev-docs/validation/2026-11-20-ios-streaming-startup-crash/device-crash-analysis.md)确认独立的UniFFI异步future构造栈溢出：`WrappedFuture::new`单帧预留477280字节，Swift cooperative工作线程栈544KiB，故障写指令触及Stack Guard。后续[栈优化交付](../../dev-docs/validation/2026-11-20-ios-streaming-startup-crash/stack-fix/README.md)将启动上下文与公共会话执行按阶段堆分配，并收敛5个重型信令/恢复入口；start future由119216降至3184B，实际Debug iPhoneOS产物中的对应构造栈帧降至13136B。15个导出入口与3个堆上启动阶段分别满足8KiB、32KiB体积预算；新增6项、7个真实544KiB小栈FFI案例，最终Rust259项、设备构建及绑定合同校验通过。任务保持`Validation Pending`，当前构建的实机首帧、退出重连与六维trace继续由用户验收。

本机 `My Mac (Designed for iPad)` 的Home截图随后停在公共操作轮询入口。公共取消/超时包装在构造前固定操作到堆上，凭据作用域future由24944降至816B，Home上下文由28960降至1568B；三个启动阶段统一满足8KiB体积预算。新增五项回归，当前11个小栈测试覆盖20个544KiB案例，实际FFI走通Home/Cloud控制流程、认证续期错误、取消和释放。最新Rust264项、Debug iPhoneOS构建与PC Tauri编译通过，续接核验源码与测试/产物哈希一致。[操作轮询交付](../../dev-docs/validation/2026-11-20-ios-streaming-startup-crash/home-auth-stack-fix/README.md)保留完整证据；现场已退出，完整运行栈和截图异常精确归因由当前源码的新运行补充。本机Home启动与实机首帧接续验收。

## Cleanup

相关构建、测试、导出和测试宿主均已结束，`/private/tmp/xbxrc-ios-streaming-pc-behavior-parity`的M1/M4两轮目录与本任务系统临时锁已清理。回收23份xcresult、独立DerivedData、克隆包、重复导出、日志及一次性脚本，共1.76 GiB；清理后可用空间60.27 GiB。最终摘要、4张必要截图、失败诊断、12份原始模拟器trace和源码快照保留在仓库证据目录；复用的Rust/WebRTC缓存继续保留。路径核验与空间变化见[清理记录](../../dev-docs/validation/2026-11-19-ios-streaming-pc-behavior-parity/cleanup.json)。

11月20日栈优化接续另回收416.26MiB临时DerivedData、绑定、基线、日志及额外xcresult；设备产物哈希在删除前核验，最终机器码、259项测试日志和固定SDK/Rust缓存保留。[栈优化清理记录](../../dev-docs/validation/2026-11-20-ios-streaming-startup-crash/stack-fix/cleanup.json)记录当前路径与空间核验。

Home 操作轮询修复接续回收 413.69 MiB 临时 DerivedData、基线源码、重复日志及一份系统临时 xcresult；清理后可用空间 61.42 GiB。[清理记录](../../dev-docs/validation/2026-11-20-ios-streaming-startup-crash/home-auth-stack-fix/cleanup.json)关联设备产物删除前哈希核验、最终证据和共享缓存保留项。
