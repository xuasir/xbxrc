# SDL3 Gamepad 单轨后端统一 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: in-progress
- Owner: codex
- Last Updated: 2026-04-22

## Background

- 当前桌面手柄接入层以 `ohmygamepad/backends/gilrs` 为主，并叠加 `win-xbox-haptics`、`macos-gccontroller-haptics`、`hid-dualsense` 等平台/设备后端。
- 实际运行表现已经出现明显平台分裂：macOS 上 `gilrs` 输入可用、haptics 可用；Windows 上 `gilrs` 输入对 Xbox 官方手柄不稳定，当前通过 `XInput` 补丁兜住输入，WinRT haptics 仍不稳定，`XInput` 基础震动可用。
- 现状本质上已经进入多路线叠加：`gilrs`、`XInput fallback`、`WinRT haptics`、平台特化 haptics 并存，代码语义与行为边界都在分叉。
- 本项目需要一个桌面侧手柄接入主线。当前目标优先级是：稳定输入、稳定基础震动、统一映射、减少平台分叉、避免双轨长期共存。

## Goal

- 以 `SDL3::gamepad` 收敛桌面物理手柄接入层，建立单一主线后端。
- 明确迁移后不再保留 `gilrs` 与 SDL3 并行、`XInput fallback` 与 SDL3 并行、`WinRT haptics` 与 SDL3 并行的长期双轨代码。
- 保持上层 `ohmygamepad/core`、`bridge/host`、`src-tauri/mods/gamepad`、`xbxengine` 的逻辑抽象稳定，只替换物理设备接入与基础 haptics 主干。
- 将 Windows/macOS 的 Xbox 官方手柄体验收敛到统一行为基线，并为更多标准手柄提供更接近浏览器 Gamepad API 的兼容面。

## Scope

- In scope:
  - 新增基于 `SDL3::gamepad` 的桌面物理手柄后端 crate。
  - 用 SDL3 替换 `crates/ohmygamepad/backends/gilrs` 的主职责：设备发现、热插拔、输入采样、标准映射、基础 rumble。
  - 收口 `win-xbox-haptics` 与 `macos-gccontroller-haptics` 的主路径职责，避免继续作为并行默认后端存在。
  - 明确 `hid-dualsense` 等设备特化能力的去留策略，并优先按“单轨”原则做减法。
  - 更新运行时选择器、宿主桥接、能力推断、任务台账与迁移文档。
- Out of scope:
  - 改写 `ohmygamepad/core` 的 `logical pad`、采样策略、路由、协议结构。
  - 改写 `xbxengine` 的输入协议、rumble 请求来源或流媒体控制语义。
  - 在第一阶段追求所有厂商专属高级能力全量对齐。
  - 保留桌面物理输入的长期多后端共存模式。

## Decision

- 桌面物理手柄接入层升级到 SDL3 单轨主线。
- 迁移完成后，`gilrs`、`XInput fallback`、`WinRT haptics fallback` 都退出主线职责，不再作为长期并行路径保留。
- 平台/设备专属增强能力仅在 SDL3 无法覆盖且业务价值明确时才允许重新引入，并且需要先补 `docs/` 设计文档说明边界、收益和退出条件。

## Plan

1. 确认 SDL3 在当前项目中的运行与分发约束，落定原生依赖、线程模型、Tauri 集成方案。
2. 设计 `ohmygamepad-sdl3` 后端边界，定义事件采样、设备标识、能力映射、rumble 能力模型。
3. 替换当前运行时选择器与宿主桥接入口，让 SDL3 成为唯一桌面物理手柄后端。
4. 清理 `gilrs`、Windows XInput 补丁路径、WinRT 专用 haptics 主路径及相关能力推断逻辑。
5. 完成跨平台验证、文档更新和迁移收尾。

## Validation

- [ ] Windows 上 Xbox 官方手柄在 SDL3 主线下实现稳定输入采样（待实机验收）。
- [ ] Windows 上基础 rumble 在 SDL3 主线下稳定可用（待实机验收）。
- [ ] macOS 上 Xbox 官方手柄输入与基础 haptics 不回退（待实机验收）。
- [ ] 运行时快照、设备能力、pad snapshot、前端导航输入保持合同稳定。
- [ ] 代码库中不存在桌面物理手柄长期双轨默认后端。

## Risks

- SDL3 引入原生运行时依赖，构建、签名、分发与 CI 复杂度上升。
- Tauri 进程模型与 SDL 事件泵/线程模型的集成存在不确定性。
- 设备专属高级能力可能弱于现有平台特化实现，短期内需要接受能力收缩。
- 迁移过程如果保留过渡代码过久，容易再次滑回双轨架构。

## Progress

- [x] Step 1: 完成 SDL3 集成约束盘点与依赖分发方案。
- [x] Step 2: 完成 `ohmygamepad-sdl3` 后端设计。
- [x] Step 3: 完成运行时切换与单轨入口接线。
- [x] Step 4: 完成旧后端与补丁路径移除。
- [ ] Step 5: 完成验证、文档与任务收尾。

