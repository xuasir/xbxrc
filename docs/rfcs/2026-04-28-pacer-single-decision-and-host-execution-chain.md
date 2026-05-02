# Pacer 单点决策与 Host 执行链收权 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: in-progress
- Owner: Codex
- Last Updated: 2026-04-28
- 本轮已完成：
  - Windows/macOS presenter 统一到 `host-present mailbox` 执行模型
  - `native_video` host trace 事件名统一到 mailbox / host-present 语义
  - `runtime_port -> runtime stats -> diagnostics/protocol/frontend snapshot/trace_projection` 的 host-present 指标字段统一切到新命名，删除旧字段双写
  - `pacer -> render` 收成“单点决策 + 单槽 handoff”：`render` 已删除价值拒绝分支，只保留 latest-slot overwrite 与恢复 telemetry
  - `display_supply / session facts / render pacing` 的局部 `present_* / queue pressure` 命名已统一到 `host_mailbox_* / host_frame_present_* / mailbox pressure`
  - `native_video viewport snapshot / runtime_state / trace_projection observation state` 内部字段名已同步切到 `host_mailbox_* / host_frame_present_*`，host mailbox 统计与真实上屏事实完成语义分层
  - `hostMailboxState / hostFramePresentResumed` 已替换旧 `hostPresent*` trace/test 命名，观测面与实现语义保持一致
  - `pacer` 本地 `queue_history / queue_pressure / recovery_budget_active` 历史支路已移除，release gate 与 mailbox handoff 成为 decode 后短路径
  - `pacer` 内已退出失效的 host release wait 残留，host cadence 只回传节奏事实，release 节拍由 `pacer` 单点驱动
  - recovery / transport 侧只保留正式请求结果语义，PLI/FIR 测试已直接走 `_with_outcome` 正式入口
  - decoder-reset control replay 旧入口已收为测试专用，运行时只保留 `transport_session` 本地 reset 主路径
  - `rtc-connection pump` 高频心跳日志已降到 debug，默认日志面只保留状态跃迁和候选对切换等有效事件

## Background

- 近期 recovery trace 持续暴露同一类尾段失稳：
  - `PLI` 已经可发，`videoRtcpFeedbackTargetPending` 已脱离主阻塞位；
  - `IDR -> decode -> clean-anchor submitted` 已发生；
  - `CleanAnchorCommitted` 长期不上账，`DisplayStable` 也不上账；
  - host 最终上屏的是后续普通 continuation，而不是当前 recovery owner 帧。
- 最新 trace 的核心症状已经稳定：
  - `keyframe_effectiveness` 中 `effective=0`、`chain_recovered=0`；
  - `first_frame_latency_observed` 中大量终点停在 `Decoded` 和 `DisplayStable` 前的 `noCleanAnchorCommit`；
  - `picture_recovery_blocker_observed` 中 `continuationAcceptedWhileAwaitingIdr` 与 `outOfRecoveryContextContinuation` 持续偏高。
- 根因集中在 decode 之后的决策分散：
  - `pacer` 在做 release 决策；
  - `render/present` 在做 latest slot 竞争决策；
  - `host` 仍在做最终呈现价值判断；
  - 同一条恢复链上存在多层比较器、多个“最新槽”、多次候选改写。
- 这类结构与 latest-only mailbox 的目标不一致：
  - decode 后链路需要单点收敛；
  - 恢复 owner 帧需要沿链路保持身份与优先级；
  - 可见性合同需要在 host 侧形成单一事实闭环。

## Goal

- 将 decode 后主决策层收敛到 `pacer`。
- 将 `render/present` 收敛为极薄 mailbox 执行层。
- 将 `host` 收敛为上屏执行层，负责 submit/present/平台资源与显示时钟。
- 建立 `CleanAnchorSubmitted -> CleanAnchorCommitted -> DisplayStable` 的清晰合同。
- 保留“等待新关键帧期间旧帧仍可继续显示”的原始设计意图。
- 消除 recovery owner 帧在 decode 成功后仍被后续普通 delta 顶掉的尾段失稳。

## Scope

