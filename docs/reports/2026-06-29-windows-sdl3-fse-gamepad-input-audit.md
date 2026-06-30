# Windows SDL3 FSE 手柄输入排查报告

## 结论

当前实现的高风险点集中在 SDL3 设备发现层与 XInput 兜底层之间。本轮已完成修复：SDL3 opened-device fallback 保留，Windows source 级 XInput direct poller 已加入同一 source 线程。

已落地的 XInput fallback 位于 SDL3 已打开 `Gamepad` 的快照读取路径，入口是 `capture_gamepad_baseline_state(gamepad)`。它只在 `gamepad.player_index()` 存在且 `XInputGetState(userIndex)` 成功时返回 XInput 状态。覆盖范围止于“SDL3 已经枚举并打开设备，但 SDL 状态缓存或事件流在 FSE 下钝住”的形态。

FSE 冷启动若表现为 `SDL_GetGamepads` 返回空列表，direct poller 会扫描 XInput user 0..3。`XInputGetState(userIndex)` 成功时，source 会合成稳定设备 `xinput:user:N`，发送 `Connected + Snapshot`，并在 trace 中标记 `sampleSource=xinputDirect`。

旧 trace 缺少 `sampleSource=sdl|xinputFallback|xinputDirect`、`playerIndex`、`xinputResult`、`xinputUserIndex`。现场看到 `sdl3PolledSnapshotChanged allZero=false` 时，只能证明快照发生变化；它无法单独证明 XInput 兜底生效。本轮补丁已让 `sdl3ConnectedBaselineSnapshot`、`sdl3PrimeSamplingSnapshot`、`sdl3PolledSnapshotChanged`、`sdl3PolledSnapshotStable` 写出这些字段，后续 Windows FSE fresh trace 可直接证明采样来源。

## 外部资料证据

本轮已补齐官方网页证据，并用本机 vendored SDL3 / `windows-sys` 源码交叉核对。

Microsoft 资料：

