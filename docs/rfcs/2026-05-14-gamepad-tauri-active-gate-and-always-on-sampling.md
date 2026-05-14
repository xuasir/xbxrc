# Gamepad Tauri Active Gate And Always-On Sampling RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: RFC drafted, awaiting execution confirmation
- Current State: planned
- Owner: codex
- Last Updated: 2026-05-14

## Background

- 当前 SDL3 手柄 source 已经具备常驻线程、后台事件、主动 poll snapshot 和 `reopen + prime + refresh` 自愈链。
- 当前首次启动无输入问题已经不再主要表现为“SDL 没有采到输入”，而是出现在更上层的收敛链：
  - runtime lifecycle
  - logical progress
  - shell promote / self-heal
  - 前端消费建链
- 当前系统把三类语义混在一起：
  - 物理采样是否持续进行
  - 当前窗口是否处于业务可交互态
  - 输入应该发往 UI 还是 Rust 串流业务
- 这导致系统需要依赖 `BackgroundWarm / Active`、`inputBaselineAbsorbed`、`resumeShellSampling`、前端 snapshot 重放等多条恢复链路，复杂度高，且在 Xbox FSE 场景下容易形成“采样活着但业务链没开”的中间态。
- 参考 Chiaki 的 SDL 模型，更稳定的结构是：
  - SDL 持续采样
  - 窗口 active 只决定事件是否继续进入业务处理
  - 业务消费链尽量短

## Goal

- 固定 gamepad 主线为：
  - SDL / runtime 常驻采样
  - Tauri 后端统一维护业务输入门控
  - 前端 UI 与 Rust 侧 xbxengine 只消费门控后的业务输入
- 将窗口阶段职责收敛为：
  - 更新 `active` / `visible` / `minimized` / `ownership`
  - 由 Tauri 后端决定物理输入是否继续进入业务链
- 让前端停止关心“采样是否已恢复”，只负责监听和决定输入用在哪里。
- 让 Rust 侧 `xbxrc/xbxengine` 停止自行猜测前后台状态，只消费后端已经门控后的输入。

## Scope

- In scope:
  - `ohmygamepad-core` 业务输入门控模型调整
  - `src-tauri` 统一维护 `active gate` 与输入 ownership
  - `src-tauri/mods/gamepad`、shell、bridge 输出合同重构
  - 前端导航 / 玩家输入层改为常驻监听门控后输入
  - Rust 侧串流输入转发改为消费同一条门控后业务输入
  - `BackgroundWarm / Active`、`resumeShellSampling`、`inputBaselineAbsorbed` 的降级与迁移计划
- Out of scope:
  - 更换 SDL3 手柄后端
  - 修改 Xbox 串流协议、输入包格式、rumble 合同
  - 重做前端焦点系统或几何导航算法
  - 处理设备映射兼容策略与机型识别扩展

## Problem Statement

当前结构存在四个核心问题：

1. 采样与业务门控耦合
- `BackgroundWarm / Active` 同时承担“采样态”和“业务外发态”。
- 结果是 SDL source 已经采到数据，业务事件仍可能没有进入 UI 或 Rust 侧消费链。

2. 恢复职责跨三层分散
- SDL3 service 负责 `reopen / prime / refresh`
- Tauri shell 负责 lifecycle / promote / hint
- 前端负责 snapshot 重放、baseline absorb、消费建链
- 同一问题会被多层补偿，难以定义唯一主线。

3. 前端和 Rust 侧都在做业务输入判定
- 前端导航层根据页面、overlay、focus 重新判断是否消费
- Rust 侧串流输入转发又根据 route / forwarding 状态再判断一次
- 缺少一个统一后端门控，导致 UI 和 Rust 侧语义容易分叉。

4. 首次启动输入链过长
- 当前“首次可用输入”依赖 runtime lifecycle、logical progress、slot broadcast、baseline absorb、前端 listener 等多环节串联。
- 任意一环延迟或误判，用户体感就是首次启动无输入。

## Decision

- SDL source 与 runtime 常驻采样继续保留，窗口 active 不再承担重建采样链的职责。
- Tauri 后端新增统一业务输入门控层，负责决定“物理输入是否继续进入业务处理”。
- 业务输入门控同时覆盖：
  - 前端 UI 导航
  - Rust 侧串流输入转发 / xbxengine
- 前端与 Rust 侧都只消费门控后的业务输入，不再各自维护一套前后台判定。
- `BackgroundWarm / Active` 从主业务合同降级为内部采样 / 诊断语义，并逐步退出主业务链。

## Design

### 1. 三层模型

#### Layer A: Physical Sampling

- SDL source 常驻运行。
- 始终负责：
  - `SDL_PollEvent`
  - 设备热插拔
  - 当前物理态快照
  - 必要的 `reopen / prime / refresh`
- 这一层只回答“物理输入是什么”，不回答“业务是否应该处理”。

#### Layer B: Tauri Active Gate

- Tauri 后端维护统一 `input gate`。
- gate 输入条件至少包括：
  - `window active`
  - `visible`
  - `minimized`
  - `ownership`
- gate 输出是两路稳定合同：
  - `physical snapshot`
  - `business input`

#### Layer C: Business Consumers