## Execution Notes

- Date: 2026-04-22 | Status: planned
- Update: 创建 SDL3 单轨手柄后端统一 RFC，明确本次升级目标是收敛桌面物理手柄接入主线，并禁止长期双轨共存。
- Decision: `SDL3::gamepad` 作为唯一桌面物理手柄后端主线；迁移完成后移除 `gilrs` 主线职责与 Windows 平台上的 `XInput/WinRT` 并行补丁路径。
- Risk/Blocker: SDL3 的 Tauri 集成、运行时分发和高级能力覆盖仍需实测确认。
- Date: 2026-04-22 | Status: completed
- Update: 新增 `ohmygamepad-sdl3` 后端 crate，并将 `selector` 与 `host` 主线切换为 SDL3 语义；对外运行时快照统一上报 `backend=sdl3` 与 `haptics provider=sdl3-gamepad`。
- Update: 将 `source/event/backend` 补充中立命名别名（`InputSource` / `InputEvent` / `InputBackendAggregator`），为后续彻底替换物理采集实现保留稳定边界。
- Update: 运行时默认不再走 `macos-gccontroller` / `windows-xbox` 平台专用 haptics provider，平台补丁路径退出默认主线职责。
- Validation: `cargo check -p ohmygamepad-host -p ohmygamepad-sdl3 -p ohmygamepad-core -p ohmygamepad-protocol` 与 `cargo test -p ohmygamepad-core runtime::selector::tests::selector_chooses_expected_default_haptics_provider && cargo test -p ohmygamepad-host` 已通过。
- Date: 2026-04-22 | Status: completed
- Update: 完成“彻底切换”收尾：移除 Windows `XInput` fallback 输入采样逻辑与 `windows-sys` 依赖；`DesktopInputProviderKind` / `DesktopHapticsProviderKind` 收敛为 SDL3 单值；协议与前端契约移除 `gilrs` 与平台专用 haptics provider 枚举项。
- Validation: `cargo check -p xbxrc` 通过，确认 `src-tauri` 到 `ohmygamepad` 全链路在 SDL3 单轨语义下可编译。
- Date: 2026-04-22 | Status: completed
- Update: `ohmygamepad-sdl3` 不再是单行 re-export，已拆分 `event/source/backend/runtime/service` 五层边界，确保后续可独立替换底层采集实现。
- Update: `src-tauri` 侧 matcher 兼容旧配置值 `gilrs`，读取时统一映射到 `sdl3`，避免旧持久化配置升级失败。
- Validation: `cargo check -p ohmygamepad-protocol -p ohmygamepad-core -p ohmygamepad-gilrs -p ohmygamepad-sdl3 -p ohmygamepad-host`、`cargo test -p ohmygamepad-core runtime::selector::tests::selector_chooses_expected_default_haptics_provider`、`cargo test -p ohmygamepad-gilrs backend::tests::poll_emits_added_device_and_sample`、`cargo test -p ohmygamepad-gilrs service_rumble::tests::prepare_rumble_dispatch_allows_sdl3_fallback_with_backend`、`cargo check -p xbxrc` 均通过。
- Risk/Blocker: 当前环境无法直接完成 Windows/macOS 手柄实机插拔与 rumble 验收；需在目标设备上按 Validation checklist 补录结果。
- Date: 2026-04-22 | Status: in-progress
- Update: 根据 review findings 将 RFC 状态回退到执行中；当前工作重点是落地真实 SDL3 source/backend/runtime/service，并恢复 macOS/Windows 现有平台 haptics 主路径。
- Risk/Blocker: 文档、宿主接线和持久化语义需要与真实 SDL3 实现重新对齐；Report 在全部实现与验收完成后补齐。
- Date: 2026-04-22 | Status: in-progress
- Update: `ohmygamepad-sdl3` 已完成真实 `event/source/backend/runtime/service` 实现，`Cargo.toml` 直接依赖 `sdl3`，不再存在 `gilrs` 别名包装。
- Update: host/runtime snapshot 已切到 SDL3 事实语义：设备能力统一为 `sdl3_capabilities`，运行时快照统一为 `devices / slot_bindings / input_policy / slots / haptics`，默认不再注入平台专用 haptics provider。
- Update: 对外契约继续收敛到 `slot` 与 `input_policy`，renderer 事件改为 `slot-snapshot`，rumble target 也改为 `slot` 语义；`ohmygamepad-core` 相关 runtime tests 已重新对齐并通过。
- Validation: `cargo check -p ohmygamepad-sdl3`、`cargo check -p ohmygamepad-host`、`cargo check -p xbxengine`、`cargo check -p xbxrc`、`cargo test -p ohmygamepad-core runtime::engine::tests -- --nocapture`、`cargo test -p ohmygamepad-core runtime::runner::tests -- --nocapture` 已通过。
- Risk/Blocker: Windows/macOS 的 Xbox 官方手柄输入与 rumble 仍缺实机验收，Validation checklist 与最终 Report 保持未完成状态，待目标设备补录结果后再切到 completed。
