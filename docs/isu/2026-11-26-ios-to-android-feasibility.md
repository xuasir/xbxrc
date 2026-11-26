# 按 iOS 规格实现 Android 端的可行性评估

可以按现有 iOS 的功能与行为合同实现 Android 端。推荐 **Kotlin + Jetpack Compose + 统一移动 UniFFI 桥接 + 共享 Rust + Android libwebrtc**；视频沿 libwebrtc 的 MediaCodec 硬解与纹理输出，使用同一条 Android 原生显示链。主要投入集中在平台宿主、低延迟显示、控制输入和设备兼容性，整体属于中大型移植。

评估日期：2026-11-26。源码基线为 `release/test` / `a98e5457c8f9` 及当前工作区的 iOS、TAK、音频与后台续接修改。规格以 11 月最新设计规范、代码与报告为准。本轮交付为方案评估，Android 构建与实机证据列入后续任务。

## 1. 问题定义

Android 应与 iOS 保持相同的游戏身份、账户准入、数据状态、串流流程、输入语义、设置消费和诊断口径。界面和系统能力按 Android 规范实现：Compose 承接页面，Android 平台 API 承接登录、安全存储、音频、显示和设备输入。

可行性的关键在于共享业务规则的成熟度，以及 Android 是否能在代表设备上达到移动串流的延迟、恢复和功耗目标。功能对齐与设备能力需要分别验收。

## 2. 当前基础与约束

### 可复用基础

- `xbox-auth-flow`、`xbox-webapi`：OAuth/PKCE/state、Xbox 认证、续期和业务 API。
- `xbox-cloud-catalog-flow`、`xbox-streaming`、`xbox-streaming-protocol`：目录、访问资格、会话/信令、ICE/TURN 策略、恢复与通道合同。
- `xbox-ios-bridge`：已有粗粒度业务接口、异步认证回调、账户代次、访问租约和查询错误投影，可作为统一移动桥接的演进入口。
- `xbox-touch-layouts`：官方/社区资源、版本化协议、Context/JSON Patch、公共编译 IR 和能力诊断。11 月 26 日冻结社区样本的 75 个模板、85 份布局全部可运行，其中 70 份完整兼容；这组结果可作为 Android 的共用输入样本。
- iOS 页面、设置与 Trace：复用信息结构、默认值、生效时机、失败/取消/恢复语义和测试样本。

现有 `Android` 字符串主要表示 Xbox 服务请求的设备画像与认证字段。Android 的 Gradle 工程、Manifest、Kotlin 宿主和 WebRTC AAR 装配属于新增范围。

### 实际移植矩阵

| 能力 | 复用与 Android 落点 | 投入 / 主要风险 |
| --- | --- | --- |
| 登录、续期、账户隔离 | 共享 Rust；Custom Tabs 与 Intent 回调；Keystore 密钥保护本机加密凭据 | 中；回调路由、进程重建、凭据轮换提交 |
| 主机、云目录、我的、控制器、设置及子页 | 复用已有业务接口和状态合同；Compose 实现五根页与手机/平板布局 | 中到高；缓存归属、分页、字号、安全区和系统返回 |
| xCloud/xHome 会话和信令 | 复用现有 Rust scheduler/plan；Android libwebrtc 执行 Peer、SDP、ICE 与通道操作 | 中；SDK 格式差异、异步取消、旧代次回调 |
| 硬解、首帧、稳定显示 | libwebrtc MediaCodec 纹理输出、latest-only mailbox、EGL 原生显示链 | 高；芯片/驱动、队列深度、Surface 重建、1080p60 长尾 |
| 音频、麦克风、后台续接 | Android 音频焦点/路由与前台服务；保留 Rust 会话和原 Peer 的正常续接语义 | 中到高；耳机路由、后台限制、厂商调度、进程回收 |
| 手柄、触屏和 TAK | 共享 wire/IR/资源；KeyEvent/MotionEvent、原生多指执行与绘制 | 中到高；轴差异、短按边沿、释放、震动设备能力 |
| 画面比例、颜色和锐化 | 参数与算法语义复用；接入同一 Android shader/显示链 | 中；色彩与 GPU 预算；高级超分另行验证 |
| 诊断、日志与导出 | schema、预算和事实含义复用；Android writer 与系统分享 | 中；真实呈现时间能力、队列压力、单调时钟对应 |
| 构建与发行 | 同一移动桥接输出 Android `.so` 和 Kotlin 绑定；独立 AAR/签名/包身份 | 中；NDK、所有原生依赖的 16KB 页大小与供应链 |

### 必须保留的边界

