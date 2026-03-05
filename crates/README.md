# OhMyGamepad Workspace

当前根级 Rust 工程采用 Cargo workspace，`ohmygamepad` 相关 crate 已按 RFC 风格收拢到 `crates/ohmygamepad/` 目录，并继续按 `core / backends / bridge / tools` 分层：

- `ohmygamepad-protocol`
  - Rust 与 TypeScript 共享的输入协议定义，原 `dto` crate 已按 RFC 收敛重命名为 `protocol`
- `ohmygamepad-core`
  - 手柄输入内核骨架，承接发现、采样、映射、过滤、路由；当前目录已继续收敛为 `api / model / mapping / filter / runtime`，并显式补出 `api::{InputProvider,HapticsProvider}` 与 `runtime::DesktopDriverSelector`
- `bridge/ohmygamepad-host`
  - `ohmygamepad` 域内的单实例 owner 层，负责在当前进程中懒初始化并共享唯一 `OhMyGamepadService`
- `backends/ohmygamepad-gilrs`
  - 桌面手柄后端、`OhMyGamepadService` 门面与 runtime 入口，当前已接入真实 `gilrs`，并支持多手柄采样策略、键盘 fallback 与外部模拟输入；服务内的键盘监听与 rumble 判定逻辑也已拆到独立内部模块
- `bridge/ohmygamepad-bridge-napi`
  - 旧的 Electron/Node 专用 N-API bridge，当前保留为禁用兼容路径，不再作为主宿主集成入口
- `backends/ohmygamepad-hid-dualsense`
  - DualSense 高级触觉 backend 占位 crate，当前统一返回 `Unsupported`
- `backends/ohmygamepad-win-xbox-haptics`
  - Windows Xbox 高级震动 backend 占位 crate，当前统一返回 `Unsupported`
- `xbxengine-protocol`
  - `rust-owned` xbxEngine 控制面与宿主桥共享的 DTO/命令语义
- `xbxengine`
  - `rust-owned` 实时 runtime 骨架，承接连接状态机、媒体链路、输入与恢复逻辑

当前阶段已经打通 Rust 内部的 DTO、输入内核、桌面 `gilrs` 后端、单实例 `ohmygamepad-host`、采样预设、多手柄采样策略、键盘 fallback、默认键盘映射与外部模拟输入；当前主宿主路径已经收敛为 `xbxengine-api` 暴露统一 N-API，Electron main 不再继续直接加载 `ohmygamepad-bridge-napi`。

同时 RFC 里提到但尚未正式实现的几块当前仍保留为占位工程：

- `backends/hid-dualsense`
- `backends/win-xbox-haptics`

这些 crate 当前只保证目录结构、workspace 编译与基础接口存在，真实平台依赖和功能后续再继续下沉。

当前 `ohmygamepad` Rust 侧面向宿主的桥接层为：

- `xbxengine-api`
  - 导出 `XbxEngineNativeBinding` 与 `XbxEngineGamepadNativeBinding`
  - 宿主当前统一从 `target/debug` / `target/release` 动态加载 `xbxengine_api`
  - 当前已同时暴露 streaming 控制面与 gamepad 的 `runtime snapshot`、`route target`、`sampling`、多手柄采样策略、主要采样设备、pause/resume 采样设备以及 rumble API
- `ohmygamepad-bridge-napi`
  - 保留为禁用兼容路径，当前不在 Electron 主路径加载
- `xbxengine`
  - 通过 `ohmygamepad-host` 复用同一个 gamepad owner，不再直接持有独立 `OhMyGamepadService`

`ohmygamepad-core` 当前也开始显式对齐 RFC 的 provider/selector 边界：

- `api::InputProvider`
  - 对外表达“输入来源”的稳定 trait 边界，当前先桥接既有 `InputBackend`
- `api::HapticsProvider`
  - 对外表达“震动/高级触觉输出”的稳定 trait 边界
- `runtime::DesktopDriverSelector`
  - 统一决定桌面端应该选择哪类 input/haptics provider
  - 当前默认选择 `gilrs + basic haptics`

`ohmygamepad-core` 当前内部目录已收敛为：

- `api`
  - 对外暴露 provider trait 与稳定入口
- `model`
  - 承载 backend/config/profile/sink 等基础模型
- `mapping`
  - 承载原始输入到 logical pad 的映射逻辑
- `filter`
  - 承载 deadzone、trigger 等输入过滤逻辑
- `runtime`
  - 承载 engine、runner、selector 与 snapshot 订阅

`rust-owned` 的首阶段实现策略也已经收敛：

- 输入继续复用 `ohmygamepad-*`
- gamepad owner 当前固定为 Rust 侧单实例 `ohmygamepad-host`
- 宿主桥固定为 `Electron + N-API`
- transport 主线固定为 `webrtc-rs`
- 当前 active 视频链固定为 `openh264` 软解 + 最小 headless `wgpu` render backend
- render 目标固定为 `wgpu + winit`
- renderer 的 `RustOwnedRuntime` 仅作为 Rust runtime client 壳存在
- `xbxengine-app` 当前仅作为 Rust 原生窗口 render 验证宿主，不代表 Electron 产品宿主回退
- `xbxengine-app --stdio` 仅作为外部宿主信令接入的验证协议保留，不再是 Electron 产品集成主线；需要联调时可由 Electron main 在 `XBXENGINE_BINDING=stdio` 下拉起，并用 `XBXENGINE_APP_PATH` 覆盖二进制路径
- Electron 开发期默认也已切回 `xbxengine-api` N-API；仅在显式使用 `pnpm run dev:stdio` 或设置 `XBXENGINE_BINDING=stdio` 时，才走 `xbxengine-app --stdio`

`OhMyGamepadService` 当前额外提供了两类更稳定的外部入口：

- 查询与发现
  - `snapshot`
  - `list_devices`
  - `discover_devices`
  - `get_device`
- 命令式 facade
  - `apply_command(OhMyGamepadServiceCommandDto)`
  - 采样策略、主要采样设备、虚拟手柄连接/断开、键盘 fallback 输入、键盘映射替换、外部模拟态注入都走同一套 DTO
- 默认键盘映射
  - `OhMyGamepadKeyboardMapper::with_default_mapping()`
  - 默认提供 `W/A/S/D`、方向键、`J/K/U/I`、`1/2/3/4`、`Tab/Enter/7/8/9` 这套逻辑 pad 键位
- 内建桌面键盘监听
  - `OhMyGamepadServiceConfig::default()` 默认会启用 `device_query` 驱动的桌面键盘监听
  - 只有在 `sampling_strategy.enable_keyboard_fallback = true` 时才会启动内部键盘线程
  - 如需自行控制监听节奏，可将 `desktop_keyboard` 设为 `None`，再手动使用 `OhMyGamepadDesktopKeyboardListener`

其中虚拟设备连接/断开与模拟输入提交会在服务层做一次短暂同步等待，尽量保证查询接口观察到的状态与刚提交的命令一致，减少测试和宿主层接入时的时序抖动；但键盘 fallback 在被真实手柄抑制时不会再做无意义的同步等待。

Rumble 当前也已经有了稳定 API：

- `OhMyGamepadService::play_rumble`
- `OhMyGamepadService::stop_rumble`
- `xbxengine-api` 当前在主宿主路径同步提供 `playRumbleJson` / `stopRumbleJson`
- `ohmygamepad-bridge-napi` 保留同名导出，但当前默认禁用

现阶段它们会先做 target 解析和能力判定，返回结构化 `accepted/reason/resolved_device_ids` 结果；真实桌面硬件 rumble 输出后续再接到 `gilrs` / 平台特化 backend。