- [XInputGetState function](https://learn.microsoft.com/en-us/windows/win32/api/xinput/nf-xinput-xinputgetstate)：`dwUserIndex` 是用户控制器索引，取值 0..3；成功返回 `ERROR_SUCCESS`；手柄未连接返回 `ERROR_DEVICE_NOT_CONNECTED`；失败返回 Winerror.h 错误码。
- [Game launchers and handhelds - Microsoft GDK](https://learn.microsoft.com/en-us/xbox/gdk/docs/gdk-dev/pc-dev/handheld/handheld-launchers)：Windows 将 handheld 的专用全屏模式称为 full screen experience (FSE)；应用可用 `IsGamingFullScreenExperienceActive` 查询 FSE；可用 `RegisterGamingFullScreenExperienceChangeNotification` / `UnregisterGamingFullScreenExperienceChangeNotification` 监听变化；文档还说明 FSE active 时 taskbar hidden，foreground/background 管理仍遵循 Windows foreground window 规则。

SDL 资料：

- [SDL_GetGamepads](https://wiki.libsdl.org/SDL3/SDL_GetGamepads)：返回当前连接 gamepad 的 joystick instance ID 列表，失败时返回 `NULL`。
- [SDL_UpdateGamepads](https://wiki.libsdl.org/SDL3/SDL_UpdateGamepads)：手动 pump gamepad updates；events enabled 且使用 event loop 时自动调用。
- [SDL_HINT_JOYSTICK_ALLOW_BACKGROUND_EVENTS](https://wiki.libsdl.org/SDL3/SDL_HINT_JOYSTICK_ALLOW_BACKGROUND_EVENTS)：默认后台禁用 joystick/gamecontroller input events；设置为 `"1"` 后后台启用。
- [SDL_SetMainReady](https://wiki.libsdl.org/SDL3/SDL_SetMainReady)：未使用 `SDL_main()` 作为入口时，用于避免 `SDL_Init()` 因 main glue 缺失而失败。

本地源码交叉核对：

- `sdl3-src-3.4.4/SDL/include/SDL3/SDL_gamepad.h` 与 SDL wiki 对 `SDL_GetGamepads` / `SDL_UpdateGamepads` 的语义一致。
- `sdl3-src-3.4.4/SDL/include/SDL3/SDL_hints.h` 与 SDL wiki 对 `SDL_HINT_JOYSTICK_ALLOW_BACKGROUND_EVENTS` 的默认值和 `"1"` 语义一致。
- `sdl3-0.18.0/src/sdl3/gamepad.rs` 对 `SDL_GetGamepads` 与 `SDL_UpdateGamepads` 做封装。
- `windows-sys-*/src/Windows/Win32/UI/Input/XboxController/mod.rs` 暴露 `XInputGetState(dwUserIndex, pstate)`，并定义 `XUSER_MAX_COUNT = 4`。

## 当前实现

FSE gate 主线：

- `derive_input_gate_from_hints()` 固定为 `sampling_lifecycle == Active && shell_app_active`，open reason 为 `sampling-active-and-shell-app-active`。
- Windows FSE 或 Tauri fullscreen 下，`shell_app_active` 由 Win32 foreground HWND 归属派生。
- FSE monitor 动态加载 `GamingExperience.dll`，解析 `IsGamingFullScreenExperienceActive`、`RegisterGamingFullScreenExperienceChangeNotification`、`UnregisterGamingFullScreenExperienceChangeNotification`。
- FSE change callback 触发 `sync_gamepad_input_gate()` 与 `refresh_gamepad_on_window_foreground()`。
- Win32 foreground gate 支持主 HWND、WebView 子 HWND、同进程 foreground HWND，并带 1500ms 触屏宽限。

SDL3 source 主线：

- source 线程启动前调用 `SDL_SetMainReady()`。
- 设置 `SDL_JOYSTICK_ALLOW_BACKGROUND_EVENTS=1`。
- 同时启用 joystick/gamepad events。
- 初始化后先执行 `joystick.update()` 与 `gamepad.update()`，再调用 `gamepad_subsystem.gamepads()`。
- 启动 2 秒内 100ms 重枚举，之后 1s 重枚举。
- 已打开设备每 8ms 轮询 snapshot。
- Connected 后立即发送 baseline snapshot，全零 baseline 也能建立 logical device 事实。

XInput fallback 当前形态：

- `try_capture_xinput_state(player_index)` 调用 `XInputGetState`。
- `capture_gamepad_baseline_state(gamepad)` 在 Windows 上优先尝试该 fallback。
- 入口依赖 SDL3 已经返回并打开 `Gamepad`，同时 `player_index` 在 0..3。
- 本轮补丁已给 baseline / prime / poll snapshot trace 增加 `sampleSource`、`playerIndex`、`xinputUserIndex`、`xinputResult`、`fallbackReason`。

XInput direct 当前形态：

- `XInputDirectPoller` 运行在 SDL3 source 线程内，跟随 8ms snapshot tick 扫描 XInput user 0..3。
- `XInputGetState(userIndex)` 成功时合成 `xinput:user:N` 设备，发送 `Connected + Snapshot`，后续轮询发送普通 `Snapshot`。
- direct snapshot trace 使用 `sampleSource=xinputDirect`，同时写入 `playerIndex`、`xinputUserIndex`、`xinputResult`。
- SDL3 opened-device 若带有同一个 `player_index=N`，direct poller 会断开 `xinput:user:N`，让 SDL3 路径成为该 user 的权威来源。

过往提交校正：

- `311e48a9 fix(gamepad): add XInput fallback for FSE mode sampling issues` 修改了 `crates/ohmygamepad/backends/sdl3/src/source.rs`，新增的实现是 SDL opened-device 快照路径上的 XInput fallback。
- `22a23c6f feat(backend): 增加Windows FSE模式下的XInput直接轮询支持` 实际变更是 `Cargo.lock` 与 `docs/fse-xinput-fix-verification.md`，没有在当前 source 中落下独立 XInput direct poller。
- `docs/fse-xinput-fix-verification.md` 将 `sdl3PolledSnapshotChanged allZero=false` 作为 XInput 直接轮询验证信号；结合当前 trace schema，这个验证信号强度不足。

## 已修复问题

1. XInput fallback 绑定在 SDL opened-device 路径

   `emit_polled_snapshots()` 遍历 `opened_gamepads`，再调用 `capture_gamepad_baseline_state(&opened.gamepad)`。当 `SDL_GetGamepads` / `gamepad_subsystem.gamepads()` 的发现结果为 0 时，`opened_gamepads` 为空，fallback 路径整轮跳过。现在 direct poller 会在这种形态下独立扫描 XInput user 0..3。

2. `player_index` 缺失会落回 SDL 读取路径

   `try_capture_xinput_state(player_index)` 依赖 SDL 给出的 `player_index`。`None`、越界、`XInputGetState` 失败都会回到 SDL `gamepad.button()` / `gamepad.axis()`。本轮补丁已把三类 fallback miss 分别记录为 `player-index-missing`、`player-index-out-of-range`、`xinput-get-state-failed`。direct poller 仍会按 XInput user 独立兜底，直到 SDL3 opened-device 报告同 user `player_index`。

3. 旧 trace 缺少采样来源字段

   已有本地 trace 仍然只有 allZero、axis/button 汇总和 device 信息。当前代码已补字段，仍需 Windows FSE fresh trace 复验。

4. 本地 trace 只能证明当前机器没有物理 SDL 设备进入链路

   已分析 trace 的共同形态是 `discoveredCount=0/openedDeviceCount=0/sdlPhysicalConnectedDevices=0`，runtime snapshot 只有 `virtual:keyboard`，`lastBackendSampleActivityAtMs=0`。这些样本证明本机没有物理 SDL raw sample 进入 core，无法替代 Windows FSE 实机验收。

## Trace 观察

分析过的本地 trace：

- `runtime-logs/runtime-trace-1782720417464-1.jsonl`
- `runtime-logs/runtime-trace-1782716659031-1.jsonl`
- `runtime-logs/runtime-trace-1782453170367-1.jsonl`

共同信号：

- `sdl3InputRuntimeHintsApplied` 已记录 main ready 与 background events。
- `sdl3InputRuntimeInitialized` 已记录 joystick/gamepad events enabled、8ms poll、100ms startup reenumerate。
- `sdl3GamepadEnumerationObserved` 持续 `discoveredCount=0/openedDeviceCount=0`。
- runtime snapshot 只有 `connectedDeviceIds=["virtual:keyboard"]`。
- `sdlPhysicalConnectedDevices=0`。
- `lastBackendSampleActivityAtMs=0`。

## 已落地修复

1. Windows source 级 XInput direct poller

   已在 SDL3 source 内新增 Windows-only poller，周期性扫描 XInput user 0..3。`XInputGetState` 成功时合成 `Sdl3InputEventKind::Connected + Snapshot`，device id 使用稳定形态 `xinput:user:N`。FSE 冷启动即使 SDL 枚举为空，XInput 设备也能进入 runtime。

2. 保留 SDL3 主线优先级和去重

   SDL3 设备出现并报告 `player_index=N` 后，direct poller 会断开 `xinput:user:N`，避免同一 XInput user 双路进入 logical slot。SDL opened-device 缺少 `player_index` 时，direct path 仍保留输入兜底能力。

3. 补 trace 字段

   已为 snapshot 类事件增加：

   - `sampleSource`: `sdl` / `xinputFallback` / `xinputDirect`
   - `playerIndex`
   - `xinputResult`
   - `xinputUserIndex`
   - `fallbackReason`

4. 修正 FSE 验收口径

   FSE 验收应同时检查：

   - FSE monitor: `fseMonitorInitialized` / `fseChangeObserved`
   - gate: `inputGate=open` 且 reason 为 `sampling-active-and-shell-app-active`
   - source: SDL 或 XInput direct 至少一路有物理设备
   - sample: `lastBackendSampleActivityAtMs > 0`
   - progress: `lastSampleProgressAtMs` 随输入变化推进
   - route: stream active 时 owner 能从 `none/ui` 切到 `stream`

## 实机验收

下一步需要 Windows FSE 冷启动 fresh trace 验收。fresh trace 需要证明 `sampleSource=xinputFallback` 覆盖 SDL opened-device 钝态，或 `sampleSource=xinputDirect` 覆盖 SDL 枚举为空 / 缺 player index 形态，并确认 `lastBackendSampleActivityAtMs`、`lastSampleProgressAtMs` 随输入推进。

## 验证

- `cargo fmt -p ohmygamepad-sdl3`
- `cargo test -p ohmygamepad-sdl3 polled_snapshot_signature_tracks_sample_source --lib -- --nocapture`
- `cargo test -p ohmygamepad-sdl3 --lib -- --nocapture`
- `cargo check -p ohmygamepad-sdl3`
- `cargo check -p ohmygamepad-sdl3 --target x86_64-pc-windows-msvc` 已尝试，当前机器缺少可用 Visual Studio / CMake generator，`sdl3-sys` build script 停在 `couldn't determine visual studio generator`。
