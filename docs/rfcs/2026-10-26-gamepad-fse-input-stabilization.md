# Windows 掌机 FSE 手柄输入稳定化：单源采样 + 常驻采样 + 单一路由 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: ✅ **Phase 1-5 全部完成**
- Current State: complete（待实机验证）
- Owner: claude
- Last Updated: 2026-10-26

## Background

### 现象

Windows 掌机 FSE（Full Screen Experience）下手柄输入反复不稳定，典型三类：

1. **启动无输入**：冷启动进入应用后手柄无响应，切一次多任务 / 碰一下屏幕才可能恢复。
2. **触屏后无输入**：触屏、软键盘、系统覆盖层出现后手柄输入消失。
3. **前后台切换不稳定**：最小化 / 切走再回来，输入时有时无，偶发卡键。

### 历史

2026-04 至今，手柄链路已经走过 6 份 RFC / 报告和十余次针对性修复，每次修一个现象、留下一层补偿：

- `04-22` SDL3 单轨统一（决定移除 XInput fallback）→ `06-29` 又把 XInput fallback + XInput direct poller 加回来，实际变成双轨。
- `05-11` 引入 `Active / BackgroundWarm / Suspended` 三态和 stalled 自愈 → 后续不断修「lifecycle 停在 BackgroundWarm 导致门开着但没数据」。
- `05-12` 加 8ms 轮询快照兜底「SDL 事件流钝住」→ 事件增量与全量快照两条路径并存。
- `05-14` FSE 专线（`GamingExperience.dll` + Win32 前台判定）→ `10-22` 删除，gate 退化后又全屏无输入 → `10-25` 收敛为「只认最小化」。
- `10-22` 触屏不再强制重开设备。

结论：问题不在单个分支判断，而在**结构本身**——采样来源、采样生命周期、健康度推断、自愈、gate、路由六套状态机叠加，任何一个组合没覆盖到就表现为「无输入」。

### 根因分析（基于当前代码）

| # | 机制 | 证据 | 对应现象 |
|---|---|---|---|
| R1 | **Windows 同一物理手柄被三路来源写入**：SDL 事件增量 + 8ms 快照（优先按 SDL `player_index` 调 `XInputGetState`）+ `XInputDirectPoller` 合成 `xinput:user:N`。SDL 用 RawInput/HIDAPI/WGI 驱动时 `player_index` 不等于 XInput user，快照可能读错设备或在 SDL/XInput 间来回切；三路都写同一个 `TrackedDevice`，增量与全量互相覆盖 | `crates/ohmygamepad/backends/sdl3/src/source.rs` `capture_gamepad_baseline_state` / `XInputDirectPoller`；`backend.rs` `update_button` / `sync_snapshot` | 方向/按键闪烁、偶发无输入 |
| R2 | **前后台切换必然重开全部设备**：最小化 → `BackgroundWarm`；恢复 → `refresh_gamepad_on_window_foreground` → `resume_shell_sampling` → `perform_resume_recovery` → `reopen_devices_and_prime_sampling`（关闭并重开所有 SDL 句柄 + 重新枚举）。FSE 下枚举本就不稳，重开一次就可能丢设备 | `src-tauri/src/shell/gamepad_sampling.rs`；`backends/sdl3/src/service.rs` `perform_resume_recovery` / `prime_and_refresh_runtime_sampling` | 前后台切换不稳定 |
| R3 | **启发式健康度驱动自愈**：`AwaitingBaseline / Stalled` 靠时间阈值 + seq 推进推断，分不清「用户没碰手柄」和「采样冻住」；判定后执行 reopen。入口有订阅环每帧检查、前台刷新、启动 900ms rearm、AppResumed 四处 | `core/src/runtime/runner.rs` `evaluate_sampling_health`；`src-tauri/src/shell/mod.rs` 订阅环 / `init_services` | 启动无输入、无故断输入 |
| R4 | **采样生命周期与输入可用性耦合**：`BackgroundWarm` 下不发布 slot、不推 stream；任何一次 lifecycle 没被提升回 `Active`，表现就是「gate 开着但没数据」 | `core/src/runtime/engine.rs` `refresh_pad_snapshots`（仅 `Active` 才 emit）；订阅环里的 `samplingPromotedOnForeground` 补丁 | 启动无输入、切回无输入 |
| R5 | **热路径阻塞跨线程调用**：串流输入环跑在媒体 tokio runtime 上，每 8ms 调 `host.snapshot()`，内部是对 runtime 线程的无超时 `recv()`，还可能触发 rebind；Tauri 订阅环在 async 任务里阻塞 `rx.recv()`，并同步执行 reopen / self-heal（含 100～150ms ack 超时），环一慢无界通道就积压 | `crates/xbxengine/core/src/transport/rtc/stack/input_loop.rs` `collect_gamepad_frames`；`core/src/runtime/runner.rs` `get_runtime_snapshot`；`src-tauri/src/shell/mod.rs` `bind_background_tasks` | 输入抖动、UI 收到陈旧快照 |
| R6 | **默认 Merged 绑定**：所有已连接设备（含掌机内置 XInput 视图、Steam 虚拟手柄、重复识别设备、`virtual:keyboard`）合并进 Pad0，去重靠 path/serial/name 启发式 | `backends/sdl3/src/service.rs` `build_binding` | 幽灵输入、卡键 |
| R7 | **输入路由分散在 Rust 与 JS 两侧**：gate（Rust）+ lifecycle + suspended + `stream_pad_forwarding`（Rust）+ owner / overlayCapturing / 等待归位（JS）。路由决策在 JS 中会被 WebView 节流影响；gate 关闭或 forwarding 关闭时串流环直接停发，**不发中性帧**，后台时按住的键可能在云端一直按着 | `src/shared/gamepad/business-input-arbiter.ts`；`src/pages/stream/stream-input-route-controller.ts`；`input_loop.rs` | 前后台切换卡键 |

