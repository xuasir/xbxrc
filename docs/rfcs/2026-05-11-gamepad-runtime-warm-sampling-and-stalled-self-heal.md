# Gamepad Runtime Warm Sampling And Stalled Self-Heal RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 代码已合入主线；实机 / Xbox 大屏回归验证仍待补
- Current State: implemented-pending-device-validation
- Owner: codex
- Last Updated: 2026-05-11

## Background

- 当前 SDL3 手柄主线已经做到 `source` 持续收系统事件、`input_policy` 控制 UI/stream 路由、`resume_shell_sampling` 负责壳层基线重建。
- 当前 Xbox 大屏模式仍会出现“首次进入应用无输入，后台重新选中后输入恢复”的现象。现有恢复链依赖 `Focused(true)`、`visibilitychange`、`Resumed`、`mounted` 等边沿事件，再由前端补 `0ms/150ms/500ms` 及后续延长重试窗口去等待采样推进。
- 这类问题的本质是：窗口可见/聚焦事件与“壳层真实把手柄输入链交给应用”的时刻并不一致。当前系统把采样恢复建模为边沿补偿，所以在 Xbox 大屏、Game Bar、系统 overlay 等壳层场景里容易提前恢复、建出空基线，然后长时间等不到下一次有效补偿。
- Moonlight / Chiaki 的方向更接近“底层持续可采样，活跃态只影响消费”。本项目当前主问题已经从“SDL 收不到事件”转成“逻辑采样链和恢复策略仍然过度依赖窗口事件”。

## Goal

- 将当前 gamepad runtime 从“事件驱动恢复”升级为“持续可采样态”结构。
- 建立三态 runtime lifecycle：`Active`、`BackgroundWarm`、`Suspended`。
- 明确 `BackgroundWarm` 是“无副作用保温态”：
  - 持续 `poll`
  - 持续 `sample`
  - 维护设备事实与当前物理态
  - 不向 UI/stream 外泄操作边沿
- 将 stalled 检测与自愈下沉到 runtime / service，减少前端重试窗补丁。
- 将前端壳层恢复器收边为轻触发和诊断消费层，不再承担主恢复职责。

## Scope

- In scope:
  - `ohmygamepad-core` runtime lifecycle 从布尔 `suspended` 收敛到显式三态。
  - `BackgroundWarm` 的调度语义、边沿抑制语义、基线吸收语义。
  - `samplingHealth` / stalled detector / self-heal 设计与落地。
  - `ohmygamepad-sdl3` service 的轻恢复与强恢复边界重定义。
  - `src-tauri` 窗口事件从“恢复驱动”降级为 lifecycle hint。
  - `AppShellLayout` 前端恢复链收边，以及诊断字段展示。
- Out of scope:
  - 更换 SDL3 主线、回退到 `gilrs/XInput/WinRT` 双轨。
  - 改动 `xbxengine` 串流输入协议与 rumble 请求合同。
  - 重写现有 `input_policy` 的 UI/stream 路由模型。
  - 在本任务中处理更深层的掌机机型识别与设备映射兼容策略。

## Decision

- Gamepad runtime 采用显式 lifecycle：
  - `Active`
  - `BackgroundWarm`
  - `Suspended`
- `BackgroundWarm` 的唯一职责是保活采样链和当前物理态，不承担对外可操作输入职责。
- `input_policy` 继续只表达路由目标，不再隐式承担“采样要不要继续”的职责。
- stalled 检测、自愈、基线刷新主逻辑下沉到 Rust runtime / SDL3 service，前端不再通过长重试窗维持采样可用性。
- 前端保留：
  - 单次轻恢复触发
  - 壳层 `shared` 策略纠偏
  - 诊断展示

## Design

### 1. Runtime Lifecycle

- `Active`
  - `poll_backend`
  - `sample_once`
  - 允许发布可操作输入边沿
  - 正常 snapshot publish
- `BackgroundWarm`
  - `poll_backend`
  - `sample_once`
  - 更新 logical 当前态与设备事实
  - 抑制可操作输入边沿发布
  - 允许发布低频 runtime heartbeat 与设备变化事实
- `Suspended`
  - 保留当前强挂起语义
  - 用于显式 shutdown / 深挂起 / 资源级停机

### 2. 物理态与可操作边沿分层

- runtime 输出语义拆成两层：
  - `physical snapshot`
  - `actionable delta`
- `BackgroundWarm` 保留前者，抑制后者。
- `BackgroundWarm` 必须抑制：
  - 导航按键边沿
  - 摇杆方向边沿
  - 长按 repeat
  - 组合键动作
- `BackgroundWarm -> Active` 时执行基线吸收：
  - 当前采样态记为新基线
  - 清空 pressed / repeat / combo 等运行态
  - 只从切回后的新增变化重新产生 action

### 3. Source / Sampling / Routing 解耦

- SDL source thread 继续长期保活：
  - 设备 attach / remove / remap
  - event pump
  - 当前设备态建模