- 前端 UI 导航
- 前端播放器输入层
- Rust 侧 `xbxrc/xbxengine` 串流输入转发

这三类消费者只处理 `business input`，不再直接解释 lifecycle / baseline / warm promote。

### 2. Active Gate Contract

后端 gate 只回答一个问题：

- 当前物理输入是否继续进入业务处理

推荐的 gate 状态：

- `closed`
  - 物理采样继续
  - 不生成 UI / stream 业务输入
- `ui`
  - 物理采样继续
  - 只生成 UI 业务输入
- `stream`
  - 物理采样继续
  - 只生成 Rust 串流业务输入
- `shared`
  - 物理采样继续
  - 同时生成 UI 与 Rust 业务输入

`ownership` 是业务语义，不再让前端和 Rust 侧各自拼装。

### 3. Output Split

后端对外拆成两路输出：

#### `physical snapshot`

- 用于：
  - 诊断
  - 设备卡片
  - 自愈判断
  - 调试面板
- 始终可读

#### `business input`

- 用于：
  - 导航按键
  - repeat
  - combo
  - stream pad forwarding
  - Rust 侧 xbxengine 输入包
- 仅在 gate open 时生成

### 4. Frontend Simplification

- `gamepad-listener`、`GamepadDriver` 改为尽早启动、持续监听。
- 前端不再关心：
  - `resumeShellSampling`
  - `inputBaselineAbsorbed`
  - `window-focus/document-visible` 补 runtime snapshot 建链
- 前端只关心：
  - 当前收到了哪些业务输入
  - 当前页面/组件要不要消费这些业务输入

### 5. Rust Consumer Simplification

- Rust 侧 `xbxrc/xbxengine` 不再直接读取未门控 pad 样本。
- 串流输入包只从后端门控后的业务输入生成。
- 这样 UI 与 Rust 侧消费链共享同一份 active / ownership 语义。

### 6. Lifecycle Decomposition

- `BackgroundWarm / Active` 不再作为主业务外发合同。
- 生命周期可暂时保留作内部诊断与兼容字段。
- `inputBaselineAbsorbed` 从主业务链退出，迁移为可删除兼容事件。
- `resumeShellSampling` 从窗口业务恢复主入口降级为内部采样维护工具。

### 7. Self-Heal Boundary

- `stalled` 自愈保留，因为它解决真实采样故障。
- `startup-active-without-progress`、`window-focused -> resume recovery` 这类“窗口驱动恢复链”从主业务路径移除。
- 自愈只处理“采样坏了”，不处理“业务门控没开”。

## Plan

1. 定义统一 `input gate` 合同，并在 Tauri 后端集中维护 `active + ownership` 状态。
2. 将 UI 导航与 Rust 串流输入转发改为消费统一 `business input`，拆掉双边重复判定。
3. 将 `BackgroundWarm / Active`、`resumeShellSampling`、`inputBaselineAbsorbed` 从主业务链降级为兼容或诊断语义。

## Validation

- [ ] SDL source 持续采样时，窗口失焦不会误把输入送进 UI 或 Rust 业务链。
- [ ] Windows / Xbox FSE 首次启动时，无需依赖前端恢复状态机即可建立业务输入链。
- [ ] 前端 `gamepad-listener` 与 `GamepadDriver` 改为常驻监听后，输入去向仍可稳定切换。
- [ ] Rust 侧 `xbxengine` 只消费门控后的业务输入，前后台行为与 UI 侧保持一致。
- [ ] `stalled` 自愈仍能处理真实采样冻结，不再承担窗口态补偿职责。
- [ ] diagnostics 仍能区分“采样正常但 gate closed”与“采样链真实故障”。

## Risks

- 这次改造会触及 `ohmygamepad-core`、`src-tauri`、前端输入层、Rust 串流输入转发四层合同，回归面较大。
- 当前很多逻辑默认 `slotSnapshot` 只在 `Active` 下广播，改成统一 gate 后需要重写测试与消费边界。
- `inputBaselineAbsorbed` 退出主链后，需要重新定义 pressed / repeat / combo 的重置时机。
- UI 与 Rust 共用一套 gate 后，ownership 定义如果不够清晰，容易出现“UI 与 stream 同时误消费”。

## Progress

- [x] Step 1: 形成基于 Chiaki 对照的目标结构，明确 SDL 常驻采样 + Tauri 统一业务门控方向。
- [ ] Step 2: 定义 gate 合同与跨层迁移策略。
- [ ] Step 3: 执行运行时、前端、Rust 侧消费者改造并完成验证。

## Execution Notes

- Date: 2026-05-14 | Status: planned
- Update: 新建 RFC，目标是将 gamepad 主线从“lifecycle 驱动恢复 + 前后端分散判定”收口为“SDL 常驻采样 + Tauri 后端统一业务门控”。
- Decision: `active` 门控放在 Tauri 后端，覆盖前端 UI 和 Rust 侧 xbxengine 两条业务链；前端与 Rust 侧都只消费门控后的业务输入。
- Risk/Blocker: 当前已有 runtime lifecycle、baseline absorb、slot broadcast、stream forwarding 等既有合同，执行阶段需要分批迁移，避免一次性击穿输入主线。
