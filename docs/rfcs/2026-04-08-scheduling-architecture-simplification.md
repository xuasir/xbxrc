# 调度层架构简化与能力保留 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 最近多轮调度层改动已经不只是“规则补丁”或“文件变厚”，而是在事实上把原先的单轴恢复模型扩展成了多事实平面驱动的分层仲裁模型。
- 当前实现已经稳定承接了几类必须保留的增强能力：
  - 首帧获取优先：连接后首帧慢、无 SPS、无 IDR、云端反馈慢等场景下优先尽快拿到可用首帧，而不是过早升级或陷入恢复循环。
  - 昂贵恢复升级面收缩：本地显示/入口缺口不能被误洗成 reconnect 或 failed-terminal。
  - 恢复成功后的爬升期保护：拿到恢复帧、clean anchor 或恢复输出后，不立即重入恢复循环。
- 这些能力本身是正确且必要的，但当前架构把过多“中间解释层”提升成了一等概念，例如：
  - `owner_state`
  - `RecoveryLivenessState`
  - transport-await lane
  - TWCC warmup / source phase
  - host cadence phase
  - 多类 `reason_label` 与 summary 字段
- 对照 [`moonlight-qt`](/Users/guo.xu/Documents/code/games/moonlight-qt) 的设计可以看到：其内部状态并不简单，但顶层控制概念极少，主线更接近“事实 -> 恢复动作 -> 执行结果”。当前 `xbxrc` 的问题不是功能太多，而是顶层概念太多。

## Goal

- 在**不删减现有有效能力**的前提下，重构调度层架构，把顶层模型收敛为少数稳定概念。
- 将当前“多解释层并行存在”的架构收口为统一主线：
  - Facts
  - Domain
  - Local Recovery
  - Expensive Recovery Gate
  - Planner / Command
- 明确并固定三类必须保留的增强能力归属：
  - `FirstFrameAcquisitionPriority`
  - `ExpensiveRecoveryGate`
  - `RecoveryRampGuard`
- 让 owner / coordinator / session policy 恢复单一权力边界，避免继续出现三层都拥有部分恢复主权。

## Scope

- In scope:
  - [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs)
  - [`crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs)
  - [`crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs)
  - [`crates/xbxengine/core/src/transport/rtc/recovery/runtime_state.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/runtime_state.rs)
  - [`crates/xbxengine/core/src/transport/rtc/policy/scheduling.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/scheduling.rs)
  - [`crates/xbxengine/core/src/transport/rtc/connection/twcc_feedback.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/connection/twcc_feedback.rs)
  - [`crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.rs)
  - [`crates/xbxengine/core/src/diagnostics/stats.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/diagnostics/stats.rs)
  - [`src-tauri/src/mods/xbxengine/trace_projection.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/xbxengine/trace_projection.rs)
  - recovery / scheduling / trace 相关回归测试与 `docs/project-task.md`
- Out of scope:
  - 删除现有首帧获取优先、昂贵恢复收缩、恢复爬升保护能力
  - 回退 host cadence / display pressure / local-feedback 等已验证有效的事实面
  - 引入新的客户端栈、旁路恢复链或第二套控制面
  - 单独调参作为本 RFC 的主要目标；阈值调整仅作为重构后行为对齐的必要补充

## Target Architecture

### 1. 顶层一等概念

- `SessionState`
  - `Starting`
  - `Connected`
  - `Recovering`
  - `Reconnecting`
  - `FailedTerminal`
- `Facts`
  - `TransportFacts`
  - `MediaFacts`
  - `DisplayFacts`
  - `FeedbackOwnershipFacts`
- `Domain`
  - `LocalDisplay`
  - `LocalIngress`
  - `MediaRecoveryBridge`
  - `ConnectivityTransport`
- `RecoveryDecision`
  - `Absorb`
  - `LocalRecover`
  - `TransportRecover`
  - `FailedTerminal`

### 2. 固定控制主线

```text
Facts
→ FirstFrameAcquisitionPriority
→ DomainClassifier
→ LocalRecoveryCoordinator
→ ExpensiveRecoveryGate
→ RecoveryRampGuard
→ Planner / Executor
→ Reporting
```

### 3. 三个必须保留的专门契约

- `FirstFrameAcquisitionPriority`
  - 负责首帧慢、无 SPS、无 IDR、云端 feedback readiness 等场景下优先尽快拿到可用首帧。
  - 只决定：当前是否仍处于首帧获取优先窗口、是否优先只允许关键帧获取/局部恢复、是否允许进入昂贵恢复。
- `ExpensiveRecoveryGate`
  - 负责 reconnect / failed-terminal 唯一审批。
  - 只消费域别与昂贵恢复上下文，不再回头解释局部媒体事实。
