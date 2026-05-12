# 串流面板信息架构拆分 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成（首批落地）
- Current State: implemented
- Owner: Codex
- Last Updated: 2026-05-12

## Background

- 当前串流页同时存在 [`StreamDiagnosticsPanel.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/components/stream/StreamDiagnosticsPanel.vue) 与 [`StreamPerformancePanel.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/components/stream/StreamPerformancePanel.vue) 两块浮层面板，[`Stream.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/Stream.vue:719) 直接把两者都挂到播放层上。
- 两个面板已经持续吸收同一批 runtime 字段：
  - 用户体验指标
  - 恢复状态
  - renderer / SR 状态
  - 控制通道状态
  - decoder 恢复事件
  - RFC/owner/decision 内部诊断语义
- 当前实现通过统一 `StreamSessionDiagnosticsSnapshot + StreamPerformanceSnapshot` 合同，把 browser 与 Rust 两条 runtime 线路的字段混在一起，再在组件里按 `runtimeMode` 做条件分支。[`useStreamExecution.ts`](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/useStreamExecution.ts:217) [`types.ts`](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/types.ts:68)
- 结果已经出现三类问题：
  - 面板冗余：状态、传输、带宽、恢复、超分等字段在两边重复展示
  - 用户噪音：大量工程术语直接暴露给用户
  - 合同漂移：browser 专属 renderer/画像字段和 Rust 专属 host/owner/recovery 字段继续塞进统一面板

## Goal

- 将串流页用户可见的指标收敛为一个共享“体验指标”面板，browser 与 Rust 使用同版结构与同版文案。
- 将恢复链、渲染实现、控制通道、decoder 恢复、RFC/owner/fault 这类内部诊断从用户面板中移出。
- 为 browser 与 Rust 分别建立独立的内部诊断面板与独立 view model，保证两条 runtime 的实现语义各自表达。
- 收窄页面层与组件层的职责，使串流页展示结构从“统一大快照 + runtimeMode 分支”改成“共享体验层 + runtime 专属诊断层”。
- 重构串流菜单入口，将用户功能与会话控制保留在同一个主菜单，将诊断与实验能力合并到独立入口。

## Scope

- In scope:
  - [`src/pages/Stream.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/Stream.vue)
  - [`src/components/stream/StreamDiagnosticsPanel.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/components/stream/StreamDiagnosticsPanel.vue)
  - [`src/components/stream/StreamPerformancePanel.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/components/stream/StreamPerformancePanel.vue)
  - [`src/streaming/useStreamExecution.ts`](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/useStreamExecution.ts)
  - [`src/streaming/diagnostics.ts`](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/diagnostics.ts)
  - [`src/streaming/types.ts`](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/types.ts)
  - 串流页相关 i18n 文案与菜单入口
- Out of scope:
  - 改动 session 启动编排主线
  - 改动 browser runtime 与 Rust runtime 的底层传输、恢复、renderer 实现
  - 改动 `RuntimeLaunchSpec`、session RPC、runtime host 创建与 attach 主线
  - 新增第二套前端框架或平行页面路线

## Current Problem Breakdown

### 1. 用户面板与工程诊断混层

- 诊断面板当前同时展示 `region/server/path/status` 与 `recoveryBudget/controlChannelError/decisionDigest/rfcFaultDomain` 这类内部字段。[`StreamDiagnosticsPanel.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/components/stream/StreamDiagnosticsPanel.vue:129)
- 性能面板当前同时展示 `RTT/JIT/FPS/bitrate` 与 `RecoLv/RecoRs/RecoEff/DecEvt` 这类恢复算法和 decoder 事件。[`StreamPerformancePanel.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/components/stream/StreamPerformancePanel.vue:120)

### 2. 两个面板重复消费同一组语义

- `status/transport/SR/bandwidth/controlChannel/recoveryCause/networkConfidence/decodeConfidence` 在两个面板中重复展示。
- `resolveStatusText` 与 SR 运行态格式化逻辑在两个组件中重复维护，并已开始出现行为分叉。[`StreamDiagnosticsPanel.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/components/stream/StreamDiagnosticsPanel.vue:86) [`StreamPerformancePanel.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/components/stream/StreamPerformancePanel.vue:86)

### 3. 统一快照合同继续放大 runtime 差异

- browser runtime 侧已经引入 `frontEndProfile* / renderProcessing / renderPipelineType / SR*` 等前端实现字段。
- Rust runtime 侧已经承载 `ownerState / ownerReason / recoveryRfc* / hostMailbox* / hostFramePresentEpoch` 等引擎侧字段。
- 当前这些字段继续共同挂在统一快照类型下，组件只能通过 `runtimeMode` 做大块条件渲染，长期可维护性会持续下降。

## Proposed Direction

### A. 用户层只保留一个共享体验指标面板

