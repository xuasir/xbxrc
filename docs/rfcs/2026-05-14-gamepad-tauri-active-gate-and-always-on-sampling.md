# Gamepad Input Routing Line Simplification RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: gate + FSE + frontend routing implemented; Windows FSE 实机待验收
- Current State: in progress
- Owner: codex
- Last Updated: 2026-05-19

## Background

- 当前 SDL3 手柄 source 已经具备常驻采样、后台事件、主动 poll snapshot 与 `reopen + prime + refresh` 自愈链。
- 当前输入线路的主要复杂度已经集中在前端 routing 层，而不是 UI 导航或 Stream consumer 本身。
- 参考 `chiaki-ng`，更合适的前端 routing 语义是：
  - UI 抢占时进入统一 `grab`
  - UI 关闭时先等 neutral release，再统一 `release`
  - routing 层不持有菜单细分态与 consumer ack
- 当前前端 routing 模型同时承载：
  - scene
  - backend gate
  - stream session 事实
  - stream UI surface 细分
  - runtime mode
  - Rust route ack
  - chrome 可见性
  - focus / visibility 补同步
- 这使“输入应该给谁”这件事被表达成一组跨页面、跨 consumer、跨焦点恢复链的组合状态。
- 近期 FSE 冷启动现象说明：Xbox/FSE/窗口焦点不稳定相关恢复逻辑没有把冷启动输入问题真正收掉。Chiaki 在 FSE 冷启动有输入，说明主问题不在 routing 表达层做更多焦点补偿。
- 当前还存在明确故障：
  - 打开 diagnostics 或通过触屏打开菜单后再关闭，可能落入“UI 不再消费、Stream 也未恢复消费”的空窗
  - 这类空窗说明 routing 当前把 overlay 关闭、neutral release、route ack、consumer 副作用拆成了多拍状态
- 本轮讨论范围固定为输入线路处理逻辑。UI 导航与 Stream consumer 保持现有实现边界稳定。

## Goal

- 精简 backend gate，让所有平台/模式下的 gate 都只依赖最少硬条件。
- 精简前端 input routing model，只保留决定 owner 所需的核心状态。
- 将 Xbox/FSE/窗口焦点不稳定相关恢复逻辑移出 routing 主线，并收成 Windows FSE 专线处理。
- 将延迟补救从主恢复链降级为临时兜底。
- 保持以下模块的职责和实现边界稳定：
  - UI 导航
  - Stream consumer
  - SDL3 物理采样主线
- 让 routing 层只回答一个问题：
  - 当前业务输入归属 `ui`、`stream` 还是 `none`

## Scope

- In scope:
  - `src-tauri/mods/gamepad/input_gate.rs` 与 gate 判定公式
  - `src/shared/gamepad/business-input-arbiter.ts` 的状态模型与派生逻辑
  - stream 页 overlay/menu 对 routing 的输入合同
  - routing 层与 `inputGate` 的边界
  - routing 层里的 focus / visibility / ack / resync 逻辑裁剪
  - Windows FSE 的检测、切换通知与 foreground 判定策略
  - 现有 delayed nudge / delayed interactive hint 的降级策略
- Out of scope:
  - 重做 UI 导航系统
  - 重做 `GamepadDriver`、`InputService`、Rust `setStreamPadForwarding` 的 consumer 主逻辑
  - 重做 SDL3 runtime、自愈、设备诊断与物理采样链
  - 解决 FSE 冷启动无输入的采样层根因

## Problem Statement

当前输入线路处理逻辑有四个核心问题：

0. backend gate 仍然过重
- 虽然逻辑上只剩一层 gate，但当前 gate 仍混入多项窗口启发式与 lifecycle 条件。
- 一层 gate 如果看错信号，效果仍然和“多层门控”一样脆弱。