## Goal

把手柄链路收敛成**一条直线、没有恢复逻辑也能工作**的结构：

1. **单源**：每个物理手柄只有一个权威采样来源，且该来源不依赖窗口焦点。
2. **常驻**：采样从进程启动到退出始终运行，不存在「采样暂停 / 保温 / 待恢复」状态；前后台只影响**路由**，不影响**采样**。
3. **无启发式自愈**：删除所有基于健康度推断的 reopen / prime / rearm；设备句柄只在明确的热插拔事实或用户手动「重新扫描」时变化。
4. **单一路由**：Rust 持有唯一 `InputRoute`，任何离开 Stream 或 gate 关闭的切换都补发中性帧。
5. **热路径无阻塞**：串流输入环和 UI 推送都以无锁方式读取最新 pad 状态。

完成后预期：FSE 冷启动首键即响应；触屏、覆盖层、前后台切换不再触发任何设备操作；不再出现卡键。

## Non-Goals

- 不改用浏览器 Gamepad API（后台/遮挡下不可靠，且违反 AGENTS.md「不把 Rust 侧系统逻辑搬到 TS」）。
- 不改云端输入协议、stick Y 轴坐标合同、rumble 请求合同。
- 不改键盘 fallback（`virtual:keyboard`）的映射与监听实现，只调整其绑定策略。
- 不改几何寻路导航引擎。
- 不处理 macOS 特有问题（SDL 非主线程等），仅保证本次改动不回退 macOS。

## Design

### 设计原则

- **故障默认放行**：gate 宁开勿关（沿用 `10-25` 的原则）。
- **状态而非事件**：source 只上报全量状态快照；事件仅用于热插拔。
- **没有证据不做恢复**：任何恢复动作必须由可观测的事实触发（设备 removed/added、用户操作），不允许由时间阈值推断触发。

### 1. Source 层：每设备单源

**Windows 采样来源（Phase 0 实机 spike 二选一，默认推荐 A）**

