# Gamepad Business Input Ownership UI Stream Routing RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: Step 5 implemented (frontend)
- Current State: implemented
- Owner: codex
- Last Updated: 2026-05-14

## Background

- 当前 gamepad 主线已经完成一轮收口：
  - SDL / runtime 常驻物理采样
  - Tauri 后端统一维护 coarse `input_gate`
  - `runtimeSnapshot.slots` 恢复为 physical snapshot
- 但“业务输入应该给谁”这件事仍没有单一真相源。
- 当前业务输入归属分散在多处状态：
  - `streamSessionActive`
  - `streamShellUiPriority`
  - `streamPadForwarding`
  - stream 页 overlay / sheet 状态
  - 浏览器直连场景下的 `suspendRtcGamepadTransport`
- 这会让系统在 stream 页菜单打开、关闭、确认动作时出现几类典型风险：
  - UI 与 Stream 同时吃输入
  - UI 与 Stream 都不吃输入
  - 浏览器直连与 Rust 渲染器行为分叉
  - chrome 仅显示但并不可交互时，错误抢回 UI owner
- 现有代码已经有一些正确约束，需要保留：
  - 主窗口仍是唯一输入入口
  - `chromeVisible` 不等于 overlay open，不应单独抢走 Stream 输入
  - `gamepad-listener` 与 `GamepadDriver` 已逐步改为更早启动、更长驻的监听模式

## Goal

- 重新设计业务层的 UI / Stream 两条输入生效通道。
- 固定默认行为为：
  - 大部分时间 UI 生效
  - 进入串流游玩时 Stream 独占
- 定义 stream 页菜单相关场景的明确切换合同：
  - 主菜单打开
  - Diagnostics 菜单打开
  - 各 sheet 打开
  - 菜单关闭
  - 菜单确认动作后的归属结果
- 兼容两种 Stream 消费模式：
  - 浏览器直连
  - Rust 渲染器
- 明确前端输入接入层需要“尽早监听、常驻监听”，不再依赖延后挂载或临时建链。

## Scope

- In scope:
  - 前端业务层 `UI / Stream` owner 模型
  - stream 页菜单 / overlay / sheet 的输入归属合同
  - 浏览器直连与 Rust 渲染器两种 Stream consumer 的统一抽象
  - `gamepad-listener`、`GamepadDriver` 的尽早监听与常驻监听要求
  - 与 Tauri coarse `input_gate` 的职责边界
- Out of scope:
  - 重做 SDL3 采样 runtime
  - 重做空间导航系统
  - 修改 Xbox 串流协议与输入包格式
  - 修改 rumble 主合同

## Problem Statement

当前结构存在四个核心问题：

1. 业务 owner 不是单一状态
- 当前系统通过多个布尔量组合表达“谁吃输入”。
- 这会让状态过渡依赖副作用顺序，而不是依赖一条稳定合同。

2. 菜单行为没有统一输入后果
- 打开菜单、进入 diagnostics 菜单、打开 display/audio/text、确认某个菜单动作，都会改变业务输入归属。
- 但目前这些动作的 owner 后果散落在页面逻辑里，不利于统一验证。

3. 浏览器直连与 Rust 渲染器分叉风险高
- 浏览器直连的 Stream consumer 在前端 `GamepadDriver/InputService`
- Rust 渲染器的 Stream consumer 通过 `setStreamPadForwarding` 落到 Rust 侧链路
- 如果业务层不先统一 owner，两个场景会逐步长成两套切换规则。

4. 前端输入接入层启动时机仍不够明确
- 业务 owner 正在从“恢复链驱动”转向“常驻监听 + owner 仲裁”。
- 如果前端监听器挂载过晚，仍会出现：
  - 首帧 baseline 错过
  - 菜单切回后需要额外补 snapshot
  - owner 已切好但消费者尚未开始监听

## Decision

- Tauri / Rust 后端继续只负责：
  - physical sampling
  - coarse `input_gate`（业务链是否打开）
- “UI 与 Stream 谁生效”收口为前端业务层唯一仲裁器，不再下沉到 runtime 合同。
- 业务层只允许一个显式 owner：
  - `ui`
  - `stream`
  - `none`
- stream 页的菜单、diagnostics 菜单、display/audio/text/failed/warning 等 sheet 都视为 UI owner 场景。
- `chromeVisible` 仅表达视觉 chrome 是否显示，不单独决定 owner。
- 前端输入接入层必须尽早启动、常驻监听：
  - `gamepad-listener` 尽早启动并常驻
  - `GamepadDriver` 尽早启动并常驻
  - owner 变化只改变“谁消费业务输入”，不改变“谁开始监听物理/业务事件”

## Design

### 1. 三层职责模型

#### Layer A: Physical Sampling And Coarse Gate

- SDL / runtime / Tauri 负责：
  - 物理采样
  - `input_gate=open/closed`
  - physical snapshot
- 后端只回答：
  - 当前物理输入是否允许进入业务输入链
- 后端不回答：
  - 业务输入该给 UI 还是 Stream