1. 前端 routing 状态过度建模
- 当前 owner 派生依赖 `streamSessionId`、`streamSessionPresent`、`streamConsumer`、`streamUiSurface`、`rustEngineStreamPadRoutedToSession`、`chromeVisible` 等多项状态。
- 其中一部分状态表达 owner 决策，一部分状态表达 consumer 细节，一部分状态表达诊断信息。

2. routing 层混入焦点恢复职责
- `window focus`、`document visibility`、route resync、startup 补同步等逻辑进入 routing 主线。
- 这些逻辑没有改善 FSE 冷启动输入建立，反而让 routing 随窗口事件反复重算。

3. routing 决策等待 consumer ack
- Rust 路径通过 `rustEngineStreamPadRoutedToSession` 把“目标 owner”与“consumer 副作用完成”绑在同一模型里。
- 这让 routing 层同时承担决策和执行确认两类职责。

4. page-local overlay 细节泄露到全局 arbiter
- `menu / diagnosticsMenu / display / audio / text / failed / warning` 都进入全局 routing 状态。
- 这些细分状态本质上是 page-local UI 结构，不是全局输入线路主状态。

5. FSE 被当成“桌面窗口事件异常”在补，而不是独立系统模式
- Microsoft 文档已经给出 FSE 的显式检测与变化通知 API。
- 当前实现仍主要依赖 `pageLoad / Focused / visibilitychange / delayed nudge` 猜测何时可输入。
- 这会让 FSE 冷启动问题长期停留在“延迟补救链是否够长”的错误方向上。

## Decision

- backend gate 在所有平台/模式下统一简化：
  - 默认只回答“当前应用是否可接收 business input”
  - 不再混入前端 routing、consumer ack、页面结构或 page-local 恢复逻辑
- backend gate 的跨平台最小语义收成：
  - `sampling_lifecycle == Active`
  - `app_is_foreground_candidate == true`
- `visible`、`minimized`、`pageLoad`、`document visibility`、frontend resync 等信号退出 gate 硬条件，最多保留为诊断或辅助恢复事实。
- routing 主模型收成四个核心状态：
  - `appScene`
  - `backendGate`
  - `streamActive`
  - `overlayCapturing`
- routing 语义对齐 `chiaki-ng`：
  - `captureUiInput()` 相当于 `grabInput()`
  - `releaseUiInputAfterNeutral()` 相当于 `releaseInput() + BlockInput`
- owner 派生固定为：
  1. `backendGate !== open` -> `none`
  2. `appScene !== stream` -> `ui`
  3. `streamActive !== true` -> `ui`
  4. `overlayCapturing === true` -> `ui`
  5. 其余 -> `stream`
- `runtimeMode`、`streamConsumer`、`setStreamPadForwarding`、RTC suspend 等信息继续留在 consumer 层。
- `chromeVisible`、细分 `surface`、Rust route ack、focus/visibility resync 全部退出 routing 主模型。
- Windows FSE 不再走普通桌面窗口启发式，而是按 Microsoft 文档走专线：
  - `IsGamingFullScreenExperienceActive`
  - `RegisterGamingFullScreenExperienceChangeNotification`
  - Win32 foreground window 判定
- 延迟补救保留为临时兜底，不再承担主恢复链；当 FSE 专线稳定后，默认关闭或删除。
- overlay 关闭时，`overlayCapturing` 保持为 `true` 直到 neutral release 完成；完成后原子切回 `false`

## Design

### 1. Backend Gate

推荐将 backend gate 的跨平台主公式收成：

```ts
gateOpen = lifecycleActive && appIsForegroundCandidate
```

其中：

- `lifecycleActive`
  - 继续来自后端采样 lifecycle
- `appIsForegroundCandidate`
  - 表达“当前应用已被系统认为是应接收输入的前台候选”

约束：

- 不把 `visible`、`minimized`、`document.visibilityState` 当成主 gate 条件
- 不把 routing owner、stream route、overlay 状态塞回 gate
- gate 只回答“业务输入能不能进入应用”