- **方案 A（推荐，SDL 单轨）**：Windows 上通过 SDL hints 让 Xbox 类设备只经 SDL 内置 XInput 驱动接入（`SDL_JOYSTICK_RAWINPUT=0`、`SDL_JOYSTICK_WGI=0`、`SDL_JOYSTICK_HIDAPI_XBOX=0`，保留 `SDL_JOYSTICK_XINPUT=1`、HIDAPI 处理 PS/Switch 等非 XInput 设备）。XInput 轮询不受窗口焦点影响。删除自研 XInput fallback 和 direct poller，与 `04-22` 单轨决策一致。
- **方案 B（兜底）**：自研 `XInputSource` 独占 XInput 类设备（user 0..3），SDL 打开设备时按 GUID 驱动标识 / 设备类型过滤掉 XInput 类设备，只负责非 XInput 设备。

两个方案都满足「每设备单源、焦点无关」。spike 的取舍标准：FSE 冷启动、触屏、切后台三个场景下，trace 中设备是否始终在线、样本是否持续推进。

**统一的 source 行为**

- 只产出 `Connected / Disconnected / Snapshot{全量状态}` 三类事件；删除 `ButtonChanged / AxisChanged / PrimeSnapshot` 和 `PrimeSampling / ReopenDevicesAndPrimeSampling` 命令。
- 已连接设备固定 4ms（250Hz）轮询全量状态；**未连接的 XInput user 每 1s 探测一次**（避免对空槽位高频调用 `XInputGetState` 带来的已知开销）。
- 周期性重新枚举保留 1s 一次，作为热插拔兜底，但只做「新增 / 移除」，不重开已在线设备。
- 新增「重新扫描手柄」命令，只由设置页按钮触发，是唯一允许重开设备句柄的入口。

### 2. Runtime 层：常驻采样

- 删除 `OhMyGamepadSamplingLifecycleDto`（`Active / BackgroundWarm`）、`set_suspended`、`absorb_current_state_as_baseline`、`reset_state` 的 suspend 触发路径。runtime 始终 poll + sample + publish。
- 删除 `evaluate_sampling_health` 驱动的行为。`last_sample_progress_at_ms` 等字段保留为**纯诊断**（写 trace / 面板展示），不驱动任何动作；协议字段在 Phase 5 删除或标记 deprecated。
- 删除 service 层 `perform_resume_recovery`、`try_stalled_sampling_self_heal`、`try_startup_sampling_self_heal`、`SAMPLING_SELF_HEAL_COOLDOWN` 等全部自愈代码。
- 默认绑定从 `Merged` 改为 `LastActiveFailover`：Pad0 跟随最近有输入的单个设备，保留用户指定主设备的 `FixedDevice` 选项。
- `observed_at_ms` 统一由 source 用单调时钟写入，删除 core 里对 SDL 纳秒 / UNIX 毫秒混用的归一化补丁 `normalize_backend_observed_at_ms`。

### 3. 发布层：无锁最新状态

- runtime 线程每次 sample 后，把各 slot 的 `LogicalPadSnapshot` 写入共享的最新状态单元（`tokio::sync::watch` 或 `ArcSwap`），附带 `sample_seq`。
- 串流输入环直接读取最新状态单元，不再调用 `host.snapshot()`，不再与 runtime 线程做命令往返。
- UI 推送仍按 60Hz 节流，改为独立线程消费 `watch`；Tauri 订阅环只负责转发事件，不再执行任何恢复或自愈。
- `host.snapshot()` 保留给 RPC / 设置页等低频场景使用。

### 4. 路由层：Rust 单一 `InputRoute`

```
InputRoute = Blocked | Ui | Stream

foreground  = !minimized（macOS 额外要求 visible）   // 沿用 10-25 判定
route =
  !foreground                         → Blocked
  stream_session_active && !ui_capture → Stream
  otherwise                           → Ui
```