桌面继续 Tauri + Vue + TypeScript + Rust；移动公共认证、Xbox API、会话/信令及 wire 合同保持 Rust 权威。Swift 与 Kotlin 各自承担系统适配，移动桥接契约保持一套。Android 原生宿主属于新增平台边界，执行前由 RFC 明确目录、依赖、装配和发行规则；本次评估完成这个方向的技术判断。

网络和恢复继续采用现有标准 WebRTC 与公共监督合同。解码后的旧帧释放、局部资源恢复和高刷新显示沿低延迟目标实现。桌面 `main` 开发构建沿既有暂停规则。

### 当前 iOS 验证成熟度

页面/数据迁移已有自动测试与设备编译证据，TAK 公共编译也有整库验证。11 月 25 日音频、11 月 26 日后台续接，以及近期显示调度的最新改动仍处于实机验收阶段。Android 应继承这些行为目标，并为对应平台提供独立设备证据；既有 iOS 自动测试用于复用合同和样本。

## 3. 路线选择

| 路线 | 价值与成本 | 判断 |
| --- | --- | --- |
| A：Kotlin/Compose + 共享 Rust + Android libwebrtc | 与 iOS 具有相同分层；系统生命周期、硬解、手柄和触屏拥有明确落点；投入集中在 Android 适配 | 推荐，最贴合当前 iOS 规格 |
| B：Vue + Tauri Mobile + Android WebView | 页面有代码复用机会；媒体、控制输入、生命周期与原生插件需要专项评估 | 适合优先追求桌面页面复用的独立提案 |
| C：Rust Native WIP 扩展 Android 媒体宿主 | 有机会共享更深的接收与恢复事实；需要新增 Android 解码、显示、音频和输入装配证据 | 作为后续研究方向，当前工程与验证成本最高 |

## 4. 推荐方向与风险处理

### 统一移动桥接

当前 crate 产物为 `lib/staticlib`，生成脚本调用 Swift 专用 bindgen。UniFFI 0.32 已有 Kotlin 后端；需要补 Kotlin 生成入口、Android 动态库、Gradle/NDK 装配与异步回调验证。Apple `.a`、Swift 绑定和现有 ABI 在分阶段演进中保持兼容，平台产物消费同一份接口与业务实现。

第一轮构建应覆盖 Rust/TLS/压缩依赖及生成绑定所用的全部原生依赖。Android WebRTC 使用独立、固定来源与 commit/checksum 的 AAR。16KB 页大小核验覆盖 Rust `.so`、WebRTC 和绑定依赖；当前 iOS XCFramework 的版本固定可以复用供应链管理方式。

### 低延迟显示优先

推荐从 libwebrtc 的 MediaCodec 硬解纹理输出接入 latest-only mailbox 与 EGL 显示；Compose 用原生 View 宿主承载该显示链。解码队列、纹理所有权、Surface 关闭和重绑定需要有界处理。视频像素和逐帧绘制保持平台热路径，UniFFI 承接粗粒度业务与会话操作。

H.264 档位、分辨率和硬解低延迟模式按实际 decoder 能力和远端接受结果确定。原画、比例、颜色和锐化先进入同一链；MetalFX 对应的超分效果按 Android GPU 能力另行设计和测量。

首帧门槛与 Trace 需要表达实际观测来源。`eglSwapBuffers`、GPU 完成和实际屏幕呈现具有各自时间含义；实际呈现时间按驱动支持采集，其他路径保留来源与能力状态。网络健康、恢复链、media supply、steady supply、submit/present P95/P99 和帧供给 delta 采用相同事实含义验收。

### 生命周期与安全存储

登录沿系统浏览器与 Intent 回调消费公共 PKCE/state。现有 `ms-xal-000000004c20a908` 回调 scheme 在 Android 上需要核对 Intent 路由、其他 Xbox 应用共存、取消和进程重建。Keystore 保存加密密钥，凭据文件按本机加密与备份边界保存；账户 scope 与凭据 revision 继续控制异步提交。

后台音频连续播放通过符合用途的前台服务、持续通知及音频焦点管理实现。服务持有当前会话/Peer，Activity 和 Surface 负责可见窗口。进入后台释放输入、暂停画面与麦克风；回前台重新绑定 Surface，正常路径继续原连接。系统回收进程和真实网络故障通过公共恢复合同创建新的有效代次。

麦克风的权限、前台服务类型与调用时机按实际采集设计；蓝牙控制器使用系统输入设备事件，主动扫描等额外能力按具体 API 增加权限。网络切换、锁屏和厂商电源管理属于设备验收范围。

### 输入、TAK 与原生界面

Android 手柄需要处理不同设备的扳机/摇杆轴、系统保留按钮、设备切换、断开和短按边沿。震动通过设备暴露的振动能力执行，主电机、触发器电机和 UI 触觉分别声明能力；四路震动的实际效果以目标手柄和系统支持为准。