新增共享体验层面板，暂定命名为 [`StreamExperiencePanel.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/components/stream/StreamExperiencePanel.vue)。

这层只表达“当前体验如何”，不表达“系统为什么这么决策”。

建议首版固定字段为：

1. 状态
2. 分辨率
3. RTT
4. JIT
5. 接收 FPS
6. 解码 FPS
7. 呈现 FPS
8. 丢包
9. 视频下行
10. 总下行
11. 连接耗时
12. 首帧耗时

可附带保留的轻量 notice：

- 当前使用中继
- 当前处于恢复中
- 当前无画面告警

这层不展示下列内容：

- `ownerState / ownerReason`
- `recoveryEpoch / recoveryBudget / recoverySuppressedBy`
- `decisionDigest / actionEffectScore / actionEffectReason`
- `controlChannelError`
- `decoder recovery event`
- `RFC fault / stage / ceiling`
- renderer 具体实现细节

### B. 内部诊断层拆成 browser / rust 两版

新增两个内部诊断组件：

- [`StreamBrowserDiagnosticsPanel.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/components/stream/StreamBrowserDiagnosticsPanel.vue)
- [`StreamRustDiagnosticsPanel.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/components/stream/StreamRustDiagnosticsPanel.vue)

#### Browser 内部诊断面板承载

- transportState
- presentationMilestone
- renderPipelineType
- renderProcessing / renderShaderPath
- frontEndProfileBaseline / Dynamic / PolicyPreset
- super resolution 运行态
- bandwidthState / bandwidthAction
- controlChannelState / error / open ratio / buffered trend
- keyframe success
- recoveryCause / qualityLadderLevel / actionEffect / decisionDigest

#### Rust 内部诊断面板承载

- transportState
- videoHealth / primaryIssueChain / latestDecision
- ownerState / ownerReason
- decoderState / decoderEvent / reset count
- stallKind
- recoveryRfcFaultDomain / Stage / Ceiling / Diagnosis
- hostMailbox / hostPresent / submit / display age 相关遥测

### C. 数据层拆成共享体验 view model 与 runtime 专属诊断 view model

建议新增三组页面消费模型：

- `StreamExperienceMetricsViewModel`
- `StreamBrowserDiagnosticsViewModel`
- `StreamRustDiagnosticsViewModel`

其中：

- `StreamExperienceMetricsViewModel` 只依赖两条 runtime 都稳定具备的 core 指标
- `StreamBrowserDiagnosticsViewModel` 只投影 `webrtc-direct` 真实存在且有意义的字段
- `StreamRustDiagnosticsViewModel` 只投影 `rust-owned` 真实存在且有意义的字段

### D. 收窄共享合同边界

保留以下共享层：

- session 启动与 progress 编排
- `RuntimeLaunchSpec`
- runtime host 创建与连接
- 体验层所需 core metrics

收窄以下共享层：

- `diagnostics.ts` 从“大一统展示快照”改成“共享体验投影 + runtime 专属诊断投影”
- `types.ts` 中与展示层强绑定的大而全接口，逐步退成核心字段 + 专属扩展字段

### E. 菜单入口重构

当前 [`Stream.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/Stream.vue:216) 的串流菜单将用户操作、会话控制、诊断入口、实验功能混在同一个 action sheet 中，后续面板重构完成后，这种入口结构会继续放大认知噪音。

本 RFC 将菜单入口固定为两类：

#### 1. 主菜单

主菜单同时承载用户常用操作与会话控制操作。

建议顺序：

1. Xbox 键
2. 长按 Xbox 键
3. 显示设置
4. 音频
5. 麦克风
6. 发送文字
7. 全屏
8. 关机并退出
9. 退出串流

其中 `Xbox 键 / 长按 Xbox 键` 前置，作为高频会话控制能力直接暴露。

#### 2. 诊断入口

诊断入口同时承载诊断能力与实验能力。

建议承载：

- 体验指标
- Browser 内部诊断
- Rust 内部诊断
- 超分开关
- 导出超分对比

命中条件由 runtime 能力和实验开关决定，不满足条件的项不展示。

#### 3. 结构约束

- 主菜单与诊断入口分离，不再把所有操作塞进同一个 action sheet。
- 主菜单不承载内部诊断字段与实验验证动作。
- 诊断入口不承载退出、音频、文字发送等用户流程操作。
- 旧 `diagnostics / performance` 菜单项在新结构落地后退场，由“体验指标”与“内部诊断”命名替换。

## Data Model Sketch