- `stream_session_active` 由 xbxengine 现有 `set_stream_pad_forwarding` 驱动（重命名为语义化接口）；`ui_capture` 由前端 overlay / 菜单打开时通过 RPC 上报。
- 前端 `business-input-arbiter` 改为**镜像**后端下发的 `route`，不再自行派生 owner。
- **中性帧**：route 离开 `Stream` 时，串流输入环对每个 slot 补发一次全零帧；route 离开 `Ui` 时，前端 `gamepad-listener` 清空按下状态和长按连发定时器。
- route 从 `Ui` 切到 `Stream` 时，沿用「等待手柄归位」语义，但判定移到 Rust 侧（检查最新状态单元是否全零），避免 JS 节流。
- gate 的重新计算时机：窗口 `Resized / Focused` 事件 + 每次 UI 推送时顺带查询 `is_minimized()`（开销很小，同时兜底事件丢失）。

### 5. 删除清单（Shell / 前端）

- `src-tauri/src/shell/gamepad_sampling.rs`：删除 `apply_shell_gamepad_sampling_recovery` 整条恢复链、`hint_gamepad_shell_background`（不再切 BackgroundWarm）、`hint_gamepad_shell_interactive`；窗口事件只触发 gate 重算。
- `src-tauri/src/shell/mod.rs`：删除启动 900ms rearm、`plan_*_refresh_actions`、订阅环内的 lifecycle 提升和 stalled 自愈。
- 前端：删除触屏 `hintShellInteractive` RPC 调用；arbiter 改为镜像；`gamepad-listener` 订阅 route 变化并在离开 `Ui` 时重置。

### 6. 诊断（trace）

只保留能回答「为什么没输入」的最小集合：

- `gamepadSourceDevice{Connected,Disconnected}`：设备 ID、来源（sdl / xinput）、VID/PID、分类。
- `gamepadSourceRate`：每秒每设备的样本数和非零样本数（1Hz）。
- `gamepadRouteChanged`：前后 route、foreground（visible / minimized）、stream_session_active、ui_capture。
- `gamepadStreamNeutralSent` / `gamepadUiListenerReset`。
- `gamepadRescanRequested`（仅用户手动触发）。

## Scope

- In scope:
  - `crates/ohmygamepad/backends/sdl3`：source / backend / service 收敛（单源、快照模型、删除 prime/reopen/自愈）
  - `crates/ohmygamepad/core`：runtime 删除 lifecycle / suspended / 健康度驱动；新增最新状态单元；默认绑定
  - `crates/ohmygamepad/protocol`：删除或 deprecate lifecycle / health / self-heal 字段；新增 `InputRoute`
  - `crates/ohmygamepad/bridge/host`：route 计算、最新状态单元导出
  - `crates/xbxengine/core/src/transport/rtc/stack/input_loop.rs`：无锁读取 + 中性帧
  - `src-tauri/src/shell/*`、`src-tauri/src/mods/gamepad/*`：删除恢复链，订阅环改为纯转发
  - `src/shared/gamepad/*`、`src/navigation/core/gamepad-listener.ts`、`src/pages/stream/stream-input-route-controller.ts`：arbiter 镜像、listener 重置
  - `dev-docs/gamepad-navigation.md` 补充输入链路与路由说明
- Out of scope:
  - 浏览器 Gamepad API、云端输入协议、rumble 合同、键盘 fallback 实现、导航引擎、macOS 专项问题

## Supersedes

本 RFC 落地后，以下仍处于 In Progress 的手柄 RFC 统一关闭，状态改为「被 2026-10-26 RFC 取代」：

- `2026-05-11-gamepad-runtime-warm-sampling-and-stalled-self-heal`：BackgroundWarm 与 stalled 自愈整体移除。
- `2026-05-12-gamepad-lifecycle-and-input-routing-simplification`：lifecycle 被移除，路由收敛到本 RFC 的 `InputRoute`。
- `2026-05-14-gamepad-tauri-active-gate-and-always-on-sampling`：gate 判定沿用 `10-25` 的「只认最小化」，FSE 专线不恢复。
- `2026-05-14-gamepad-business-input-ownership-ui-stream-routing`：前端 owner 派生改为镜像后端 route。
- `2026-04-22-sdl3-gamepad-single-track-backend-unification`：若 Phase 0 选方案 A 则按单轨完成收尾；选方案 B 则在本 RFC 中记录对单轨决策的修订。
- 任务台账 `2026-10-25-fse-gamepad-input-gate-picks-minimized` 并入本任务。