### 2. FSE Route

Windows FSE 单独走 Microsoft 推荐路径：

- 启动时调用 `IsGamingFullScreenExperienceActive`
- 注册 `RegisterGamingFullScreenExperienceChangeNotification`
- 若 FSE active，则 `appIsForegroundCandidate` 优先由 Win32 foreground window 事实派生
- 若非 FSE，则可保留现有桌面窗口判定作为兼容路径

推荐的 FSE 判定：

```ts
appIsForegroundCandidate = (GetForegroundWindow() == mainWindowHwnd)
```

这里的核心是：

- FSE 下只显示一个窗口，系统明确按 foreground app 路由可交互行为
- 因此应直接读取 Win32 foreground 事实
- 不再把 Tauri `Focused` 事件缓存当作唯一真相源

### 3. Routing State

推荐将前端 routing state 收成：

```ts
type BusinessInputOwner = 'ui' | 'stream' | 'none'

interface InputRoutingState {
  appScene: 'shell' | 'stream'
  backendGate: 'open' | 'closed'
  streamActive: boolean
  overlayCapturing: boolean
}
```

说明：

- `appScene`
  - 路由层事实
- `backendGate`
  - 继续来自后端 coarse `inputGate`
- `streamActive`
  - 表达“当前 stream 会话路径是否活跃”
- `overlayCapturing`
  - 表达“当前 UI 是否抢占业务输入”

### 4. Owner Derivation

派生逻辑固定为：

```ts
function deriveOwner(state: InputRoutingState): BusinessInputOwner {
  if (state.backendGate !== 'open') {
    return 'none'
  }
  if (state.appScene !== 'stream') {
    return 'ui'
  }
  if (!state.streamActive) {
    return 'ui'
  }
  if (state.overlayCapturing) {
    return 'ui'
  }
  return 'stream'
}
```

这个模型只保留“归属决策”，不携带 consumer 实现细节。

### 5. Overlay Contract

stream 页继续保留页面内的 menu / sheet / failed / warning 结构，routing 层只看一个收敛结果：

- overlay 打开 -> `captureUiInput()` -> `overlayCapturing = true`
- overlay 请求关闭 -> 保持 `overlayCapturing = true`
- neutral release 完成 -> `releaseUiInputAfterNeutral()` -> `overlayCapturing = false`

细分 surface 类型留在 page-local UI 层，用于：

- 焦点管理
- 文案展示
- 菜单层级
- 页面行为

### 6. Consumer Boundary

以下逻辑继续留在 consumer 层：

- 浏览器直连模式下的 RTC suspend
- Rust 模式下的 `setStreamPadForwarding`
- 程序化输入 bypass
- rumble stop
- neutral release 后恢复发流

routing 层只负责表达“目标归属”，consumer 层负责执行自己的副作用。

### 7. Route Timing And Neutral Release

当前 `close overlay -> wait neutral -> resume stream` 保护语义继续保留。

这条保护语义的推荐归属是：

- overlay/page controller 继续负责 neutral release 时机
- arbiter 只接收最终的 `overlayCapturing` 结果
- overlay 真正关闭的业务语义以 neutral release 完成为准，不以前端视觉层先隐藏为准

这样可以保留现有 consumer 行为，同时把“按键释放保护”与“owner 状态建模”拆开。

### 8. Chiaki-Style Route API

推荐在 stream 页 route controller 提供两条显式 API：

```ts
captureUiInput(reason: 'menu' | 'diagnostics' | 'sheet' | 'warning' | 'failed'): void
releaseUiInputAfterNeutral(reason: 'menu-close' | 'diagnostics-close' | 'sheet-close'): Promise<void>
```

语义固定为：

- `captureUiInput`
  - 立刻把 owner 收到 `ui`
  - consumer 执行自己的暂停副作用
- `releaseUiInputAfterNeutral`
  - 等待当前 pad neutral
  - consumer 恢复自己的发流副作用
  - owner 原子切回 `stream`

