# Streaming Dual Window Overlay Architecture Refresh RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: in-progress
- Owner: native_video / shell 主线（主窗口协调由 Supervisor 持续把关）
- Last Updated: 2026-04-09

## Background

- 当前仓库虽然保留了 `native-video-*` capability 和 `native_video` 原生视频宿主能力，但现行主线已经回到“单主窗口 + 主窗口内视频宿主”的模型。
- 用户对桌面串流窗口方案提出了新的明确要求：
  - 采用双层窗口
  - 只在播放时让 Web 主窗口透明
  - 视频流窗口支持动态调节显示模式，包括保比例、铺满等
  - 视频流窗口无需交互
  - 双层窗口要高度融合，包括外观
- 现状中最接近的基础能力已经存在：
  - `src-tauri/src/mods/native_video/mod.rs` 已具备原生视频宿主与平台特化能力
  - `src/pages/Stream.vue` 已把视频层与交互 chrome 分离
  - `src/streaming/runtime/browser-video-display.ts` 与 `src/player/app/media/PlaybackService.ts` 已有 `contain / cover / fill` 语义
  - `src-tauri/capabilities/default.json` 已允许 `main` 与 `native-video-*` 窗口操作
- 但当前仍缺少一套新的 canonical 双层窗口状态机和 owner 约束，导致历史双窗方向与当前实现出现漂移。

## Code Facts (Ground Truth)

> 目的：约束 RFC “可落地性”，避免写出当前代码/依赖做不到或需要大改的承诺；以下条目以 2026-04-09 仓库源码为准。

- 当前 `native_video` 的 viewport 目标仍固定为主窗口：
  - `NativeVideoViewportTarget` 只有 `MainWindow`，`resolve_viewport_target()` 也直接返回 `MainWindow`。
  - 这意味着“独立视频窗”在现状代码里 **还没有被纳入 viewport target / presenter 的 canonical 路径**，双窗落地必须先扩展 target 与窗口 label 绑定。
  - 参考：`src-tauri/src/mods/native_video/mod.rs`
- macOS 侧已经存在“按窗口 label 配置透明/黑底/圆角裁剪”的实现入口：
  - `configure_macos_window_video_host(app_handle, window_label, transparent_window)` 会同时设置 webview/NSWindow 的 opaque 与 backgroundColor，并在透明时强制 content layer 圆角裁剪（当前硬编码 corner radius 14.0）。
  - 参考：`src-tauri/src/mods/native_video/mod.rs`
- host present 的 Rust-side 观测信号已经存在且更接近“真实呈现”：
  - layer presenter 在首次 present 时会记录 timing event：`stage="first_present"`，并且每次 present 会记录 `stage="sample_presented"`。
  - wgpu presenter 虽无显式 `first_present` 事件，但 telemetry 中的 `present_epoch` 可作为“已 present 过”的事实信号。
  - 参考：`src-tauri/src/mods/native_video/mod.rs`（`run_layer_present_tick` / `MacOsWgpuTelemetry`）
- `wgpu_renderer` 当前只实现了 `aspect-fit` 视口（对应 `contain`），没有 `cover/fill`：
  - 渲染时固定调用 `compute_aspect_fit_viewport(...)` 并设置 viewport/scissor。
  - 参考：`src-tauri/src/mods/native_video/wgpu_renderer.rs`
- 前端 `Stream.vue` 当前把串流页面当作“运行在透明 UI 窗口”处理：
  - `applyStreamUiWindowClass(true)` 在 `onMounted` 总是执行（并在卸载时移除），并且页面本身 `background: transparent;`。
  - 这与本 RFC 的“非播放期主窗不透明”目标 **存在现状差异**，因此需要一个 Rust 侧可控的开关驱动前端切换，而不是页面 mount 即透明。
  - 参考：`src/pages/Stream.vue`

## Goal