## Plan

每个阶段独立可合入、可回滚，并且都必须带实机 trace 验证。

1. **Phase 0 · 基线与 spike**
   - 用当前版本在掌机 FSE 下录三份基线 trace：冷启动、触屏 / 覆盖层、前后台切换。
   - 加临时环境变量 `OHMYGAMEPAD_WIN_SOURCE=sdl-xinput|xinput-direct` 切换方案 A / B，在同一场景下对比，选定 Windows 采样来源。
   - 退出条件：选定方案写入本 RFC 的 Decision 记录。
2. **Phase 1 · 单源 source**
   - 落地选定方案；source 改为纯快照模型；删除 player_index XInput fallback、（方案 A）direct poller、prime/reopen 命令、事件增量路径；空槽位 1s 探测；新增手动重新扫描。
3. **Phase 2 · 常驻 runtime + 删除自愈**
   - 删除 lifecycle / suspended / 健康度驱动 / service 自愈；删除 shell 恢复链、启动 rearm、触屏 hint；默认绑定改为 `LastActiveFailover`；统一时钟。
4. **Phase 3 · 无锁发布**
   - 新增最新状态单元；串流输入环和 UI 推送改为无锁读取；Tauri 订阅环改为纯转发。
5. **Phase 4 · 单一路由 + 中性帧**
   - Rust `InputRoute`；离开 Stream 补发中性帧；前端 arbiter 镜像 route；listener 在离开 Ui 时重置。
6. **Phase 5 · 收尾**
   - 清理废弃协议字段与前端死代码；更新 `dev-docs/gamepad-navigation.md`；关闭被取代的 RFC；输出 Report。

## Validation

### 自动化

- [ ] `cargo test -p ohmygamepad-sdl3 -p ohmygamepad-core -p ohmygamepad-host`：单源去重、快照模型、route 派生、离开 Stream 时补发中性帧、无 lifecycle 状态下持续发布
- [ ] `cargo test -p xbxengine`：input loop 不再调用 `host.snapshot()`、route 切换时发出中性帧
- [ ] 前端单测：arbiter 镜像 route；listener 离开 Ui 时清空连发定时器
- [ ] `cargo fmt`、`cargo check -p xbxrc`、`pnpm lint:fix`

### 掌机 FSE 实机（每项附 trace）

- [ ] 冷启动 10 次：进入首页后首个按键 ≤1s 有 UI 响应，10/10
- [ ] 触屏 / 软键盘 / Xbox 覆盖层（Game Bar）唤出后返回，各 10 次：输入不中断，trace 中无设备 Disconnected、无重新枚举设备变化
- [ ] 最小化恢复 / 任务视图切换 20 次：返回后 ≤200ms 有输入
- [ ] 串流中按住按键切后台再返回：trace 有 `gamepadStreamNeutralSent`，云端无卡键
- [ ] 外接手柄热插拔 5 次：插入 ≤1s 可用；拔出无卡键；Pad0 正确跟随最近使用的设备
- [ ] 30 分钟串流：全程无设备句柄重开；input loop 发送间隔 p99 ≤12ms；`gamepadSourceRate` 持续约 250Hz
- [ ] macOS 回归：冷启动、前后台切换、串流输入正常

## Risks

