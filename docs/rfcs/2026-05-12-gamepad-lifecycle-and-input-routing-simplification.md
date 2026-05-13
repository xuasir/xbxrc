# Gamepad Lifecycle And Input Routing Simplification RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: RFC drafted, awaiting execution confirmation
- Current State: planned
- Owner: codex
- Last Updated: 2026-05-12

## Background

- 当前 gamepad 主线已经具备 runtime lifecycle、`samplingHealth`、SDL3 source 常驻采样、自愈链，以及 Tauri / 前端恢复补偿。
- 但当前职责边界仍然混杂：
  - runtime contract 同时承载“采样事实”“壳层 lifecycle”“业务路由策略（`shared/ui-only/stream-only`）”
  - shell 与前端都在做 lifecycle correction / recovery settle / 轻恢复触发
  - `Active` 更像“允许外发”的控制标签，而不是“输入已可用”的事实判定
- 近期 Windows 11 Xbox 大屏 / FSE 日志已经证明：
  - runtime 可以处于 `Active`
  - device/source 事实已经成立
  - 但 `lastSampleProgressAtMs == 0`
  - 用户体感仍然是“无输入”
- 这说明当前实现把三类语义混在了一条合同里：
  - 持续采样
  - 壳层外发控制
  - 前端业务消费路由
- 目标应当回到更单纯的结构：
  - SDL3 runtime 持续采样
  - shell 唯一协调 `BackgroundWarm / Active`
  - 前端只消费采样，并在自身业务里决定“导航用 / 串流用 / overlay 用”

## Goal

- 固定 gamepad 主线为：
  - 一个持续运行的 SDL3 runtime
  - 一个由 shell 唯一协调的 lifecycle 外发门控层
  - 一个只负责消费输入的前端业务层
- 明确公开 lifecycle 只保留 `BackgroundWarm / Active`，只表达 runtime 外发语义，不再表达业务路由或输入可用性。
- 将真正的停机 / 清状态 / 退出收口为独立动作，而不是公开 lifecycle 状态。
- 将 `inputPolicy` 从 runtime 主合同降级为兼容字段，并最终移出 runtime / bridge / renderer 通道。
- 把“输入是否真实可用”的主判据固定为 logical progress（例如 `lastSampleProgressAtMs > 0`），而不是 lifecycle / baseline。
- 删除前端恢复状态机，让 shell 成为唯一 lifecycle / self-heal 协调层。

## Scope

- In scope:
  - `ohmygamepad-core` / `ohmygamepad-sdl3` runtime lifecycle 语义收紧
  - `src-tauri` shell / gamepad provider 成为唯一 lifecycle 协调层
  - `src/shared/gamepad/contract.ts` 与桥接 DTO 的 contract 简化
  - `AppShellLayout`、streaming 输入接入层、导航输入接入层的消费职责重分配
  - `inputPolicy` 的降级、兼容层与删除迁移计划
- Out of scope:
  - 更换 SDL3 后端或回退双轨手柄栈
  - 变更 Xbox 串流输入协议、包格式、rumble 合同
  - 在本 RFC 内完成所有历史 diagnostics 字段清理
  - 重做前端焦点系统或导航系统

## Problem Statement

当前系统存在四类结构性问题：

1. lifecycle 语义过载
- `Active` 目前混合了承诺：
  - runtime 正在运行
  - shell 希望允许外发
  - 输入已经可用
- 其中前两项是控制语义，第三项是事实语义，日志已经证明它们并不总是同步成立。

2. runtime contract 混入业务路由
- `shared / ui-only / stream-only` 本质上是前端消费路由语义，而不是底层采样模式。
- 让 runtime 携带这些业务态，会导致前后端都围绕同一字段做路由纠偏。

3. 恢复逻辑分散在三层
- SDL3 service：prime / reopen / refresh / self-heal
- shell：interactive hint / lifecycle correction / auto promote
- 前端：recovery token / retry / settle
- 结果是同一问题会被三层补偿，难以证明谁才是真正主线。

4. 可观测性中心不稳定
- 目前排障需要同时读：
  - lifecycle
  - health
  - device/source facts
  - `inputPolicy`
  - front-end recovery trace