- 重新定义并固定串流期 canonical 窗口方案：
  - 上层 `main`：唯一 Web 主窗口，负责 Vue 页面、HUD、菜单、错误态、焦点与输入
  - 下层 `native-video-*`：纯视频流窗口，负责原生视频展示与黑底基底
- 固化以下行为：
  - 仅在视频已经实际显示后，`main` 进入透明叠加态
  - 非播放期 `main` 保持普通不透明桌面应用窗口
  - 视频流窗口支持至少 `contain` 与 `cover` 两种显示模式，并预留 `fill` 扩展
  - 视频流窗口不接业务交互、不抢焦点、不承担路由
  - 双层窗口在位置、尺寸、全屏、阴影/圆角/背景等视觉上保持高度融合
- 把窗口编排 owner 固定在 Rust `native_video` 主线，避免前端分散控制原生窗口。

## Scope

- In scope:
  - `src-tauri/src/mods/native_video/*` 的双层窗口编排、状态机与平台策略
  - `src-tauri/src/shell/*`、`src-tauri/src/lib.rs` 中与窗口事件相关的接入点
  - `src-tauri/src/mods/app_state/*` 中全屏与主窗口状态的双窗语义调整
  - `src-tauri/src/mods/xbxengine/*`、`src-tauri/src/mods/streaming/*` 中对首帧显示、停流和失败回退的合同接入
  - `src/pages/Stream.vue` 及相关前端样式对“透明叠加态”的消费方式调整
  - `docs/project-task.md`、后续 Report 跟踪
- Out of scope:
  - 重写 RTC、解码、播放器或输入主线
  - 为视频流窗口增加交互式 UI 或焦点路由
  - 引入新的桌面运行时或平行原生壳
  - 在本 RFC 中一次性承诺所有平台视觉细节都完成

## Plan

1. 明确双层窗口拓扑、状态机与 owner 边界，替代当前单窗主线下的隐式时序。
2. 在 `native_video` 内实现独立视频流窗口的创建、复用、同步与非交互策略。
3. 以“宿主已稳定 present 首帧”为准触发 `main` 透明叠加，禁止按页面进入或 renderer 收帧提前透明。
4. 为视频流窗口增加显示模式合同，首版支持 `contain` 与 `cover`，并统一黑底与视觉融合策略。
5. 打通停流、失败、全屏、移动/缩放、显示器切换等回退与同步路径，并补足验证与跟踪文档。

## Validation

- [ ] `cargo fmt --all`
- [ ] `cargo check -p xbxrc`
- [ ] `pnpm -s exec tsc -p tsconfig.json --noEmit`
- [ ] 手动验证双窗创建、销毁、位置/尺寸/全屏同步
- [ ] 手动验证播放前不透明、首帧后透明、停流后恢复不透明
- [ ] 手动验证 `contain` / `cover` 显示模式、黑底留白与窗口无交互

## Risks

- 当前前端的 `frameReady` 更接近“renderer 收到帧”，不等价于“视频窗已经稳定显示”，若直接复用会导致过早透明。
- 双窗位置、尺寸、全屏同步如果分散在多个入口处理，容易出现抖动、错位或闪烁。
- macOS 与 Windows 在透明窗、焦点、阴影、装饰、任务栏、全屏行为上差异明显，需要平台特化。
- 视频窗设为无交互后，若层级、focusable、ignore-cursor 等策略组合不稳，可能影响主窗焦点或系统行为。
- 高度融合外观要求主窗和视频窗在圆角、边框、阴影、底色和过渡时机上严格一致，否则会暴露“双窗感”。

## Proposed Architecture

### Window Topology

- `main`
  - 唯一 Web 主窗口
  - 承载所有 Vue 路由、HUD、菜单、设置、诊断与错误态
  - 保留应用焦点入口和输入入口
- `native-video-*`
  - 独立视频流窗口
  - 只承载视频与黑底基底
  - 不加载业务页面
  - 不承担输入交互