- **SDL hints 在具体掌机驱动上的行为未知**（例如关闭 RawInput 后扳机分离、Guide 键、内置手柄识别）→ Phase 0 spike 先验证；方案 B 兜底。
- **删除自愈可能暴露此前被掩盖的真实冻结**→ 焦点无关的单源采样是根治手段；保留手动重新扫描入口和 `gamepadSourceRate` 诊断。只有拿到 trace 证据后才考虑恢复机制，且必须由事实触发。
- **XInput 局限**：最多 4 个 user、无电量细节、Guide 键需私有 API → 若方案 A 下 SDL 已覆盖则无影响；方案 B 需评估 Guide 键需求。trigger rumble 继续走现有 `WinXboxHaptics` provider。
- **「未最小化但失焦」仍向串流转发输入**，与当前行为一致；掌机 FSE 下基本不会出现多窗口并行，暂不改动（见 Open Questions）。
- **跨层删除量大**→ 按 Phase 拆分合入，每个 Phase 独立回滚；Phase 2 之前不删除协议字段，保证前后端可以分批升级。

## Open Questions

1. 实机型号（ROG Ally / Ally X / Legion Go / MSI Claw / 其他）？Phase 0 spike 需要在目标机型上进行。
2. Phase 0 若方案 A、B 效果相当，是否同意默认选 A（SDL 单轨、代码更少）？
3. 「未最小化但失焦」时是否继续向串流转发？默认维持现状（继续转发）。

## Progress

- [x] Phase 0: 基线 trace 与 Windows 采样来源 spike（按决策跳过实机基线，采样来源选定方案 A：SDL 单轨）
- [x] Phase 1: 单源 source
- [x] Phase 2: 常驻 runtime + 删除自愈
- [x] Phase 3: 无锁发布（ArcSwap latest_pad_states + AtomicU8 input_route_cache）
- [x] Phase 4: 单一路由 InputRoute + 中性帧（三态 Blocked/Ui/Stream + transition 补发中性帧 + setUiCapture RPC 四层链路）
- [x] Phase 5: 协议字段清理（删除 OhMyGamepadInputGateModeDto / input_gate / input_gate_reason / BackendCoarseGate / snapshotGateToBackendGate；arbiter 只靠 backendRoute 三态派生 owner）

## Execution Notes

- Date: 2026-10-26 | Status: planned
- Update: 起草 RFC。基于代码审阅整理 R1～R7 根因：三路采样源互相覆盖、前后台切换必然重开设备、启发式自愈、lifecycle 与可用性耦合、热路径阻塞、默认 Merged、路由分散且不发中性帧。
- Decision: 保持 Rust 原生采样主线，不引入浏览器 Gamepad API；目标结构为「单源 + 常驻采样 + 无自愈 + 单一 `InputRoute` + 无锁发布」。
- Risk/Blocker: Windows 采样来源（方案 A / B）需 Phase 0 掌机实机 spike 后确定；待用户确认 Open Questions 后进入执行。
- Date: 2026-10-26 | Status: implementing
- Decision: 跳过 Phase 0 实机基线（当前 macOS 环境无法采集掌机 trace），Windows 采样来源直接选定**方案 A：SDL 单轨**（SDL hints 关闭 RawInput / WGI / HIDAPI-Xbox，保留 `SDL_JOYSTICK_XINPUT=1`）。
- Decision: 本轮交付 Phase 1 + Phase 2；Phase 3～5 后续单独实施。协议诊断字段 `sampling_lifecycle` / `sampling_health` / `sampling_self_heal_count` 按 Phase 2 风险约定暂不删除，固定输出 `active` / `healthy` / `0`。
- Update Phase 1: `sdl3` backend 收敛为每设备单源全量快照，删除 XInput fallback / direct poller / 事件增量路径 / prime-reopen；轮询固定 4ms；空槽 1s 枚举只做新增与移除；新增手动 `rescanDevices`（1000ms ack）。默认绑定改为 `LastActiveFailover`。
- Update Phase 2: 引入进程级单调时钟（`monotonic_origin` / `monotonic_now_ms`），删除 core 侧 observed_at 归一化；runtime 常驻采样，删除 suspended / lifecycle / 健康度评估与自愈命令；Tauri shell 删除前台刷新链、启动 rearm、触屏 hint 与订阅环自愈；前端删除 `hintShellInteractive` / `activateSampling` / `resumeShellSampling`，设置页新增「重新扫描手柄」。
- Validation: `cargo fmt --all`、`cargo check -p xbxrc`、`cargo test -p ohmygamepad-core -p ohmygamepad-sdl3 -p ohmygamepad-host`（37 项通过）、`pnpm lint:fix`、`pnpm test`（169 项通过）均通过。
- Risk/Blocker: 掌机 FSE 实机验证（冷启动首键、触屏/覆盖层、前后台切换、热插拔、30 分钟串流）仍待用户在目标机型补做。