- `RecoveryRampGuard`
  - 负责恢复成功后的稳定窗口保护。
  - 只决定：是否抑制快速重入、是否只允许吸收/局部恢复、何时回归正常仲裁。

### 4. 顶层必须裁撤或降级的概念

| 当前概念 | 处理方式 | 说明 |
| --- | --- | --- |
| `VideoSchedulingOwnerState` 细粒度状态 | 降级为 `MediaFacts` 内部状态 | 顶层不再直接依赖 `SeekingAnchor/Priming/RebuildingSupply/...` |
| `RecoveryLivenessState` | 降级为 reporting | 允许继续投影到 trace/UI，但不再主导控制面 |
| transport-await lane | 降级为 `LocalRecoveryCoordinator` 内部子状态机 | 保留能力，不保留顶层存在感 |
| TWCC warmup/source phase | 降级为 `FeedbackOwnershipFacts` 内部状态 | 先投影成 `FeedbackReadiness` 再参与 gate |
| `reason_label` 字符串 | 删除一等控制地位 | 只保留内部枚举与边界投影 |
| `video_owner_source/latest_decision_summary/runtime_summary` | 降级为 reporting | 禁止回流控制面 |

### 5. 新模块权力边界

- `FactCollector`
  - 只做事实提取与标准化。
- `DomainClassifier`
  - 只回答“当前主问题属于哪个域”。
- `LocalRecoveryCoordinator`
  - 只做局部恢复动作选择：`Wait / RequestKeyframe / RequestDecoderReset`。
- `ExpensiveRecoveryGate`
  - 只做 reconnect / failed-terminal 审批。
- `Planner`
  - 只做命令优先级与映射。
- `Reporting`
  - 只做 runtime stats / trace / diagnostics / frontend 投影。

## Capability Preservation Contract

### A. 首帧获取优先能力必须保留

- 保留“连接后首帧慢但仍合理”时的首帧获取优先窗口。
- 保留“无 SPS / 无 IDR / bootstrap 未就绪”时快速请求关键帧并停留在局部恢复链的能力。
- 保留 cloud / home / Moonlight 等不同远端画像下不同的首帧容忍度。
- 禁止因为简化架构而把首帧阶段重新收敛成单一粗暴 timeout 或提前升级到昂贵恢复。

### B. 昂贵恢复升级面必须继续收缩

- `LocalDisplay`、`LocalIngress` 不得直接落成 reconnect。
- `MediaRecoveryBridge` 只有在局部恢复链失败且具备强 transport 证据时才允许升级。
- reconnect / failed-terminal 只能由 `ExpensiveRecoveryGate` 统一批准。

### C. 恢复成功后的爬升期保护必须保留

- `clean anchor + chain healthy + decode/present progress` 成立后，保留短窗重入抑制。
- 短抖动、短 no-pending、短 present gap 不得立即重新打回 transport 主恢复链。
- 恢复爬升保护窗结束后，再回到正常仲裁。

## Plan

1. 先定义并落地新的顶层合同与概念边界。
2. 重构事实提取链，把 startup / feedback / display / media / transport 统一投影为 Facts。
3. 将 owner 收窄为域别与局部恢复表面判定，不再直接承担完整恢复叙事。
4. 将 coordinator 收窄为局部恢复动作选择器，并把 lane 收口为内部子状态机。
5. 将 session policy 收窄为昂贵恢复 gate + planner orchestration，不再回头解释局部事实。
6. 把 reporting 字段与控制字段彻底分层，避免 summary/source/state 再次回流控制面。
7. 补齐“首帧获取优先 / 昂贵恢复升级面 / 爬升期保护”三大合同的回归测试，再补 trace/frontend 对齐。

## Validation

- [x] `session/policy`、`video_scheduling_owner`、`recovery/coordinator` 的职责边界完成重构，且不再出现三层并列解释权
- [x] 首帧慢、无 SPS / 无 IDR 的现有行为保持：相关集成用例与 trace 回归通过
- [x] `displaySupplyCritical/local ingress` 不升级 reconnect 的域别合同保持稳定
- [x] clean anchor / 恢复帧后的爬升期保护保持稳定，不重新形成恢复循环
- [x] `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::policy::video_scheduling_owner -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::stream::video_source -- --nocapture`
- [x] `cargo test -p xbxengine runtime_stats_sink -- --nocapture`
- [x] `cargo test --manifest-path src-tauri/Cargo.toml trace_projection -- --nocapture`
- [x] `cargo check -p xbxengine`
- [x] `cargo check -p xbxrc`

## Risks

- 如果只是“改文件组织”而不真正删减顶层概念，复杂度会以新名字回流。
- 如果把 lane、warmup、summary 等概念直接硬删而不先变成内部 facts/子状态机，会误伤现有兼容行为。
- 如果 reporting 字段在重构中仍继续被控制面读取，架构回流会再次发生。
- “一口气改造到位”范围较大，必须以契约回归测试兜住行为，不可只依赖人工 trace 观察。