### 9. Delayed Recovery Policy

现有 delayed nudge / delayed interactive hint 政策调整为：

- 短期：
  - 保留一条低频、可关闭的 Windows-only fallback
  - 仅在 `startup + FSE active + gate 长时间未开 + sampling 已有事实` 时触发
- 中期：
  - 当 FSE 专线稳定后，默认关闭该 fallback
- 长期：
  - 删除固定 `500ms / 2s / 4s` 这类主链延迟补救

原则：

- 延迟补救只为容错，不为建模
- 任何需要多轮延迟补打才能稳定进输入链的路径，都视为主判定信号选错

### 10. Removal List

本轮从 routing 主线移除的内容：

- `streamSessionId`
- `streamConsumer`
- `streamUiSurface`
- `rustEngineStreamPadRoutedToSession`
- `chromeVisible`
- `window focus -> syncTarget`
- `document visibility -> syncTarget`
- routing 层里的 route ack 状态

本轮从 backend gate 主线移除的内容：

- `visible` 作为硬 gate 条件
- `minimized` 作为硬 gate 条件
- Tauri `Focused` 事件缓存作为唯一真相源
- `pageLoad / visibilitychange / mounted` 作为主恢复判据
- 多轮 delayed nudge 作为主恢复链

### 11. What Stays Stable

本轮保持稳定的边界：

- `gamepad-listener` 继续只在 owner=`ui` 时消费业务输入
- `GamepadDriver` 继续只在 owner=`stream` 时发业务输入帧
- 浏览器直连与 Rust consumer 继续各走原有执行链
- SDL3 runtime、sampling health、stalled self-heal 不在本 RFC 范围内调整

## Recommended Direction

推荐采用“极简 backend gate + FSE 专线 + 薄 arbiter + chiaki-style grab/release route controller + consumer 自持副作用”的结构。

理由有四点：

1. 这条路径直接削掉 gate 和 routing 的表达复杂度。
2. 这条路径把 FSE 从“桌面窗口异常”改成“有官方 API 的独立模式”处理。
3. 这条路径直接消掉“视觉层已关、输入归属未原子切回”的空窗。
4. 这条路径保持 UI 导航和 Stream consumer 的现有边界稳定。

## Plan

1. 收缩 backend gate 判定，只保留 `lifecycleActive + appIsForegroundCandidate` 两个硬条件。
2. Windows 接入 FSE 显式检测与 change notification，建立 FSE 专线路径。
3. 将 FSE 下 `appIsForegroundCandidate` 改为 Win32 foreground window 派生。
4. 将现有 delayed nudge 降级为低频 fallback，不再承担主恢复链。
5. 收缩 `business-input-arbiter` 状态模型，只保留四个核心字段。
6. 将 stream 页 surface 细分状态收口为单个 `overlayCapturing` 输出。
7. 引入 chiaki-style `captureUiInput / releaseUiInputAfterNeutral` route API。
8. 删除 routing 层中的 focus / visibility / ack / resync 逻辑。
9. 保留 neutral release 保护语义，位置放在 page controller，不扩散到 arbiter。
10. 用现有 UI 导航与 Stream consumer 测试行为做回归验收。

## Validation