TAK 的协议解析、公共 IR 与资源可以直接复用。多指几何、手势执行、连发计时、输入贡献合并和绘制目前位于 Swift，Android 需要实现对应执行器。官方/社区样本、Context/状态更新、取消与归零作为两端共用验证样本；鼠标、键盘与传感器沿当前 iOS 已暂缓的范围管理。

设计迁移保持品牌、内容层级、五根页、请求状态和操作流程。SwiftUI/Liquid Glass 的视觉按 Compose 与 Android 表面能力表达；触屏目标以 Android 48dp 为基线，文字采用 sp，布局使用窗口安全区和有效宽度，手机与平板保持相同信息语义。页面手柄导航若纳入范围，沿现有几何寻路合同做平台适配。

### 阶段顺序

1. 冻结 Android 首版规格与平台边界，建议从 Android 12+、arm64 手机开始，首轮选择 Snapdragon 与 MediaTek 各一台代表设备。
2. 完成共享桥接 Android 构建与 Kotlin 调用，验证登录、凭据恢复和一个真实数据查询。
3. 建立一个 xCloud/xHome 共用串流入口，优先验证 1080p60/720p 回退、音频、标准触屏及实体手柄、退出与后台恢复。
4. 根据设备证据完成五根页、数据子页、完整设置、TAK 保存/编辑与诊断导出，随后扩大平板与设备矩阵。

## 5. 待定事项

- 首版最低 Android 版本、手机/平板的首批范围，以及需要保证的芯片、刷新率和手柄型号。
- Android WebRTC AAR 的固定来源、与 iOS SDK 的协议/API 差异和 16KB 原生依赖证据。
- 后台连续音频与麦克风的产品范围，以及相应服务、通知和权限行为。
- 分发选择、包身份和 target SDK；商店的版本门槛与 SDK 规则在实施和发行阶段核对。

Android 12+/arm64 是降低首轮设备组合的建议基线，最终支持范围由设备样本和产品需求确定。工作量采用相对判断，周期在垂直串流验证后估算。

## 6. 候选后续任务

| 候选任务 | 类型 | 交付与最小门槛 |
| --- | --- | --- |
| Android 原生宿主与统一移动桥接 RFC | complex | 平台边界、目录、构建/SDK、接口兼容、生命周期和发行身份 |
| Android Rust/Kotlin 构建接线 | complex | 目标 ABI 构建、异步/错误/回调往返及原生库装配冒烟 |
| Android 双目标低延迟串流闭环 | complex | 两类会话、首帧/声音/输入/退出，代表设备的 Trace 与系统恢复 |
| iOS 规格的 Android 页面与能力映射表 | simple | 每项能力的复用入口、Android API、生效时机、回退与样本 |
| Android 数据页面、设置、TAK 和诊断完善 | complex | 当前五根页与手机/平板操作流、同源 IR/输入样本及导出证据 |

后续具体执行由 `task-run` 管理 RFC、任务追踪和验证闭环。建议先完成共享桥接与双目标串流的垂直验证，以此确定 Android 的性能与兼容性预算。

## 依据与本轮验收

- [iOS 最新设计规范](../../iosapp/DESIGN.md)、[当前运行与模块边界](../../iosapp/README.md)。
- [页面与数据迁移报告](../reports/2026-11-17-ios-pc-capability-migration.md)、[串流行为与设置合同](../rfcs/2026-11-19-ios-streaming-pc-behavior-parity.md)。
- [TAK 公共协议引擎报告](../reports/2026-11-26-ios-tak-protocol-engine.md)、[任务状态](../project-task.md)。
- [现有移动桥接配置](../../crates/xbox-ios-bridge/Cargo.toml)、[Swift 绑定生成](../../iosapp/scripts/generate-rust-bindings.sh)、[公共 TAK 编译入口](../../crates/xbox-ios-bridge/src/touch_layouts/runtime.rs)。
- Android 官方参考：[16KB 页大小](https://developer.android.com/guide/practices/page-sizes)、[前台服务类型](https://developer.android.com/develop/background-work/services/fgs/service-types)、[手柄输入](https://developer.android.com/develop/ui/views/touch-and-input/game-controllers/controller-input)、[Keystore](https://developer.android.com/privacy-and-security/keystore)。本轮外网读取受环境限制，SDK/商店最新门槛列入后续核验。
- 本轮采用源码/依赖静态核对与一次文档链接、差异冒烟；既有测试结果按报告引用。源码指纹、验证摘要和产物清单见[本轮证据](../../dev-docs/validation/2026-11-26-ios-to-android-feasibility/README.md)。
