# Gamepad Gate、FSE 专线与前端的 Routing 收口

## 摘要

落实 [`rfcs/2026-05-14-gamepad-tauri-active-gate-and-always-on-sampling.md`](../rfcs/2026-05-14-gamepad-tauri-active-gate-and-always-on-sampling.md) 的 gate + FSE + 前端 routing 主线。

## Backend gate

- `ShellWindowGateHints` 主字段改为 `app_is_foreground_candidate`；`focused/visible/minimized/fse_active/foreground_hwnd_matches_main` 仅诊断。
- `derive_input_gate_from_hints`：`gateOpen = lifecycle==Active && app_is_foreground_candidate`。
- Tauri [`input_gate.rs`](../../src-tauri/src/mods/gamepad/input_gate.rs) 负责计算 hints 并写入 ohmygamepad；非 Windows 仅保留 `focused_from_event` 作为 foreground candidate 硬条件。

## Windows FSE

- 新模块 [`fse_windows.rs`](../../src-tauri/src/mods/gamepad/fse_windows.rs)：运行时加载 `GamingExperience.dll`，`IsGamingFullScreenExperienceActive` + change notification。
- FSE 下 `app_is_foreground_candidate = (GetForegroundWindow() == main_hwnd)`。
- 启动时在 `build_services` 调用 `init_fse_monitor`。
- FSE change callback 现在显式触发 `sync_gamepad_input_gate + refresh_gamepad_on_window_foreground`，把 `BackgroundWarm -> Active` 恢复链重新挂回 foreground 事实，而不是继续等 Tauri `Focused(true)`。

## 延迟补救

- 移除全屏冷启动 `500/2000/4000ms` 三轮 `hint_gamepad_shell_interactive` 主链。
- 新增配置 `gamepad_fse_gate_fallback_nudge`（默认 `false`）：仅 FSE + gate 长时间 closed + 已有 slot 时单次 fallback。
- `pageLoad` / `Focused` 改为 `sync_gamepad_input_gate` + `refresh_gamepad_on_window_foreground`（gate 开才 resume）。

## 前端 routing（先行已落地）

- 4 字段 `business-input-arbiter` + `stream-input-route-controller`（capture/release、heldCaptures、replaceUiCapture、adapter 切换）。
- `wait-pad-neutral.ts` 新增 `AbortSignal` 支持；overlay 重开 / page unmount 会显式中止旧 neutral wait，避免遗留轮询与事件订阅。

## 验证

- `cargo test -p ohmygamepad-sdl3 derive_input_gate`
- `cargo check -p xbxrc`
- `./node_modules/.bin/tsc -p tsconfig.json --noEmit`
- `esbuild + node` 自定义 abort harness：验证 `waitForPadNeutral({ signal })` 可被中止并完成 cleanup
- Windows Xbox 大屏 / FSE 实机：gate reason、`input_gate` 开合、冷启动输入（待人工勾选）