- [x] backend gate 跨平台只依赖 `lifecycleActive + appIsForegroundCandidate`
- [x] `visible/minimized/document.visibilityState` 不再作为硬 gate 条件
- [x] Windows FSE 可被显式检测，并能在切换时收到变更通知（`GamingExperience.dll` 可用时）
- [x] Windows FSE 下 foreground window 事实可直接驱动 gate open/close
- [ ] delayed nudge 退出主恢复链后，冷启动输入不回退（需 Windows FSE 实机）
- [x] owner 派生只依赖 `appScene/backendGate/streamActive/overlayCapturing`
- [x] 打开任意 overlay 后 owner 稳定切到 `ui`
- [x] overlay 视觉层隐藏后，owner 继续保持 `ui` 直到 neutral release 完成
- [x] overlay 关闭并完成 neutral release 后 owner 稳定回到 `stream`
- [x] routing 层删除 focus / visibility resync 后，UI 导航与 Stream consumer 行为保持稳定
- [x] 浏览器直连与 Rust 模式共享同一套 owner 派生规则
- [x] diagnostics 或触屏菜单关闭后，不再落入“UI 与 Stream 都不消费”的空窗
- [ ] FSE 冷启动无输入问题不再被归因到 routing 模型复杂度（需实机确认 gate 链）

## Risks

- FSE 前台窗口事实与 Tauri window lifecycle 可能在短窗口内不一致，需要先接受“Win32 foreground 是 gate 真相源，Tauri 事件是诊断源”。
- 如果 FSE 下 foreground app 不是主窗口本身而是宿主代理窗口，需要补一层 HWND 归属映射。
- delayed fallback 过早移除时，可能暴露尚未收齐的 FSE 专线缺口。
- Rust 模式目前的 route ack 退出后，需要接受“routing 目标”和“consumer 副作用完成”分层表达。
- overlay 关闭后的 neutral release 如果位置不清晰，容易重新把保护语义塞回 arbiter。
- 现有 trace 字段会减少，需要同步调整诊断口径。

## Open Questions

1. ~~`streamActive` 是否直接等价于当前的 `streamSessionPresent`。~~ **已决**：是，已重命名落地。
2. ~~neutral release 是否继续保留在 `xstream-page-ui`。~~ **已决**：neutral 检测在 `wait-pad-neutral.ts`，route API 在 `stream-input-route-controller`。
3. ~~Rust 路径是否接受 arbiter 不等待 forwarding ack。~~ **已决**：接受。
4. FSE 下是否需要把“主窗口 HWND”扩展成“一组可接收前台输入的 HWND”映射，而不是单一句柄。（仍开放，实机观察）

## Progress

- [x] Step 1: 完成 Chiaki 对照与现状分析。
- [x] Step 2: 将范围收窄到“输入线路处理逻辑”，排除 UI 导航与 Stream consumer 改造。
- [x] Step 3: 形成精简前端 routing 模型与裁剪清单。
- [x] Step 4: 按 Microsoft FSE 文档补齐 gate / foreground 路线调整方案。
- [x] Step 5: 实现 backend gate 极简公式与 Tauri hints 写入。
- [x] Step 6: 实现 Windows FSE 专线（`fse_windows.rs`）。
- [x] Step 7: 降级 delayed nudge；新增 `gamepad_fse_gate_fallback_nudge`。
- [x] Step 8–10: 前端 routing / overlay / 单测（见 [`reports/2026-05-19-gamepad-gate-fse-and-routing-line.md`](../reports/2026-05-19-gamepad-gate-fse-and-routing-line.md)）。

## Execution Notes

- Date: 2026-05-19 | Status: implementation landed (code + unit tests)
- Date: 2026-05-19 | Status: follow-up fixes landed (`BackgroundWarm` foreground refresh, non-Windows gate simplification, cancellable neutral wait)
- Date: 2026-05-19 | Status: refined
- Update: 用户已将范围明确收窄到输入线路处理逻辑，要求精简前端模型，并将 Xbox/FSE/窗口焦点不稳定相关恢复逻辑移出 routing 主线。
- Decision: 本 RFC 不再推动 consumer 边界重写；当前只处理 gate 简化、FSE 专线、routing model、overlay contract 与 route-side 恢复逻辑裁剪。
- Decision: FSE 冷启动无输入问题继续视为 gate 信号与宿主前台判定问题，不在 routing 模型里继续叠补偿。
- Decision: 延迟补救仅保留为临时 fallback；主恢复链必须由显式 FSE 检测与 foreground 事实承担。