#### Layer B: Frontend Business Input Arbiter

- 前端新增统一业务输入仲裁器，作为 UI / Stream owner 的唯一真相源。
- 仲裁器负责：
  - 持有当前业务场景状态
  - 派生当前 owner
  - 协调 owner 切换顺序
  - 驱动不同 Stream consumer adapter

#### Layer C: Business Consumers

- UI consumer：
  - `gamepad-listener`
  - 空间导航 / 弹层 / 菜单
- Stream consumer：
  - 浏览器直连：`GamepadDriver` + `InputService`
  - Rust 渲染器：`setStreamPadForwarding` 对应的 Rust 侧输入链

### 2. Unified Business Route State

建议前端仲裁器维护如下状态：

```ts
type BusinessInputOwner = 'ui' | 'stream' | 'none'

type StreamUiSurface =
  | 'none'
  | 'menu'
  | 'diagnosticsMenu'
  | 'display'
  | 'audio'
  | 'text'
  | 'failed'
  | 'warning'

interface BusinessInputRouteState {
  appScene: 'shell' | 'stream'
  backendGate: 'open' | 'closed'
  streamSessionId: string | null
  streamConsumer: 'browser-player' | 'rust-engine' | 'none'
  streamUiSurface: StreamUiSurface
  chromeVisible: boolean
}
```

派生逻辑固定为：

1. `backendGate !== 'open'` -> `none`
2. `appScene !== 'stream'` -> `ui`
3. `streamSessionId == null` -> `ui`
4. `streamUiSurface !== 'none'` -> `ui`
5. 否则 -> `stream`

### 3. Owner Semantics

#### `ui`

- 导航 / 菜单 / sheet / overlay 消费业务输入
- Stream consumer 不得继续吃业务输入

#### `stream`

- 串流会话独占业务输入
- UI 层继续监听，但不得消费业务输入

#### `none`

- coarse gate 已关闭
- UI 与 Stream 都不消费业务输入
- 但物理采样和 physical snapshot 继续存在

### 4. Stream Page Menu Rules

#### A. Playing

- 条件：
  - `appScene=stream`
  - `streamSessionId!=null`
  - `streamUiSurface=none`
  - `backendGate=open`
- owner：`stream`

#### B. Open Main Menu

- 触发：
  - stream 菜单按钮
  - 菜单键
  - `menu+view`
- 切换：`stream -> ui`

#### C. Switch Main Menu <-> Diagnostics Menu

- `menu -> diagnosticsMenu`
- `diagnosticsMenu -> menu`
- 切换：`ui -> ui`
- owner 不变，只变 surface

#### D. Open Display / Audio / Text Sheet

- `menu -> display/audio/text`
- 切换：`ui -> ui`
- owner 不变

#### E. Warning / Failed Sheet

- `warning` / `failed` 视为高优先级 UI surface
- owner 固定为 `ui`

#### F. Close Overlay And Resume Game

- `menu/diagnosticsMenu/display/audio/text/warning -> none`
- 切换：`ui -> stream`

### 5. Menu Action Outcome Contract

建议每个菜单动作显式声明输入后果，而不是隐式靠页面副作用推导：

```ts
type ActionInputOutcome =
  | { kind: 'stay-ui', nextSurface?: StreamUiSurface }
  | { kind: 'resume-stream' }
  | { kind: 'leave-stream' }
```

示例：

- `display` / `audio` / `sendText` / `diagnosticsMenu`
  - `stay-ui`
- `back` / `close`
  - `resume-stream` 或回到上级 `stay-ui`
- `exit` / `powerOffExit`
  - `leave-stream`

这样菜单“确认”本身就是 owner 切换合同的一部分。

### 6. Stream Consumer Adapter

业务仲裁器不直接感知浏览器直连和 Rust 渲染器细节，而是调用统一适配器：

```ts
interface StreamInputConsumerAdapter {
  activateStreamInput(): Promise<void>
  deactivateStreamInput(): Promise<void>
}
```

#### Browser Player Adapter

- 负责浏览器直连场景：
  - 让 `GamepadDriver` / `InputService` 作为 Stream consumer
  - overlay 打开时暂停 RTC gamepad transport
  - 保留明确白名单的程序化单帧 bypass

#### Rust Engine Adapter

- 负责 Rust 渲染器场景：
  - `rpc.gamepad.setStreamPadForwarding(true/false)`
  - 必要时清理 rumble / reset input edge

### 7. Early Listening Requirement

前端监听策略明确为：

- `gamepad-listener` 尽早启动、常驻监听
- `GamepadDriver` 尽早启动、常驻监听
- owner 变化只改变消费权，不改变监听生命周期
- stream 页进入前不等待菜单、overlay、player transport 就绪后才开始监听
- renderer 晚挂载、overlay 晚打开、session 晚建立都不应再成为“开始监听”的前置条件

这条要求的目的有三点：

1. 避免错过初始 baseline / runtime snapshot
2. 避免 owner 已切换但消费者还没挂好
3. 避免在浏览器直连与 Rust 渲染器场景下出现不同步的首轮输入建链