## Progress

- [x] Step 1: 完成架构方向回顾，对照 `moonlight-qt` 与当前 `xbxrc`，确认问题在于顶层概念膨胀而非功能过多
- [x] Step 2: 明确三类必须保留能力，并将其收敛为 `FirstFrameAcquisitionPriority / ExpensiveRecoveryGate / RecoveryRampGuard`
- [x] Step 3: 完成顶层合同落地与事实提取链重构
- [x] Step 4: 完成 owner / coordinator / session policy 权力边界收口
- [x] Step 5: 完成 reporting/control 分层与回归验证

## Execution Notes

- Date: 2026-04-08 | Status: planned
- Update: 基于当前仓库与 `moonlight-qt` 的对照分析，确认本轮任务目标不是删除能力，而是减少顶层概念数量，收敛为“Facts -> Domain -> Decision -> Command”主线。
- Decision: 明确保留三类增强能力：首帧获取优先、昂贵恢复升级面收缩、恢复成功后的爬升期保护；后续重构不得以简化名义回退这些行为。
- Decision: 顶层简化不采用“继续加 phase/lane/source 解释层”的方式，而是固定为 `FirstFrameAcquisitionPriority / DomainClassifier / LocalRecoveryCoordinator / ExpensiveRecoveryGate / RecoveryRampGuard` 五段控制主线。
- Risk/Blocker: 当前 owner/coordinator/session policy 三层都有部分恢复解释权，属于本轮改造的核心风险点；若中途保留双轨旧逻辑，将很难证明架构已经真正收口。
- Date: 2026-04-08 | Status: in progress
- Update: 第一阶段已完成 `facts / startup_compat / expensive_recovery_gate` 的落地；本轮继续把恢复爬升期保护从 [`session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 抽到独立 `RecoveryRampGuard`，让 session policy 只保留 orchestrator 角色。
- Validation Note: `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture` 已通过；`transport::rtc::recovery::coordinator` 发现 1 条旧测试名仍绑定“等待阶段”叙事，但当前实现实际仍停留在本地恢复链且允许推进到 decoder reset stage，已按现行合同修正回归断言。
- Date: 2026-04-08 | Status: in progress
- Update: 继续收窄 `VideoSchedulingOwner` 对外契约，已把 `reason_label / reason_source / temporary_diagnostic_summary` 从控制输出中收进独立 diagnostics surface；控制面现在显式只依赖 `state / health / recovery_intent`，诊断字段只用于 runtime stats 与临时 trace 打点。
- Validation Note: `cargo test -p xbxengine transport::rtc::policy::video_scheduling_owner -- --nocapture` 与 `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture` 已通过，说明此次 owner contract 收权未改变行为。
- Date: 2026-04-08 | Status: in progress
- Update: 继续完成第二阶段三项收口：1) `owner` 为 recovery intent 引入结构化 canonical reason，字符串 reason label 退到 reporting 边界；2) `session policy` 构建 recovery signal 时改为直接消费结构化 intent reason，不再依赖 owner 字符串映射控制面；3) `coordinator` 将对外暴露的 transport-await `probe/decode/reset` 三个离散布尔判断收口为单一 `transport_await_recovery_stage` 查询，外层只消费“当前处于哪一段本地恢复阶段”。
- Validation Note: `cargo test -p xbxengine transport::rtc::policy::video_scheduling_owner -- --nocapture`、`cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`、`cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture` 已通过；说明本轮结构化收权保持了首帧获取优先、局部恢复升级面收缩与恢复爬升保护合同。
- Date: 2026-04-08 | Status: completed
- Update: 收尾对齐了 `video_source` 侧最后两条回归合同：`clean anchor` 仅记录当前 recovery epoch 的 clean anchor，不再直接关闭 transport recovery episode；`expired deadline -> recovered` 的稳定性用例改为把恢复锚点时刻与检查点对齐，避免伪造“clean anchor 后 800ms 无 decode/present 进展”的假恢复场景。
- Decision: 当前架构的恢复爬升保护合同固定为“clean anchor + current recovery epoch + fresh media output + short hold window”组合条件；因此恢复保护不再由字符串 reason 或测试假设驱动，而由 `RecoveryRampGuard` 统一裁定。
- Validation Note: `cargo test -p xbxengine transport::rtc::stream::video_source -- --nocapture` 与 `cargo check -p xbxrc` 已通过，至此 RFC 列出的全部验证完成。
- Date: 2026-04-08 | Status: completed
- Update: 参考 `moonlight-qt` 对“码流问题 / decoder 问题 / 本地恢复 / 昂贵恢复”的动作边界，对 [`recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 与 [`session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 做了最后一轮收口：`TransportAwaitRecoveryKeyframe` 即使进入 hard fallback 也只允许停留在 `RequestDecoderReset` 本地恢复链，不再升级成 reconnect；`session policy` 进一步把 `RequestReconnectCandidate` 收敛为连接域 reason 白名单，媒体/decoder/显示供应等本地域只能继续留在 `Wait / RequestKeyframe / RequestDecoderReset`。
- Decision: 调度层的最终动作边界固定为“下层只能申请本地动作，上层才有权批准昂贵动作，连接失败必须由连接证据证明，不能由解码失败或码流坏窗猜出来”。
- Validation Note: `cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture` 与 `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture` 已通过，说明全链路边界收口没有回退现有恢复合同。
- Date: 2026-04-08 | Status: completed
- Update: 对照 `moonlight-qt` 的“小波动先吸收、持续失败再 reset”边界，继续在 [`stream/video_source/source.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs) 与 [`recovery/escalation.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs) 做防风暴收口：`sample loss` 需更长的连续坏窗才会从 `DropAndRequestKeyframe` 升到 `WaitKeyframe`；`AdapterThinStream / AdapterIdleTimeout / TransportSampleLoss` 不再因为 keyframe 后短窗重复出现而直接升级到 `RequestDecoderReset`，decoder reset 只保留给 `WaitKeyframe / TransportAwaitRecoveryKeyframe / Reconfigure / DecoderBackendFailure` 这类更硬的持续失败证据。
- Decision: 小波动抖动的默认处理边界固定为“等待 / 丢帧 / 请求关键帧”，而不是“重复轻信号 -> keyframe -> decoder reset”的短链升级；本地 decoder reset 现在只由持续性局部失败证据驱动。
- Validation Note: `cargo fmt --all`、`cargo test -p xbxengine transport::rtc::recovery::escalation -- --nocapture`、`cargo test -p xbxengine transport::rtc::stream::video_source::source -- --nocapture` 已通过，说明防风暴收口未回退现有恢复主链。
- Date: 2026-04-08 | Status: completed
- Update: 继续沿 `moonlight-qt` 的“先等待一个短窗口再决定是否上报”思路，在 [`stream/video_source/source.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs) 为 `idle timeout / thin stream stall` 新增入口确认窗：首次命中只启动短暂 confirmation window，只有短窗后仍持续满足条件才真正发出 timeout/thin-stream observation；一旦收到新包或样本完成，待确认状态立即清空。
- Decision: `idle timeout / thin stream` 现在不再是“单点命中即上报”的边界，而是“短确认窗内持续存在才上报”；这层吸收逻辑属于 source 本地历史窗口，不上抬到上层恢复控制面。
- Validation Note: `cargo fmt --all` 与 `cargo test -p xbxengine transport::rtc::stream::video_source::source -- --nocapture` 已通过，新增确认窗回归也已覆盖。
- Date: 2026-04-08 | Status: completed
- Update: 基于真实 trace 中“decoder 仍在 waiting-keyframe，但 source/timeline 已把 delta continuation 当成可继续链”的错位，本轮继续收紧 [`stream/video_source/source.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs) 的首帧边界：`resolve_inspection_admission()` 现在显式绑定 `first_frame_acquired`，pre-first-frame 阶段即便 `committed SPS/PPS + deltaContinuationReady` 也不会放行 non-IDR continuation；`resolve_recovery_keyframe_action()` 在首帧未建立且仍处于 waiting-keyframe 时不再吸收 non-keyframe delta，必须继续停留在本地 `WaitKeyframe`。
- Decision: “首帧获取优先”不再只是命名变化，而是正式成为 source admission / local recovery 的动作边界：首帧未建立前，delta continuation 只能作为观测事实，不能被误升成链路恢复成功或 soft reentry 依据。
- Update: 同时补强 keyframe episode 观测链而不继续膨胀 DTO：[`runtime_stats_sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/runtime_stats_sink.rs) 新增 `response-observed` 中间状态，用现有 `first_video_packet* / first_keyframe_packet* / status_detail` 直接记录“首个响应是 non-keyframe”或“首个 keyframe 仍因 bootstrapMissingSps/InvalidSliceHeader 等不可用”；[`trace_projection.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/xbxengine/trace_projection.rs) 同步投影 `keyframeRequestEpisodeResponseObserved` 事件，便于直接在 trace 中定位“请求已发出，但首个响应为何不可用”。
- Validation Note: `cargo fmt --all`、`cargo test -p xbxengine transport::rtc::stream::video_source::source -- --nocapture`、`cargo test -p xbxengine runtime_stats_sink -- --nocapture`、`cargo test -p xbxrc trace_projection -- --nocapture` 已通过。
