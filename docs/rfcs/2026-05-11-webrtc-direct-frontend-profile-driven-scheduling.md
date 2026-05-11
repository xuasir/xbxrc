# `webrtc-direct` 前端画像驱动调度 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成（首轮实现 + 单测；实机会话验收见 Report Follow-up）
- Current State: implemented
- Owner: Codex
- Last Updated: 2026-05-11

## Background

- 当前 `webrtc-direct` 浏览器直连运行时已经掌握多类画像输入：启动前已知 `targetType=home/cloud`，运行期还能拿到 `transportPath`、stats、前端 render/decode 指标，以及来自后端 snapshot 的 `remoteProfileBaseline / remoteProfileDynamic / remoteProfileEffectiveLabel`。
- 但前端主调度仍主要围绕统一的 `warmup + bandwidthState + qualityLadder + displayDegrade` 展开，导致 `home-lan`、`home-relay`、`cloud` 共享过多阈值，`30fps` 与 `60fps` 内容也共享绝对 `fps` 门槛。
- 这个结构会在两类场景里持续暴露问题：
  - `home-lan` 局域网低 RTT、低 jitter 会话仍被统一 warmup 和保守码率线压在 `L1`
  - 原生 `30fps` 游戏因为绝对 `presentFps/decodeFps` 门槛被长期视作 `warning`
- Rust 侧已经有更成熟的画像主线：基线画像 `HomeLanGaming / CloudGaming / RelayGaming`，动态子画像 `steady / cloudStartup / cloudHighRtt / decoderConstrained / displayConstrained`。浏览器前端调度需要一条与之同向、但保持轻量的画像驱动合同。

## Goal

- 为 `webrtc-direct` 前端运行时建立一层明确的 `RuntimeProfileClassification` 合同，统一消费启动事实、ICE/transport path、运行期 stats 与内容帧率信息。
- 将 `warmup`、`quality ladder`、`display degrade`、`bandwidth gate` 从统一启发式收口为按画像驱动的策略表。
- 让 `home-lan`、`home-relay`、`cloud` 拥有不同的启动窗口、升降档阈值与拥塞解释方式。
- 让 `30fps` 内容使用相对内容帧率门槛，避免继续被 `60fps` 假设误伤。
- 为 diagnostics / runtime trace 增加前端画像与策略观测字段，使后续调参建立在可验证证据上。

## Scope

- In scope:
  - [`src/streaming/runtime/browser-runtime.ts`](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/runtime/browser-runtime.ts)
  - [`src/streaming/runtime/runtime-contract.ts`](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/runtime/runtime-contract.ts)
  - [`src/streaming/runtime/xbxengine-runtime.ts`](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/runtime/xbxengine-runtime.ts)
  - [`src/streaming/diagnostics.ts`](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/diagnostics.ts)
  - [`src/streaming/types.ts`](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/types.ts)
  - `webrtc-direct` 运行时 trace / diagnostics / snapshot 中与前端画像和策略观测有关的字段
- Out of scope:
  - 重写 Rust 侧 recovery / BWE / RTC 状态机
  - 改动 `xbxengine` 已存在的后端画像合同含义
  - 引入新的浏览器运行时、第二套媒体栈或并行发送链
  - 本 RFC 首轮内直接重设计 SDP patch / ICE policy 的底层算法

## Current Problem Breakdown

### 1. 前端有画像输入，少画像调度

- `targetType` 在会话路由和 runtime launch 阶段就已稳定存在。
- `transportPath`、远端画像字段和媒体 stats 在连接后持续可见。
- 当前 `browser-runtime` 只把这些信号局部用于日志或 capability 分支，没有成为统一策略入口。

### 2. 统一 `warmup` 对 `home-lan` 过保守

- 连接建立后固定进入 `L1 + displayL1` warmup，持续时间统一。
- 这个策略对 cloud 启动慢热有保护价值，对 `home-lan` 低抖动直连的收益较小，体验代价更明显。

### 3. 统一 `bandwidthState` 把链路画像和内容画像混在一起

- 当前 `warning / congested / recovering / stable` 的判定同时承担：
  - 网络健康解释
  - 启动期保守保护
  - 内容帧率解释
  - 显示/解码约束解释
- 单一状态承载过多语义，导致调度意图不清楚，trace 也难解释“为何仍停在 L1”。

### 4. 绝对 `fps` 门槛误伤 `30fps` 内容

- `presentFps < 30`、`decodeFps < 30` 这类规则默认把 `30fps` 看作下限线。
- 原生 `30fps` 内容即使稳定输出，只要轻微波动，就会频繁被压回 `warning`。

## Proposed Direction

### A. 前端新增 `RuntimeProfileClassification`

建议前端引入三层画像：

1. 基线画像 `baseline`
   - `homeLan`
   - `homeRelay`
   - `cloud`

2. 动态子画像 `dynamic`
   - `startup`
   - `steady`
   - `highRtt`
   - `decoderConstrained`
   - `displayConstrained`

3. 内容画像 `contentFpsClass`
   - `content30`
   - `content60`
   - `contentUnknown`

### B. 画像输入面

建议分类器输入按优先级收口为：

- 启动事实
  - `targetType`
- 链路事实
  - `transportPath`
  - local/remote candidate 类型
  - relay / non-relay
  - 地址族组合
- 运行时 stats
  - `rtt`
  - `loss`
  - `feedbackInterval`
  - `inboundVideoBitrateKbps`
  - `packetAgeMs`
  - `presentAgeMs`
  - `decodeFps`
  - `presentFps`
- 后端透传事实
  - `remoteProfileBaseline`
  - `remoteProfileDynamic`
  - `remoteProfileEffectiveLabel`
- 内容事实
  - 远端显式 `fps` 信息
  - 近窗稳定上沿估计