### 8. Transition Protocol

owner 切换需要统一顺序，不再散落在页面逻辑里。

#### `stream -> ui`

- 更新 `streamUiSurface`
- reset UI listener 边沿态
- 停止 Stream consumer
- 启用 UI consumer

#### `ui -> stream`

- 先确认 `streamUiSurface=none`
- 启用 Stream consumer
- 清理 UI 优先态
- reset UI listener 避免旧 pressed/repeat/combo 残留

#### `ui -> ui`

- owner 不变
- 只切换 `streamUiSurface`

#### `leave-stream`

- owner 最终回到 `ui`
- `streamSessionId` 清空
- Stream consumer 停止

## Compatibility With Existing Contracts

- 保留后端 coarse `input_gate`：
  - 负责业务输入链总闸
  - 不负责 UI/Stream owner
- 保留 `runtimeSnapshot.slots` 作为 physical snapshot
- `streamPadForwarding` 降级为 Rust 渲染器 adapter 的实现细节
- `streamSessionActive` / `streamShellUiPriority` 逐步迁移为业务仲裁器内部状态，不再作为对外业务合同

## Plan

1. 定义统一 `business-input-arbiter` 状态与 owner 派生规则。
2. 为浏览器直连与 Rust 渲染器补齐 `StreamInputConsumerAdapter`。
3. 将 stream 页菜单动作改为显式 `ActionInputOutcome`。
4. 将 `gamepad-listener` 与 `GamepadDriver` 的“尽早监听、常驻监听”写成实现要求并补测试。
5. 删除页面内散落的 owner 切换副作用顺序逻辑。

## Validation

- [ ] 非 stream 页默认 owner=`ui`。
- [ ] stream 播放中 owner=`stream`。
- [ ] 打开主菜单后 owner 从 `stream` 切到 `ui`。
- [ ] `menu -> diagnosticsMenu -> display/audio/text` 过程中 owner 始终保持 `ui`。
- [ ] 关闭 overlay 回到游戏后 owner 从 `ui` 切回 `stream`。
- [ ] `warning` / `failed` 打开时 owner 固定为 `ui`。
- [ ] `chromeVisible=true` 但没有交互 surface 时，owner 仍保持 `stream`。
- [ ] 浏览器直连与 Rust 渲染器两种模式共享同一套 owner 测试，只替换 adapter。
- [ ] `gamepad-listener` 与 `GamepadDriver` 均满足尽早监听、常驻监听，不再依赖晚挂载建链。

## Risks

- 如果 owner 切换协议没有收口到统一仲裁器，页面侧仍可能保留隐式竞态。
- 如果把 `chromeVisible` 与 `overlayOpen` 混用，容易再次出现“顶部 chrome 只是显示但游戏输入被抢走”。
- 如果浏览器直连与 Rust 渲染器不通过 adapter 抽象统一，后续功能会继续长成两套业务规则。
- 尽早监听会扩大“监听常驻、消费受控”的代码面，需要补 pressed/repeat/combo reset 相关验证。

## Progress

- [x] Step 1: 完成 UI / Stream owner 新模型设计。
- [x] Step 2: 明确 stream 页菜单、diagnostics 菜单、sheet、warning/failed 的 owner 合同。
- [x] Step 3: 明确浏览器直连与 Rust 渲染器两种 adapter 边界。
- [x] Step 4: 将“前端尽早监听、常驻监听”上升为正式决策。
- [x] Step 5: 实现拆分与验证：`business-input-arbiter` + `input-routing` 薄封装 + Stream 页 `streamUiSurface` 归一 + Rust pad 路由 API + `rustEngineStreamPadRoutedToSession` 竞态门 + Vitest 矩阵；`GamepadDriver` 移除 focus/visibility 补快照链改由仲裁器 owner 判定。

## Execution Notes

- Date: 2026-05-14 | Status: implemented (frontend arbiter)
- Update: 新建 RFC，目标是把 UI / Stream 两条业务输入生效通道收口为前端业务层唯一 owner 仲裁器。
- Decision: 后端只维护 coarse `input_gate`，不承担 UI/Stream owner；owner 完全由前端业务层根据 scene/session/surface 派生。
- Decision: stream 页菜单、diagnostics 菜单、display/audio/text/failed/warning 全部归为 UI owner 场景；关闭后回到 Stream owner。
- Decision: `gamepad-listener` 与 `GamepadDriver` 必须尽早监听、常驻监听，owner 切换只改变消费权，不改变监听生命周期。
- Update (2026-05-14): 已落地 `src/shared/gamepad/business-input-arbiter.ts` 为单一真相源；Rust 渲染路径在 `streamUiSurface===none` 时仍要求 `rustEngineStreamPadRoutedToSession===true`（由 `useGamepadRouteForStreamOverlay` 在 RPC 成功后写入），以对齐旧 `streamShellUiPriority` 与 `setStreamPadForwarding` 之间的顺序窗口。