- sampling lifecycle 决定：
  - 采样频率
  - 是否允许 action publish
- `input_policy` 决定：
  - `shared`
  - `ui-only`
  - `stream-only`
  - 哪一侧消费 action

### 4. stalled 检测与自愈

- 新增 runtime 诊断字段：
  - `samplingLifecycle`
  - `samplingHealth`
  - `lastSampleProgressAtMs`
- `samplingHealth` 枚举：
  - `healthy`
  - `awaitingBaseline`
  - `stalled`
- stalled 判据建议：
  - lifecycle in `Active | BackgroundWarm`
  - 存在 connected device
  - 当前场景要求手柄可用，或壳层页处于可见态
  - 一段窗口内 `sampleSeq` / `sampledAtMs` 无推进
  - 允许结合 source 仍有设备事实这一条件，避免把“无输入空闲”误判成故障
- 自愈动作建议：
  1. `prime_sampling()`
  2. `refresh_snapshot()`
  3. 必要时重建当前 slot binding / descriptor snapshot
  4. 记录节流与连续失败诊断

### 5. 前后端职责重分配

- Rust runtime / service
  - lifecycle 状态机
  - stalled detector
  - self-heal
  - baseline absorb
- Tauri window layer
  - 仅提供 focus / visibility / minimize / resume hint
  - 驱动 `Active <-> BackgroundWarm`
  - 不承担主恢复职责
- AppShell
  - 单次轻恢复触发
  - `shared` 策略纠偏
  - 诊断展示
  - 去掉长期重试窗主逻辑

## Plan

1. 将 `ohmygamepad-core` runtime 从布尔 `suspended` 升级到三态 lifecycle，并完成调度语义拆分。
2. 在 runtime 内部分离 `physical snapshot` 与 `actionable delta`，实现 `BackgroundWarm` 边沿抑制与 `BackgroundWarm -> Active` 基线吸收。
3. 在 SDL3 service / runtime 引入 `samplingHealth`、stalled detector、自愈动作与结构化诊断。
4. 将 `src-tauri` 窗口事件改成 lifecycle hint，把前端长重试窗回收为单次轻恢复触发与诊断消费。

## Validation

- [ ] `Active` 下保持现有 UI/stream 输入语义不回退。
- [ ] `BackgroundWarm` 下 `sampleSeq` 继续推进，但不会触发 UI 导航、repeat、combo、stream action。
- [ ] `BackgroundWarm -> Active` 不会把已按住状态误发成新的操作边沿。
- [ ] Xbox 大屏模式首次进入应用时无需后台二次切回即可恢复手柄输入。
- [ ] Game Bar / overlay 往返不会再依赖前端长重试窗维持手柄可用性。
- [ ] stalled 检测不会把普通空闲状态误判成故障。
- [ ] runtime snapshot / diagnostics / 手柄卡片能准确区分 `stream-only` 路由残留与真实采样 stalled。

## Risks

- lifecycle 三态会触及 `runner/engine/service/host/tauri/frontend` 多层合同，回归面较大。
- `BackgroundWarm` 的边沿抑制如果定义不清晰，容易出现“采样活着但旧边沿泄漏”或“恢复后首拍丢动作”。
- stalled 判据如果过宽，会把自然空闲误判成故障，造成不必要的基线刷新与 trace 噪声。
- 前端去掉长重试窗后，如果后端 stalled 自愈实现不完整，Xbox 大屏场景会再次暴露首进失败。

## Progress

- [x] Step 1: runtime lifecycle 三态化与调度重写
- [x] Step 2: 物理态 / 可操作边沿分层与基线吸收（壳层门控 `slotSnapshot` + `inputBaselineAbsorbed` + UI listener 重置）
- [x] Step 3: stalled detector、自愈与诊断字段（含 `lastBackendSampleActivityAtMs`、节流 prime/refresh、`samplingSelfHealCount`）
- [x] Step 4: Tauri / AppShell 恢复链收边（焦点/恢复 lifecycle hint；AppShell 单次 `resumeShellSampling`）

## Execution Notes

- Date: 2026-05-11 | Status: implemented-pending-device-validation
- Update: 已落地 `ohmygamepad-core` 三态调度、`samplingHealth` 与 broadcaster 语义去重；`ohmygamepad-sdl3` `try_stalled_sampling_self_heal`；Tauri shell 仅在 `Active` 下发 `slotSnapshot` 并在 Warm→Active 发基线事件；`lib.rs` 窗口事件改为 lifecycle hint；AppShell 去掉长重试窗。
- Update: 创建 gamepad runtime 常驻采样与 stalled 自愈 RFC，收口当前 Xbox 大屏模式首进无输入问题的结构化改造方向。
- Decision: 不继续放大前端重试窗，改以 runtime 三态、`BackgroundWarm` 无副作用保温、后端 stalled 自愈为主线设计。
- Risk/Blocker: 现阶段缺少稳定实机环境，验证需要高度依赖结构化日志、runtime snapshot 与后续目标设备回归。