### C. 策略表 `ProfilePolicyPreset`

前端调度不再硬编码统一阈值，而改成画像表驱动。每个画像 preset 至少包含：

- `warmupDurationMs`
- `qualityLadderInitLevel`
- `displayInitLevel`
- `warning` / `congested` 判定阈值
- `L0/L1/L2` 升降档阈值
- `displayL0/L1/L2` 升降档阈值
- `sdpDownshift` 启动阈值
- `fpsExpectation`

建议首版 preset：

- `homeLan`
  - 短 warmup
  - 更积极升回 `L0`
  - 更依赖 `age/loss`，弱化绝对码率门槛
- `homeRelay`
  - 中等 warmup
  - 中性阈值
  - 更关注 relay path 与反馈延迟
- `cloud`
  - 长 warmup
  - 保守 startup 和拥塞阈值
  - `highRtt` 进入专门子画像

### D. 相对内容帧率门槛

`fps` 判定改成相对内容期待值，而不是统一绝对线。

建议规则：

- `effectivePresentRatio = presentFps / expectedContentFps`
- `effectiveDecodeRatio = decodeFps / expectedContentFps`

首版达标线建议：

- `stable`：`>= 0.8`
- `warning`：`0.6 - 0.8`
- `congested`：`< 0.6`

`expectedContentFps` 来源优先级：

1. 远端显式目标值
2. 近窗稳定输出上沿估计
3. 默认 `60`

## Policy Sketch

### `homeLan + steady + content30`

- warmup `1500-2500ms`
- 初始 `L1`，更早尝试回 `L0`
- `presentFps >= 24` 且 `decodeFps >= 24` 允许视作稳定
- 码率线更关注相对最近基线，弱化固定 `baseBitrate * x`

### `homeRelay + steady`

- warmup `3000-5000ms`
- 中性 `L1 -> L0` 恢复
- 保留对 `feedbackInterval / loss / packetAge` 的敏感性

### `cloud + startup`

- warmup `6000-8000ms`
- 保留保守 `L1`
- `highRtt` 和 `startup` 可叠加成更谨慎的策略集

## Trace And Diagnostics Contract

需要在前端 snapshot / trace / diagnostics 新增以下观测字段：

- `frontEndProfileBaseline`
- `frontEndProfileDynamic`
- `frontEndContentFpsClass`
- `frontEndExpectedContentFps`
- `frontEndPolicyPreset`
- `frontEndWarmupUntilMs`
- `frontEndUpshiftBlockedReason`

新增后应能直接回答：

- 当前为什么仍在 `L1`
- 这是网络画像、内容画像、显示画像还是启动画像导致
- 如果解除阻塞，下一档会升到哪里

## Plan

1. 定义 `RuntimeProfileClassification` 与 `ProfilePolicyPreset` 合同，确定字段命名与 trace 面。
2. 在 `browser-runtime` 中补齐前端画像分类器，先基于当前可得输入落地 `baseline/dynamic/contentFpsClass`。
3. 将 `warmup`、`bandwidthState`、`quality ladder`、`display degrade` 改成按 preset 驱动。
4. 将绝对 `fps` 门槛替换为相对内容帧率门槛，并提供 `expectedContentFps` 推导策略。
5. 补齐 diagnostics / trace / 测试，验证 `home-lan` 与 `30fps` 内容不再被统一策略误压。

## Validation

- [x] `browser-runtime` 新增画像分类测试，覆盖 `homeLan / homeRelay / cloud`（`browser-runtime-profile.test.ts`）
- [x] `browser-runtime` 新增内容帧率分类测试，覆盖 `30fps / 60fps / unknown`
- [x] `browser-runtime` 新增画像策略表测试，覆盖 warmup、相对带宽门槛、cloud/home 保守性对照
- [ ] `home-lan + 30fps` 实机会话在稳定输出后可升回 `L0`（需设备侧确认）
- [x] `cloud + startup` 策略 warmup 长于 `homeLan + startup`（单测断言）；`highRtt` 动态由 stats / `remoteProfile*` 提示驱动
- [x] diagnostics / snapshot / trace payload 可携带前端画像与 `frontEndPolicyPreset`

## Risks

- 如果前端命名合同和 Rust 侧画像长期分叉，后续调试会重新变得混乱。
- 如果 `expectedContentFps` 推导不稳，可能把临时波动误当成内容上限，造成误升档。
- 如果策略表切得过细而没有统一配置入口，维护成本会再次上升。

## Progress

- [x] Step 1: 已完成问题收敛，确认方向为“前端轻量画像分类器 + 画像策略表”
- [x] Step 2: 前端画像字段、策略表与 trace 合同已落地（见 `browser-runtime-profile.ts` 与 `StreamPerformanceSnapshot.frontEnd*`）
- [x] Step 3: 已实现 `browser-runtime` 画像驱动调度接入
- [x] Step 4: 已补单测与 diagnostics 透传；实机 `home-lan + 30fps` 回归见 Report

## Execution Notes

- Date: 2026-05-11 | Status: implemented
- Update: 基于 `webrtc-direct` 当前实现与 Rust 侧现有画像主线，整理出前端画像驱动调度的 RFC 初稿。
- Decision: 采用方案 B，在浏览器直连前端补一层轻量画像分类器，并让现有 `warmup / quality ladder / display degrade / bandwidth gate` 改为按画像策略表驱动。
- Decision: 首轮不复刻 Rust 全套恢复状态机，保持前端分类器轻量，重点解决 `home-lan` 与 `30fps` 内容被统一门槛误伤的问题。
- Risk/Blocker: `expectedContentFps` 的数据来源与稳定性仍需在实现前进一步确认，否则相对帧率门槛可能出现误判。