### Ownership

- `native_video` 是双层窗口的唯一展示 owner，负责：
  - 视频窗创建/复用/销毁
  - 双窗位置、尺寸、全屏、可见性同步
  - 视频窗显示模式与黑底策略
  - 首帧显示判定与主窗透明切换
  - 失败与回退
- `streaming` / `xbxengine` 只负责：
  - 串流开始、停止、失败、present 相关事件
  - 不直接编排原生窗口
- Vue 前端只消费：
  - 当前是否进入透明叠加态
  - 是否显示 HUD、错误层、设置层
  - 不直接控制独立视频窗

## Window Behavior Model

### Main Window

- 非播放期：
  - 普通不透明窗口
  - 正常背景、正常 WebView 内容
- 播放期但首帧未稳定显示：
  - 仍保持不透明
  - 允许显示 loading / preparing UI
- 播放稳定后：
  - 切换到原生透明 + Web 内容透明叠加态
  - 仅保留必要 HUD 与交互层

### Video Window

- 作为底层窗口存在
- 背景固定黑色，承担留白基底
- 默认居中保比例显示
- 不响应鼠标命中与业务焦点
- 跟随 `main` 的位置、尺寸与全屏变化

## Stream Lifecycle State Machine

1. `Idle`
   - 仅 `main` 可见
   - `main` 不透明
   - 视频窗不存在或隐藏
2. `PreparingWindow`
   - 收到串流启动意图
   - 创建或复用视频窗
   - 先同步双窗位置、尺寸、全屏
   - 视频窗先显示黑底
3. `AwaitingFirstPresent`
   - 播放链路已启动
   - 等待宿主确认至少一帧已稳定 present
   - `main` 仍不透明
4. `OverlayActive`
   - 视频窗已可见且 present 稳定
   - `main` 开始切换原生透明与 Web 透明
5. `Streaming`
   - 双窗稳定运行
   - HUD 叠加在透明主窗上
6. `Stopping`
   - 用户退出、会话关闭或失败
   - 先撤销 `main` 透明，再隐藏或销毁视频窗
7. `Failed`
   - 任一关键步骤失败
   - 立即恢复 `main` 不透明，视频窗隐藏或销毁

## Transparency Contract

- 透明切换必须由 Rust 侧统一驱动，而不是由前端页面进入时机或普通 `frameReady` 推断。
- 触发 `main` 透明的前置条件：
  - 视频窗已创建并完成一轮布局同步
  - 视频 presenter 已确认至少一帧真实显示
  - 当前窗口状态未处于失败或停止流程
- 停流或失败时恢复顺序固定为：
  1. 撤销 Web 透明样式
  2. 撤销 `main` 原生透明
  3. 隐藏或销毁视频窗

### First Present Stability Gate

- “开始播放”与“允许透明”不是同一个事件，至少需要区分：
  - `firstFrameObserved`：渲染链路首次收到可用帧
  - `firstFramePresented`：宿主至少成功 present 一帧
  - `firstFrameStable`：present 已跨过最小稳定门槛，可以安全切主窗透明
- 首版建议以 `firstFramePresented + 短稳定窗口` 作为透明触发条件，避免“刚有首帧就透明”导致黑底、桌面或错位短闪。
- 必须定义首帧超时：
  - 若超时，`main` 保持不透明
  - 视频窗可隐藏或进入错误态
  - 不允许停留在“视频窗已出现但主窗半准备”的中间状态

#### Stable Gate: 可落地的首版实现建议（基于现有 telemetry）

> 本段刻意以现有 Rust 实现可直接获得的数据为准，避免依赖前端 `frameReady` 或浏览器 `requestVideoFrameCallback`。