- 这些信号之间存在重复和误导，导致复杂度上升。

## Design

### 1. 新的三层职责模型

#### Layer A: Persistent SDL3 Runtime

- SDL3 source / service / runtime 常驻运行。
- runtime 始终负责：
  - 设备事实
  - 原始样本摄入
  - logical pad 采样
  - progress 观测
- runtime 不再负责表达“当前前端想把输入给谁”。

#### Layer B: Shell Lifecycle Gate

- shell 是唯一 lifecycle 协调层。
- shell 根据窗口事实驱动：
  - `Active`
  - `BackgroundWarm`
- shell 负责：
  - 轻恢复触发
  - stalled / startup self-heal 调用
  - Warm -> Active promote
  - 统一 runtime trace
- shell 不再把恢复职责下放给前端。

#### Layer C: Frontend Consumers

- 前端只消费统一采样输出。
- 导航、串流页、overlay、modal 各自根据前端焦点/页面状态决定是否消费该输入。
- “输入发往导航还是发往串流会话”属于前端业务层，不再属于 runtime contract。

### 2. Lifecycle Semantics

#### `Active`

- runtime 持续 `poll_backend`
- runtime 持续 `sample_once`
- 允许向外广播 snapshot / delta / slot 更新
- 仅表示“外发开启”
- 不保证“输入已经可用”

#### `BackgroundWarm`

- runtime 持续 `poll_backend`
- runtime 持续 `sample_once`
- 内部 logical state 允许推进
- 不向外广播可操作输入
- 用于后台保温与快速恢复

#### Explicit Shutdown / Reset Action

- 不再作为公开 lifecycle 状态存在。
- 如果未来确实需要停机、释放资源、清状态或进程退出：
  - 使用独立 `shutdown / dispose / reset` 动作表达
  - 不把它混入日常 `BackgroundWarm / Active` 生命周期合同
- 这样可以避免把“极少走的停机路径”拖进主状态机。

### 3. Input Availability Contract

- “输入真实可用”不再由 lifecycle 推断。
- 唯一主判据固定为真实 logical progress：
  - `lastSampleProgressAtMs > 0`
  - 或其等价 progress 令牌前移
- runtime health 仅表达采样事实：
  - `AwaitingBaseline`
  - `Healthy`
  - `Stalled`
- shell 与前端均不得把“`Active` + connected + baseline”当作输入已可用的充分条件。

### 4. `inputPolicy` Decomposition

当前 `inputPolicy` 混合了两件事：

- 底层 runtime 是否向 UI/stream 侧外发
- 前端页面当前希望消费哪一路输入

本 RFC 的目标是拆开：

- runtime contract 只保留 lifecycle gate，不再维护 `shared/ui-only/stream-only`
- 前端消费层自行决定：
  - AppShell 是否消费导航输入
  - Stream 页是否消费串流输入
  - overlay / modal 是否拦截输入

迁移期间允许：

- `inputPolicy` 作为兼容字段保留在 bridge / snapshot 中
- 但它降级为 deprecated compatibility field
- 不允许新增逻辑继续依赖它做 lifecycle 或恢复判定

### 5. Recovery Simplification

#### Shell Owns Recovery

- shell 成为唯一恢复协调层：
  - `resume shell sampling`
  - `startup self-heal`
  - `stalled self-heal`
  - `BackgroundWarm -> Active` promote

#### Frontend Stops Owning Recovery State

- 前端移除：
  - recovery token
  - retry timer
  - recovery settle 状态机
- 前端仅保留：
  - 焦点 / 页面事实
  - 消费层启停
  - UI diagnostics 展示

### 6. Output Model

runtime 对外统一输出应收敛为：

- runtime snapshot
  - lifecycle
  - health
  - devices
  - slots
  - `lastSampleProgressAtMs`
  - `lastBackendSampleActivityAtMs`
- 可选 slot / delta 事件
  - 仅在 `Active` 下外发

前端所有页面只从统一输出消费，不再从不同业务态推导不同版本的“手柄恢复成功”。

## Migration Plan

### Step 1. 固定语义，不先删字段