- In scope:
  - [`crates/xbxengine/core/src/media/video/pacer/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/pacer/actor.rs)
  - [`crates/xbxengine/core/src/media/video/render/pacer.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/render/pacer.rs)
  - [`crates/xbxengine/core/src/media/video/render/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/render/actor.rs)
  - [`crates/xbxengine/core/src/media/video/render/renderer.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/render/renderer.rs)
  - [`src-tauri/src/mods/native_video/mod.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/native_video/mod.rs)
  - recovery owner / trace / validation 相关测试与投影
- Out of scope:
  - ingress、jitter、SampleBuilder、H264 bootstrap 输入侧重写
  - 新 transport / signaling / media pipeline
  - 平台 presenter 大规模重写
  - 推翻当前 recovery phase 术语和统计口径

## Problem Framing

### 1. 当前链路有三层决策

- `pacer` 决定当前候选是否 release。
- `render/present` 决定当前 latest slot 保留谁。
- `host` 决定当前 pending/displayed 是否替换。

这会产生三类后果：

- recovery owner 帧在不同层被按不同规则比较；
- `latest-only` 语义被拆成多段局部 latest；
- trace 很难回答“谁对最终未上屏负责”。

### 2. 当前故障集中在 decode 之后

- 目前输入侧的关键阻塞已有多轮修复：
  - feedback target bootstrap 缺口已补；
  - `first_frame_acquired` 已具备 recovery-epoch 语义；
  - renderer comparator 已补 owner 优先。
- 现象依然反复，说明根因是尾段职责分散，而不是单个 admission 条件。

### 3. 需要保留的原始设计意图

- 等待新关键帧期间，旧帧继续显示，画面持续保活。
- recovery 新锚点一旦具备显示价值，应沿链路持续保持 owner 优先，直到真正上屏。
- 一旦新锚点完成可见提交，系统进入 post-IDR climbing，再回到 steady latest-only。

## Design

### 1. 方案选择

本 RFC 固定采用 B 方案：

- `pacer` 成为 decode 后唯一主选择器；
- `render/present` 只保留 mailbox 执行职责；
- `host` 只负责上屏执行与可见事实回写。

### 2. 职责划分

#### `pacer`

- 输入：decode 后候选帧流。
- 持有：
  - `current_release`
  - `latest_release_candidate`
- 职责：
  - 唯一主比较器；
  - 唯一 release / hold / drop 决策点；
  - 统一维护 recovery owner、clean-anchor、post-IDR climbing、steady latest-only 的排序语义。

#### `render/present`

- 输入：来自 `pacer` 的单拍候选。
- 持有：
  - `inflight_current`
  - `latest_present_candidate`
- 职责：
  - mailbox handoff；
  - 下游忙时只保留单个最新候选；
  - 极薄保护规则，允许更高 `recovery_epoch` 抢占。

#### `host`

- 输入：runtime/present 交付的单个候选。
- 持有：
  - `displayed_current`
  - `pending_next`
- 职责：
  - submit
  - present
  - 平台资源生命周期
  - 显示时钟
  - visible fact 回写

### 3. 比较规则前移到 `pacer`

`pacer` 统一采用以下优先级：

1. 更高 `recovery_epoch`
2. owner 命中帧
3. clean-anchor 候选
4. recovery IDR 候选
5. post-IDR climbing 内的更新帧
6. steady 路径下更高 `rtp_timestamp`
7. 已过显示时效的帧降到最低优先级

约束：

- `render/present` 与 `host` 不再重新计算这套价值顺序；
- 两层执行链只识别 pacer 已给出的候选身份；
- trace 直接记录 pacer 判定结果与最终 host 可见结果。

### 4. 旧帧保活与新帧切换

- recovery 新锚点尚未具备上屏条件时，`displayed_current` 继续服务旧帧。
- recovery owner 帧一旦进入 `pending_next`，普通 continuation 不再在同一 epoch 内挤掉它。
- 更高 `recovery_epoch` 的 owner 候选可以越级抢占旧 epoch owner。
- `host` stall reset 继续保留旧 viewport 显示态，避免无画面硬切。

### 5. `CleanAnchorCommitted` 合同

- `CleanAnchorSubmitted`
  - decode / pipeline 事实；
  - 表示 recovery owner 帧已经被提交进显示链。
- `CleanAnchorCommitted`
  - host visible 事实；
  - 建议定义为：`last_displayed_frame_rtp_timestamp == recovery_owner_rtp_timestamp`。
- `DisplayStable`
  - host 在 `CleanAnchorCommitted` 之后持续完成 fresh present 的事实。

这三个事实分别回答：

- recovery owner 帧是否进入显示链；
- recovery owner 帧是否真正可见；
- 新链路是否已经持续稳定供给。

### 6. 状态机收敛

- decode 后统一采用两槽模型：
  - `current`
  - `latest_candidate`
- queue 语义继续退出 decode 后链路。
- trace 与 stats 收口以下语义：
  - `supersededByHigherValueCandidate`
  - `ownerProtectedPending`
  - `cleanAnchorCommitted`
  - `displayStable`

## Plan

1. 收敛 `pacer`：明确唯一主比较器与 release/hold/drop 合同。
2. 收敛 `render/present`：删除重复价值比较，保留 mailbox handoff 与极薄 epoch 抢占。
3. 收敛 `host`：删除恢复价值判断，只保留执行层状态与 visible fact 回写。
4. 收敛 phase gate：把 `CleanAnchorSubmitted/Committed/DisplayStable` 绑定到统一 owner 身份。
5. 补 trace 与测试：围绕 owner 帧保护、旧帧保活、新 epoch 抢占、visible commit 闭环补回归。

### Phase Plan

#### Phase 1: Presenter 边界收口

- 将 `MacOsWgpuPresenter`、`WindowsWgpuPresenter` 收到统一 `host-present mailbox` 提交语义。
- 删除 presenter 内部 `latest_frame` 的价值判断、overwrite/reject 决策。
- `renderer_state.latest_frame` 只允许作为“最近成功渲染的缓存帧”供 resize / repaint 使用。
- `MacOsDisplayLinkContext`、`run_wgpu_render_tick`、Windows 对应 tick 入口统一改为消费 `ScheduledFrameSlot::take_ready_frame(...)`。

#### Phase 2: Host-present mailbox 单体化

- `scheduling.rs` 成为唯一 host 尾段 mailbox。
- 唯一持有：
  - `displayed_current`
  - `pending_next`
  - `last_presented_frame_seq`
  - `view_epoch`
- overwrite / stale drop / no-pending / replay / owner protected telemetry 只从 mailbox 发出。

#### Phase 3: Visible contract 收口

- `CleanAnchorCommitted` 仅由 host visible fact 驱动。
- `DisplayStable` 仅由 committed 之后持续 fresh present 驱动。
- `runtime_port` 只吃 mailbox 汇总后的 visible facts，不再跨层拼补 fallback。

#### Phase 4: Trace 与指标收口

- 收敛事件名：
  - `pacerCandidateSelected`
  - `pacerCandidateDecision`
  - `renderMailboxDecision`
  - `renderMailboxStateTransition`
  - `hostMailboxAccepted`
  - `hostMailboxRejected`
  - `hostMailboxPendingProtected`
  - `hostMailboxState`
  - `hostFramePresentResumed`
  - `hostFramePresented`
  - `cleanAnchorCommitted`
  - `displayStable`
- 删除或降级表达“present 层再次挑帧”的旧事件。
- 用新 runtime trace 复核 recovery owner 是否还会在尾段失守。
- 当前进展：
  - `XbxEngineHostVideoPresentMetrics` 已统一为：
    - `host_mailbox_submit_epoch`
    - `host_display_tick_epoch`
    - `host_frame_present_epoch`
    - `host_mailbox_enqueue_count_total`
    - `host_mailbox_drop_count_total`
    - `host_mailbox_overwrite_count_total`
  - `XbxEngineMediaRuntimeStats`、`XbxEngineStatsDto`、前端 RPC 类型与 runtime snapshot 映射已同步切到同一命名
  - `trace_projection` 对外 payload 与 runtime trace 已删除旧 key，仅保留 mailbox / host-present 语义
  - `trace_projection` 内部 observation state 与 `native_video` viewport telemetry 字段已同步完成命名收口，避免 runtime/trace 继续混用 mailbox 计数与 present 事实
  - `clean-anchor` 可见提交路径已收成 `record_transport_clean_anchor_with_rtp(...)` 单入口；旧 visible-present 入口与历史残留测试语义已退出

## Validation

- [x] `cargo test -p xbxengine media::video::pacer -- --nocapture`
- [ ] `cargo test -p xbxengine media::video::render -- --nocapture`
- [x] `cargo test -p xbxrc mods::native_video::tests -- --nocapture`
- [x] `cargo test -p xbxrc mods::native_video::scheduling::tests -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::stack::runtime_port::tests -- --nocapture`
- [x] `cargo test -p xbxrc mods::xbxengine::trace_projection -- --nocapture`
- [x] `cargo check -p xbxengine`
- [x] `cargo check -p xbxrc`
- [ ] 新增 pacer 单点决策定向测试
- [ ] 新增 render mailbox 不再重算 owner 优先的定向测试
- [ ] 新增 host visible commit 合同测试
- [ ] 用新 runtime trace 验证：
  - `CleanAnchorCommitted` 上升
  - `DisplayStable` 上升
  - `continuationAcceptedWhileAwaitingIdr` 下降
  - `noCleanAnchorCommit` 下降
  - `keyframe_effectiveness.chain_recovered` 上升

## Risks

- `pacer` 比较器收权后如果规则定义不完整，会把历史局部特判丢失到执行层外。
- `host` visible fact 回写如果继续依赖松散近似条件，会让 `CleanAnchorCommitted` 继续失真。
- 旧帧保活与 owner 保护窗口如果没有按 epoch 明确切开，会继续产生跨 episode 污染。

## Progress

- [ ] Step 1: 完成 `pacer` 主比较器与候选身份合同
- [x] Step 2: 完成 `render/present` mailbox 执行化
- [x] Step 3: 完成 `host` visible commit 回写收口
- [ ] Step 4: 完成 trace/stats 字段对齐
- [ ] Step 5: 完成 trace 回归复核

## Execution Notes

- Date: 2026-04-28 | Status: in-progress
- Update: 基于近期 recovery trace，已确认主故障集中在 decode 后三层并行决策导致 owner 帧尾段失守。
- Decision: 本任务采用 B 方案，`pacer` 作为 decode 后唯一主决策层。
- Decision: `render/present` 与 `host` 收敛为执行链，保留 mailbox 与 visible fact 职责。
- Decision: 保留“等待新关键帧期间旧帧继续显示”的原始设计目标，并将 owner 保护窗口与 epoch 明确绑定。
- Update: 已将 `renderer` latest-slot 与宿主 `pending/displayed` 链路从完整 comparator 收敛为 mailbox 保护规则：仅保留 epoch 抢占、owner 锚点保护、stale 过滤与最新帧覆盖，删除 displayed 对 pending 的二次价值否决。
- Update: 已将 `CleanAnchorCommitted` 收敛为 host visible 合同：仅当当前 recovery epoch 存在 `chain-clean-anchor-submitted` submission，且 `last_displayed_frame_rtp_timestamp == latest_clean_anchor_submission_rtp_timestamp` 时才记账；删除“decoded owner 直接触发 cleanAnchorCommitted”的 fallback。
- Update: `MacOsWgpuPresenter` 与 `WindowsWgpuPresenter` 已统一切到 `ScheduledFrameSlot + HostCadenceTelemetry` mailbox 模型；presenter 本地二次挑帧与 `compare_host_present_frame_value` 测试 helper 已移除，host 侧仅保留 mailbox 统计与平台执行统计。
- Update: `native_video` host trace 事件已开始收口到 mailbox 语义：`hostMailboxAccepted / hostMailboxRejected / hostMailboxPendingProtected / hostMailboxIdle / hostMailboxSubmitGap / hostMailboxUpdateFailed / hostFramePresented`；旧 `frame_submit / frame_slot_take_* / sample_presented` 事件名已退出主链。
- Update: `analyze-runtime-logs` skill 已同步切到 `hostMailboxState` / `hostFramePresentResumed` 新事件名；依赖仓库外旧 trace 样本的黑盒测试已退出，只保留仓库内可复现合同。
- Update: `RuntimeTraceRecorder` 测试态停止并发 prune 活跃 trace 文件，`trace_projection` 历史读盘脆弱用例已改回统一 helper，避免测试互相删文件导致假红。
- Risk/Blocker: 需要先把 `CleanAnchorCommitted` 的 host 可见合同钉死，否则 recovery trace 仍会在 submitted/committed 之间漂移。