- **数据来源**（现状已存在）：
  - `NativeVideoViewportState` 已包含 `latest_host_present_time_ms`、`host_present_epoch`、`host_present_fps` 等字段（由 presenter 侧 telemetry 注入）。
  - macOS layer presenter 还会产生 `hostTiming` 事件中的 `stage="first_present"` / `stage="sample_presented"`（用于追踪与调试，不要求前端消费）。
  - 参考：`src-tauri/src/mods/native_video/mod.rs`
- **初始实现建议值**（不是长期产品合同，可按平台验证结果调整）：
  - `FIRST_PRESENT_TIMEOUT_MS = 8_000`
  - `STABLE_PRESENT_MIN_EPOCH = 2`
  - `STABLE_PRESENT_MIN_FPS = 8.0`（以 `host_present_fps` 判断“不是单帧闪现”）
  - `STABLE_PRESENT_GRACE_MS = 120`（给透明切换与双窗几何同步留最小缓冲，减少黑底/错位一闪）
- **判定逻辑**（简化且可测试）：
  - `presented = host_present_epoch >= 1`
  - `stable = host_present_epoch >= STABLE_PRESENT_MIN_EPOCH && host_present_fps >= STABLE_PRESENT_MIN_FPS`
  - `stable` 额外要求：`now_ms - latest_host_present_time_ms <= 500`（防止已经断流/停摆仍误判稳定）
  - 只要超时未达 `presented`，则走 `Failed`/错误态回退，主窗保持不透明（不进入透明中间态）。
- **约束说明**：
  - `host_present_epoch / host_present_fps / latest_host_present_time_ms` 当前本质上属于 presenter telemetry 投影，RFC 只把它们作为首版实现的可复用事实来源，不把上述阈值提升为长期不可变公共 API。
  - 若后续平台验证表明这些阈值会误伤低帧率、菜单态或恢复态，则允许在不改变整体状态机语义的前提下调整实现参数。

### Rust -> Frontend 的透明开关输出（最小新增合同）

> 现状前端 `Stream.vue` 会在页面 mount 时默认把页面当作透明窗口处理。本 RFC 要求“只在 stable present 后透明”，因此需要一个 Rust 侧“单一真相”输出给前端做 class/style 切换。

- **新增一个只读事件**（不新增“前端控制窗口”的通道）：
  - event name：`nativeVideo.overlayStateChanged`
  - payload：
    - `phase`: `Idle | PreparingWindow | AwaitingFirstPresent | OverlayActive | Streaming | Stopping | Failed`
    - `mainTransparent`: boolean
    - `reason?`: string（仅用于失败/回退诊断）
    - `tsMs`: number
- **前端消费方式**（最小改动策略）：
  - `Stream.vue` 不再在 `onMounted` 无条件 `applyStreamUiWindowClass(true)`；
  - 改为订阅事件后按 `mainTransparent` 打开/关闭 `stream-ui-window` class（并对应调整页面 overlay 背景策略，确保非透明期仍可读）。
- **约束**：
  - 前端不得反向发指令控制 `main` 透明与否；透明切换严格由 `native_video` 状态机驱动（见上节）。
  - 该事件是“主窗透明态”的唯一 canonical 前端合同；前端不得同时以 `frameReady`、session lifecycle 或其他 UI phase 推断透明态。
  - 事件归口为 `native_video` 对外状态投影；如果后续需要经 `streaming`/shared contract 做桥接，也必须保持单一真相，不得形成并行状态源。

### Recovery Transparency Policy

- 恢复链路必须定义“是否继续保持透明”：
  - 短暂网络抖动或轻量 reconnect，可保持透明，只显示弱侵入 HUD
  - 严重恢复或首帧再次丢失，应回退为主窗不透明 loading/error 态
- 该策略必须由 Rust 侧状态机统一裁决，不允许前端单独凭 UI phase 推断。

## Display Mode Contract

- 产品语义沿用已有 `contain / cover / fill` 认知，但渲染 owner 改为 `native_video`。
- 首版要求：
  - `contain`：保比例居中，留白黑底
  - `cover`：保比例铺满，允许裁切