```ts
interface StreamExperienceMetricsViewModel {
  status: string
  resolution: string
  rtt: string
  jit: string
  recvFps: string
  decodeFps: string
  presentFps: string
  packetLoss: string
  videoBitrate: string
  totalBitrate: string
  connectedElapsed?: string
  mediaReadyElapsed?: string
  relayNotice: boolean
  recoveringNotice: boolean
  noVideoNotice: boolean
}

interface StreamBrowserDiagnosticsViewModel {
  transportState: string
  presentationMilestone: string
  renderPipelineType: string
  renderProcessing?: string
  renderShaderPath?: string
  frontEndProfileBaseline?: string
  frontEndProfileDynamic?: string
  frontEndPolicyPreset?: string
  srSetting: string
  srRuntime: string
  bandwidthState?: string
  bandwidthAction?: string
  controlChannelState?: string
  controlChannelError?: string
  keyframeSuccessRate?: string
  recoveryCause?: string
  qualityLadderLevel?: string
  decisionDigest?: string
}

interface StreamRustDiagnosticsViewModel {
  transportState: string
  videoHealth: string
  primaryIssueChain: string
  latestDecision: string
  ownerState: string
  ownerReason: string
  decoderState: string
  decoderEvent?: string
  stallKind: string
  recoveryDiagnosis: string
  recoveryRfcFaultDomain: string
  recoveryRfcStage: string
  recoveryRfcCeiling: string
  hostPresentTelemetry?: string
}
```

## Plan

1. 定义共享体验指标字段清单与 browser/rust 内部诊断字段归属表。
2. 新增共享体验 view model 与 browser/rust 专属诊断 view model，停止让页面组件直接消费混杂大快照。
3. 重构串流入口结构，拆出“主菜单”和“诊断入口”，并调整主菜单内的会话控制排序。
4. 落地 `StreamExperiencePanel`、`StreamBrowserDiagnosticsPanel`、`StreamRustDiagnosticsPanel`，替换旧的双面板结构。
5. 清理旧组件中的重复状态解析、SR 文案拼装、旧菜单项与 runtimeMode 条件分支。
6. 收口 i18n、菜单入口、增强挂载与页面层可见性逻辑。

## Validation

- [x] 共享体验面板在 `webrtc-direct` 与 `rust-owned` 下显示同版字段、同版布局、同版文案（`buildStreamExperienceMetricsViewModel` + 单测对齐）
- [x] browser 内部诊断面板只展示 browser 专属字段，不再混入 Rust 术语（`v-if="runtimeMode === 'webrtc-direct'"` + `StreamBrowserDiagnosticsViewModel`）
- [x] Rust 内部诊断面板只展示 Rust 专属字段，不再混入 browser renderer / frontEndProfile 字段（`v-if="runtimeMode === 'rust-owned'"` + `StreamRustDiagnosticsViewModel`）
- [x] 主菜单只保留用户操作与会话控制，`Xbox 键 / 长按 Xbox 键` 顺序前置
- [x] 诊断入口单独承载体验指标、内部诊断与实验功能（`StreamActionSheet` `stream.diagnostics-menu-sheet`）
- [x] 旧 `StreamDiagnosticsPanel` / `StreamPerformancePanel` 中的重复状态解析逻辑已收敛至 `stream-panel-formatters.ts` / `stream-panel-view-models.ts`
- [x] `pnpm lint:fix`
- [x] 对应前端单测：`src/streaming/stream-panel-view-models.test.ts` 与既有 `diagnostics.test.ts` 通过

## Risks

- 如果体验指标字段选得过窄，用户层面板会失去排障价值。
- 如果体验指标字段选得过宽，内部诊断会重新回流到用户层。
- 如果新 view model 只是把旧大快照换一层薄包装，类型漂移问题会保留。
- 如果菜单只改顺序不改入口边界，用户操作、诊断和实验能力仍会继续互相污染。
- 菜单入口与增强挂载若不一起收口，页面层复杂度仍会留在 [`Stream.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/Stream.vue)。

## Progress

- [x] Step 1: 已完成现状评估，确认“共享体验指标 + browser/rust 分离内部诊断”方向
- [x] Step 2: 设计并实现新的 view model 分层（`stream-panel-view-models.ts`、`stream-panel-formatters.ts`）
- [x] Step 3: 落地新的主菜单与诊断入口结构（`Stream.vue` + `xstream-page-ui.ts`）
- [x] Step 4: 落地新面板组件并迁移旧字段；移除 `StreamDiagnosticsPanel` / `StreamPerformancePanel`
- [ ] Step 5: 收窄 `StreamSessionDiagnosticsSnapshot` 与 `buildStreamDiagnosticsSnapshot` 输出字段（`StreamBadgeRow` / `StreamDiagnosticNoticeBar` 仍依赖全量快照，后续单独 PR）

## Execution Notes

- Date: 2026-05-12 | Status: implemented
- Update: 已落地体验 / browser / rust 三面板、`StreamEnhancementId` 扩展、runtime-host 统计轮询 OR 门、主菜单与「诊断与实验」子菜单拆分；`StreamSessionDiagnosticsSnapshot` 类型与 builder 的字段级收窄未在本次完成。
- Decision: 共享层保留会话编排与 core experience metrics；renderer、恢复链、control channel、decoder、owner/RFC fault 归入 runtime 专属诊断层；主菜单承载用户功能与会话控制，诊断入口承载诊断与实验功能。
- Risk/Blocker: 体验指标字段边界需要在实现前定死，避免迁移过程中再次把工程字段带回用户层。