### Phase 3 · 无锁发布

- **ArcSwap 最新状态单元**：在 `ohmygamepad-core::InputRuntimeHandle` 引入 `latest_pad_states: Arc<ArcSwap<Vec<LogicalPadSnapshotDto>>>` + `latest_pad_seq: Arc<AtomicU64>`。runtime 线程每次真正 `sample_once` 后立即 replace ArcSwap（独立于 UI 60Hz 节流），高频读方永远读到最新。新增 `replace_latest_state` helper 和 33→34 项回归测试。
- **AtomicU8 input_route_cache**：Phase 4 派生但 Phase 3 已铺垫 —— `peek_input_gate_open` 从直接调 Mutex 改为 load AtomicBool。

### Phase 4 · 单一路由 + 中性帧

- **OhMyGamepadInputRouteDto 三态**：Blocked / Ui / Stream，取代旧 gate 的真相源。`derive_input_route(hints, stream_forwarding, ui_capture)` 统一派生函数。`peek_input_route()` 无锁 AtomicU8 load。
- **中性帧补发**：`xbxengine::input_loop::pump_input_stream_once` 内部检测 `Stream → 非 Stream` transition，构造同数量全零帧发远端归零，避免断帧后远端 pad 卡在按下状态。
- **setUiCapture RPC 四层链路**：`contract.ts → rpc.rs GamepadCommand → GamepadProvider trait → host::set_ui_capture → service.ui_capture AtomicBool`。前端 `streamInputRouteController.syncOverlayCapturingFlag()` 每次变化上报。
- **关键 bug 修复**：`setStreamActive(false)` 必须走 `syncOverlayCapturingFlag()` 发出 setUiCapture(false)，否则后端 ui_capture 残留 true 导致下次进入串流 route 还是 Ui 而非 Stream。

### Phase 5 · 协议字段清理

- Rust protocol 删：`OhMyGamepadInputGateModeDto` 枚举 + `input_gate` / `input_gate_reason` 字段 + `derive_input_gate_from_hints` / `peek_derived_input_gate` / `peek_input_gate_open`。
- sdl3 service / host bridge / Tauri gamepad 模块全量删 gate 残留，enrich_runtime_snapshot 只填 `input_route`。
- 前端 contract.ts 删：`GAMEPAD_INPUT_GATE_MODES` / `GamepadInputGateModeDto` / `inputGate` / `inputGateReason` 字段。
- arbiter 简化：`deriveBusinessInputOwner` 只靠 `backendRoute` 三态直接映射，彻底移除 `BackendCoarseGate` / `backendGate` / `snapshotGateToBackendGate`。
- 测试同步改：`business-input-arbiter.test.ts` 和 `stream-input-route-controller.test.ts` 全部适配新三态。
- **意外修复**：过程中发现 Phase 2 残留的 `last_input_packet_sent_at_ms` 记录用 `now_ms_f64()` 但在中性帧补发路径里没更新 —— 已补上。

### 实机待验证

- Windows 掌机 FSE 冷启动首键（SDL single-track + XInput fallback 已删）
- 串流会话 overlay 抢占/释放完整路径（中性帧归零）
- 前后台切换 route 同步（非前台 → Blocked）
- 热插拔（空槽 1s 探测）
- 30 分钟串流 steady-state（无卡死、无热重复）