- 预留：
  - `fill`：强制拉伸铺满
- 当前 `wgpu_renderer.rs` 只实现了 aspect-fit 视口，后续需要扩成可切换模式，而不是始终 `aspect-fit`。

### Display Mode Details

- 除模式枚举外，还需要补充以下合同：
  - 模式切换必须支持播放中动态生效
  - `cover` 的裁切锚点首版固定为中心，后续如有需要再扩展
  - 超宽屏、竖屏或异常比例输入源都必须有确定行为，不能退回未定义布局
  - 黑底留白属于视频窗自身职责，不能依赖主窗 Web 背景兜底
- 配置维度需要提前决定：
  - 首版默认全局配置
  - 后续可扩展为“按设备”或“按标题”记忆，但不在本次首版范围内

## Non-Interactive Contract

- 视频窗必须满足：
  - 不参与业务焦点
  - 不接键鼠命中
  - 不接游戏内菜单交互
  - 不承载 Web UI
- 主窗继续作为唯一输入入口，保证现有 gamepad navigation 与 HUD 行为不分叉。

### Input And Focus Rules

- 视频窗不仅“无需交互”，还需要明确：
  - 是否完全点击穿透
  - 是否允许系统级焦点短暂落到视频窗
  - 是否在任何场景下接收键盘事件
- 首版建议：
  - 视频窗完全不参与业务焦点
  - 视频窗对业务鼠标命中采用点击穿透语义
  - 所有业务键鼠/手柄输入统一由 `main` 接收
- 系统级快捷键需要明确归属：
  - `Esc`
  - 全屏切换
  - 调试快捷键
  - 开发者工具开关
- 若平台上“不可聚焦 + 点击穿透”组合不稳定，优先保证主窗输入正确，其次再追求更强的视频窗隔离。

## Visual Integration Strategy

- 双窗“高度融合”在本 RFC 中定义为以下要求：
  - 相同的窗口外框几何：位置、尺寸、圆角、全屏状态
  - 相同的视觉边界：阴影、边缘裁剪、背景过渡时机
  - 相同的生命周期切换：出现、隐藏、首帧后切透明、停流后恢复
- 实现上需要统一：
  - 主窗透明时的圆角和 content 裁剪
  - 视频窗黑底、圆角与阴影策略
  - 两窗的 show/hide 与 resize 节拍

### Visual Tokens

- 为避免“双窗感”，建议把以下视觉参数沉淀为统一 token，而不是分别硬编码：
  - 圆角半径
  - 阴影大小与透明度
  - 边缘裁剪策略
  - 显示/隐藏动画时长
  - 首帧后切透明的过渡时长
- 如果后续需要更强的一体感，可考虑给主窗保留极轻的材质层或 HUD scrim，而不是完全裸透明。

## Window Sync Matrix

- 双窗同步不能只覆盖位置与尺寸，还必须覆盖：
  - 最大化 / 还原
  - 进入 / 退出全屏
  - 多显示器切换
  - DPI / scale factor 变化
  - 主窗拖动中的实时跟随
  - 主窗隐藏 / 显示 / 最小化
- 同步策略原则：
  - 优先由 `main` 作为几何主源
  - 视频窗被动跟随
  - 同步失败时优先回到“主窗可用 + 视频窗隐藏”的安全态

## Window Labels & Creation Contract (Tauri v2)

> 现状 capability 已允许 `main` 与 `native-video-*` 窗口操作（`src-tauri/capabilities/default.json`），因此 label 约定需要尽早固定，避免后续出现“同名窗口复用/泄漏/残留”。

- **label 约定**：
  - `main`：既有主窗口（固定）
  - `native-video-stream`：串流视频窗（首版仅需一个）
  - `native-video-*`：保留给未来（例如诊断/实验窗），但首版只承认 `native-video-stream` 作为 canonical 视频窗