- 保留现有 `inputPolicy` 字段
- 但在 RFC 和代码注释中把它标记为 deprecated compatibility field
- 所有新的 lifecycle / recovery 逻辑不得继续依赖它

### Step 2. shell 成为唯一 lifecycle 协调层

- 所有 `window focus / visibility / mount / fullscreen cold start` 事实都收口到 shell
- shell 成为唯一 `setSamplingLifecycle / resume / self-heal` 发起者
- 前端不再做 lifecycle correction
- runtime 公开 lifecycle 合同收敛为 `Active / BackgroundWarm`

### Step 3. 删除前端 recovery 状态机

- 删除 `AppShellLayout` 中的：
  - pending recovery token
  - baseline progress settle
  - retry timer
- 仅保留 snapshot 消费和 shell 事实通知

### Step 4. 前端接管输入归属

- 导航输入消费在 AppShell / navigation 层
- 串流输入消费在 Stream 页 / player 输入层
- overlay / modal 用前端焦点系统决定拦截优先级

### Step 5. 删除 runtime contract 中的业务路由字段

- 当所有消费层都不再依赖 runtime `inputPolicy` 后：
  - 从 Rust DTO / bridge / shared contract / renderer 侧移除
  - 保留必要的迁移 trace / compatibility window

## Validation

- [ ] `BackgroundWarm` 下 runtime 继续采样并允许 progress 推进，但不会向前端消费层外发可操作输入。
- [ ] `Active` 只表达“外发开启”，不再被任何逻辑当作“输入已可用”的充分条件。
- [ ] 停机 / reset / 退出能力不再通过公开 lifecycle 第三态表达，而是通过独立动作表达。
- [ ] shell 成为唯一 lifecycle 协调层；前端不再主动做 lifecycle correction。
- [ ] 前端删除 recovery 状态机后，Xbox 大屏 / FSE 首开输入恢复不回退。
- [ ] `inputPolicy` 从 runtime 主合同降级后，导航与串流输入消费仍能稳定切换。
- [ ] 日志中能清晰区分：
  - runtime active but no progress
  - runtime active and progress established
  - runtime background warm with internal progress
- [ ] 串流页 overlay / modal / 壳层导航对输入归属的切换不会再通过 runtime 策略字段编码。

## Risks

- 当前导航输入、串流输入、overlay 输入都已经与 `inputPolicy` 有一定历史耦合，迁移时容易出现“谁都能听到，谁也没拦住”的回归。
- 如果 shell 协调层收口不彻底，前端删掉 recovery 状态机后可能暴露真实后端缺口。
- `BackgroundWarm` 的“只保温不外发”如果没有统一事件门控，仍可能泄漏旧边沿。
- 删除 `inputPolicy` 需要跨 Rust DTO、bridge、shared contract、renderer listener、stream page 多层改动，迁移窗口必须分步进行。

## Progress

- [ ] Step 1: 产出简化 RFC，固定三层职责模型与迁移顺序
- [ ] Step 2: 让 shell 成为唯一 lifecycle/self-heal 协调层，并把公开 lifecycle 收敛为两态
- [ ] Step 3: 删除前端 recovery 状态机
- [ ] Step 4: 将 `inputPolicy` 从 runtime 合同降级并最终删除

## Execution Notes

- Date: 2026-05-12 | Status: planned
- Update: 基于近期 Windows 11 Xbox 大屏 / FSE runtime logs，确认 `Active` 当前只能作为“外发开启”的控制标签，不能继续充当“输入已可用”判据；因此提出新主线：“SDL3 runtime 常驻采样 + shell 唯一协调 lifecycle + 前端只消费采样”。
- Decision: 公开 lifecycle 不保留 `Suspended`。如果未来需要停机、清状态、释放资源或退出，改用独立动作表达，不再污染主状态机。
- Decision: `inputPolicy` 从 runtime 主合同降级为待删除兼容字段；导航 / 串流 / overlay 输入归属改由前端业务层决定。
- Decision: 不继续在前端堆 recovery token / retry 窗；恢复主线统一下沉到 shell。
- Risk/Blocker: 当前已有大量历史逻辑围绕 `shared/ui-only/stream-only` 组织，真正迁移前需要先做调用点清单与分阶段 cutover 计划。