- **创建/复用原则**：
  - 若 `native-video-stream` 已存在：优先复用（避免频繁创建导致闪烁与资源抖动）
  - 若不存在：由 Rust `native_video` 在进入 `PreparingWindow` 时创建
  - 任何失败：必须回收或隐藏 `native-video-stream`，并确保 `main` 恢复不透明（安全态）
- **与现状 `native_video` 代码的对齐点**：
  - macOS 的 `configure_macos_window_video_host(app_handle, window_label, transparent_window)` 已支持按 label 配置透明/黑底与圆角裁剪，可直接复用到 `native-video-stream`（并将 `transparent_window=false` 作为视频窗默认）。
  - `ensure_wgpu_host_view(...)` 已显式区分 `window_label != "main"` 的分支来创建独立 host view，这为“独立视频窗承载 wgpu presenter”提供了现成落点。
  - 参考：`src-tauri/src/mods/native_video/mod.rs`

## Window Sync: 单入口监听与去抖策略（可实现约束）

> 抖动/错位的根因通常是“多入口监听 + 互相 set 导致循环”。首版必须强制单入口与循环保护。

- **几何主源**：永远以 `main` 为准（position/size/fullscreen/maximized）。
- **监听入口唯一化**：
  - 只在 Rust 侧集中订阅 `main` 的窗口事件（Tauri v2 的窗口事件流），由 `native_video` 统一转译为一次“视频窗同步动作”。
  - 禁止在其他模块（`streaming` / `xbxengine` / 前端）额外监听并同步 `native-video-stream`。
- **循环保护**：
  - 同步 `native-video-stream` 时必须携带“本轮同步 epoch”，并在视频窗回调事件里忽略同 epoch 的反射事件（或视频窗事件不参与同步决策）。
- **去抖/节流**（首版建议固定值，避免引入配置复杂度）：
  - 拖动/resize 期间：以 16ms~33ms 节流同步（更保守可 33ms）
  - 结束事件：再做一次最终对齐（确保边界一致）
- **失败回退**：
  - 任意同步动作失败（set_position/set_size/set_fullscreen 等）：先记 telemetry，并进入“可重试同步失败”分支，而不是立即隐藏视频窗。
  - 只有在短时间内连续失败、或命中明确不可恢复错误时，才升级到隐藏视频窗并回到安全态。
  - 原则是优先避免“轻微瞬时失败 -> 用户可见黑闪”的过度升级，同时仍保证连续失败不会长期暴露半同步状态。

## Platform Notes

### macOS

- 重点关注：
  - 透明窗圆角裁剪
  - Spaces / 全屏桌面切换
  - 失焦后的层级与阴影
  - 原生 content view / host view 插入时机

### Windows

- 重点关注：
  - DWM 阴影与无边框窗外观
  - DPI / 多屏坐标一致性
  - 任务栏、最大化与全屏边界
  - 透明叠加时的系统合成成本

### Cross-Platform Principle

- 不要求 macOS 与 Windows 使用完全相同的底层细节。
- 要求两端收敛到相同产品语义：
  - 非播放期不透明
  - 播放稳定后透明叠加
  - 视频窗纯展示、无交互
  - 双窗在视觉和几何上足够一体

## Failure And Edge Cases

- 必须明确以下场景的回退策略：
  - 首帧超时
  - 串流启动失败
  - 播放中断流 / reconnect
  - 主窗被关闭、隐藏、最小化
  - 系统睡眠 / 唤醒
  - 显示器拔插或主显示器变化
  - 分辨率 / DPI 热切换
  - 应用崩溃后遗留视频窗
- 安全态原则：
  - 任一关键路径异常，优先保证 `main` 恢复为普通不透明应用窗
  - 视频窗残留必须能被主动隐藏或回收

## Observability

- 建议增加窗口编排 telemetry，至少记录：
  - 视频窗创建耗时
  - 首帧 observed / presented / stable 时间点
  - 主窗透明切换耗时
  - 双窗同步失败次数
  - 因异常回退到单窗安全态的次数
- 这些指标用于验证双窗不是“能跑”，而是“切换稳定且无明显闪烁/错位”。

## Product Flags

- 需要尽早决定是否提供以下产品开关：
  - 是否允许关闭双窗，回退兼容单窗模式
  - 是否允许关闭“播放时主窗透明”
  - 是否允许在播放中快速切换 `contain / cover`
- 首版若不暴露给用户，也建议在内部保留受控开关，便于灰度与问题回退。

## Implementation Stages

### Stage 1: 双窗骨架

- 恢复独立 `native-video-*` 视频窗主线
- 完成创建、复用、销毁、位置/尺寸/全屏同步
- 保持 `main` 不透明

### Stage 2: 首帧后透明切换

- 由 `native_video` 输出稳定的 host-present 信号
- 引入 `PreparingWindow -> AwaitingFirstPresent -> OverlayActive` 状态机
- 播放期才透明，失败和停流必须可回退

### Stage 3: 显示模式

- 扩展视频窗显示模式到 `contain` / `cover`
- 统一黑底策略
- 与现有展示设置项对齐

#### Stage 3 的代码事实约束（避免空想）

- `wgpu_renderer` 当前只有 `compute_aspect_fit_viewport`（`contain`）。因此：
  - `contain` 可以作为首个落地模式，优先确保双窗与透明切换稳定；
  - `cover/fill` 必须通过新增 viewport 计算函数（例如 `compute_aspect_cover_viewport` / `compute_fill_viewport`）或改 shader 采样策略来实现；首版建议先走“viewport 级别的 cover/fill”（scissor + viewport）而不是引入更复杂的 UV 裁切。
  - 参考：`src-tauri/src/mods/native_video/wgpu_renderer.rs`

### Stage 4: 非交互与外观融合

- 收口 focusable、cursor hit-test、skip taskbar、shadow、decorations 等平台策略
- 对齐双窗圆角、阴影、背景与动画时机

### Stage 5: 异常与平台打磨

- 验证首帧超时、停流、失败、显示器切换、主窗关闭、全屏切换等回退
- 在 macOS / Windows 上分别修正平台差异

## Progress

- [x] Step 1: 重新梳理需求并对齐当前代码、旧 RFC 与现状偏差
- [ ] Step 2: 在 `native_video` 内落双窗骨架与状态机
- [ ] Step 3: 接入主窗透明切换合同
- [ ] Step 4: 接入显示模式与非交互策略
- [ ] Step 5: 完成验证并补最终 Report

## Execution Notes

- Date: 2026-04-09 | Status: planned
- Update: 基于当前仓库重新规划双层窗口方案，确认现有能力可复用，但现行实现仍是单窗主线，因此新增本 RFC 作为新的 canonical 方案。
- Decision: 采用“上层 `main` Web 主窗口 + 下层 `native-video-*` 纯视频流窗口”的双层窗口；窗口编排 owner 固定在 Rust `native_video`，透明切换以宿主 stable present 为准。
- Decision: 视频窗首版至少支持 `contain` 与 `cover`；仅在播放稳定后让 `main` 透明，非播放期始终保持普通不透明应用窗口。
- Decision: 补充固定了首帧稳定门槛、恢复期透明策略、输入/焦点/点击穿透、同步矩阵、平台差异、异常回退与产品开关等实现前合同，避免后续边做边漂。
- Decision: `nativeVideo.overlayStateChanged` 被定义为前端透明态唯一 canonical 合同；`frameReady` 等其他信号不得再参与透明判定。
- Decision: 首帧稳定阈值目前只作为首版实现建议，不作为不可变公共参数；同步失败采用“先重试、后升级回退”的分级策略。
- Risk/Blocker: 当前 `frameReady` 信号不足以直接作为主窗透明触发；需要补一条更接近 host present 的 Rust 侧合同。
