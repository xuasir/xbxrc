# 解码后 Latest-Only Mailbox 收敛 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: planned
- Owner: Codex
- Last Updated: 2026-04-24

## Background

- 当前 `decode -> pacer -> render -> host present` 仍保留排队系统语义：
  - decode 有输出队列；
  - pacer 会承接和消费一段待发历史；
  - render/runtime/host 之间仍存在 staging / pending / latest 多层暂存；
  - trace 中仍持续出现 `outputQueueOverflow`、`staleAfterDecode`、`backendNoOutputAfterWaitingKeyframeContinuation`。
- 最新几份 recovery trace 已经稳定表明：
  - transport 侧可以继续收到 continuation；
  - H264 inspection 可以明确识别 `bootstrapMissingIdr` 与 `continuationAcceptedWhileAwaitingIdr`；
  - 真正放大卡死的常见因素，是 decode 之后的本地积压把恢复窗口拖老。
- 当前模型的问题不在某一个阈值，而在模型本身：
  - 三层浅队列串联后形成深积压；
  - 旧帧先进入队列，再在更后面因为 age 或 queue pressure 被淘汰；
  - 恢复期 continuation 和普通 steady 帧共享排队语义，clean anchor 到达后仍可能被旧帧阻塞；
  - `outputQueueOverflow` 这类容量型标签会掩盖真实语义，系统实际上是在“先排队，后淘汰”。
- 低延迟串流更适合另一种模型：
  - decode 前保顺序和参考链；
  - decode 后只保最新可显示候选；
  - host tick 作为唯一最终显示时钟；
  - 旧帧更多以“价值被覆盖”结束，而不是“队列溢出”结束。

## Goal

- 将 `decode -> pacer -> render -> host present` 收敛为 latest-only mailbox 链。
- 将顺序性严格收口在 `rtp reorder / jitter / H264 bootstrap / decoder input` 之前。
- 将 decode 后的本地链路从“排队系统”改成“当前帧 + 最新候选”的双槽语义。
- 将 drop 语义从容量溢出改成价值淘汰，直接反映低延迟设计意图。
- 让 recovery 关键帧、clean anchor、post-IDR 爬升在 latest-only 模型下继续保持优先权。
- 让 trace、stats、policy 看到的本地事实与真实故障域一致，减少脆弱调度链。
- 将 `upstreamSenderDropped`、`videoRtcpFeedbackTargetPending`、`twcc receiver mapping missing`、`pump exit` 收口为同一条控制面出口故障链。
- 让 `RtcVideoFrameSource rx closed` 与控制面退化共享结构化 episode 关联，而不是停留在分散日志。
- 将 `feedback target` 可用性提升为独立合同，要求图片级恢复、NACK、TWCC 都能看到明确的 `ready / warming / unavailable / lost` 状态。

## Scope

- In scope:
  - [`crates/xbxengine/core/src/media/video/decode/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/actor.rs)
  - [`crates/xbxengine/core/src/media/video/decode/video_decode.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/video_decode.rs)
  - [`crates/xbxengine/core/src/media/video/pacer/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/pacer/actor.rs)
  - [`crates/xbxengine/core/src/media/video/render/pacer.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/render/pacer.rs)
  - [`crates/xbxengine/core/src/media/video/render/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/render/actor.rs)
  - [`crates/xbxengine/core/src/media/video/render/renderer.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/render/renderer.rs)
  - [`crates/xbxengine/core/src/api/runtime/sync.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/runtime/sync.rs)
  - [`crates/xbxengine/core/src/transport/rtc/stack/runtime_port.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack/runtime_port.rs)
  - [`crates/xbxengine/core/src/runtime_stats_sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/runtime_stats_sink.rs)
  - [`crates/xbxengine/core/src/diagnostics/stats.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/diagnostics/stats.rs)
  - [`crates/xbxengine/core/src/transport/rtc/connection/service.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/connection/service.rs)
  - [`crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs)
  - [`src-tauri/src/mods/xbxengine/trace_projection.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/xbxengine/trace_projection.rs)
  - 相关 decode / pacer / renderer / runtime tests
- Out of scope:
  - RTP 重排序、NACK admission、SampleBuilder、H264 bootstrap 输入侧重写
  - host presenter 平台实现大改
  - 新增第二套 media pipeline
  - transport 控制动作主链改造

## Design

### 1. 总体原则

- decode 前：
  - 以顺序性、参考链、可解码性优先；
  - 允许重排序、修补、bootstrap gate、IDR 等待。
- decode 后：
  - 以显示时效优先；
  - latest-only；
  - 旧帧优先被更新覆盖；
  - host tick 是唯一最终显示时钟。

### 2. mailbox 模型

decode 后每一层只允许两类持有状态：

- `inflight_current`
  - 已被下一层接住、正在消费、暂时不可覆盖。
- `latest_candidate`
  - 下一次可以交给下游的最新候选，可被更高价值新帧覆盖。

这条规则应用到三段：

1. `decode -> pacer`
2. `pacer -> render/runtime`
3. `runtime -> host presenter`

任一层都不再承诺“排空历史队列”。

### 3. decode 输出语义

- 删除“普通 decoded output queue”主语义。
- decode actor 成功产出后，直接尝试写入 `latest_decoded_candidate`。
- 如果 pacer 正在消费当前帧，则保留一个 `latest_decoded_candidate`。
- 新帧到达时执行价值比较：
  - 更高 `recovery_epoch`
  - clean anchor / recovery anchor 候选优先
  - 同 epoch 下更新的 `rtp_timestamp`
  - 同类帧下更新的 display deadline
- 新帧价值更高时，直接覆盖旧候选。

结果：

- 常态下不再出现“decode 先堆满 5 帧，再因为输出队列溢出而丢”。
- decode 后的 drop 主语义改成“候选被更新覆盖”。

### 4. pacer 语义

- pacer 从“带历史待发队列的调度器”收敛为“当前帧准出控制器”。
- pacer 只维护：
  - `current_release`
  - `latest_release_candidate`
- pacer 不再承担“排空旧历史帧”的义务。
- pacer 的决策仍保留：
  - `Drop`
  - `Ready`
  - 极短暂的 `HoldForRecovery`
- `HoldForRecovery` 只服务于恢复锚点保护，不服务于普通 steady backlog。
- 新候选到达后，pacer 比较当前 `latest_release_candidate` 与新帧价值，保留更值得显示的一帧。

### 5. render/runtime 语义

- render/runtime 不再承接普通 pending queue 语义。
- runtime tick 每拍最多只向 host 交付一个普通显示候选。
- host 忙时保留一个 `latest_present_candidate`，新帧可覆盖旧 pending。
- runtime 不再跨多个 tick 排空历史 render frame。
- `latestSlotOverwrite` 从“latest 队列挤压”收口为“最新候选被更高价值帧覆盖”。

### 6. host presenter 语义

- host presenter 仍是最终显示时钟拥有者。
- host 继续负责：
  - display tick
  - actual present
  - 平台侧资源生命周期
- host 不承担普通 backlog 排空语义。
- 当 host 下游仍在消费当前帧时，只保留一个 `latest_present_candidate`。
- host 下一拍只消费当前最值得显示的候选。

### 7. 价值比较规则

latest-only 成败取决于比较规则，统一采用以下优先级：

1. 更高 `recovery_epoch`
2. `clean anchor` 候选
3. 同 epoch 的 recovery IDR 候选
4. post-IDR 爬升窗口内的更新帧
5. steady 状态下更高 `rtp_timestamp`
6. 已过 display deadline 的帧最低

结果：

- clean anchor 一到，旧 continuation 立刻让位；
- 爬升期继续优先收新帧；
- 普通 steady 帧只保最新。

### 8. 控制面出口故障链

当前 `upstreamSenderDropped`、`videoRtcpFeedbackTargetPending`、`twcc queue feedback packet without receiver mapping`、`pump exit` 还分散在日志和单点 observation 中。这个状态会让 `rx closed` 看起来像孤立终点。

本 RFC 将它们收口成统一的结构化故障链：

- `controlPlaneEpisode`
  - `episode_id`
  - `recovery_epoch`
  - `transport_state`
  - `owner_state`
  - `feedback_target_state`
  - `feedback_target_availability`
  - `twcc_mapping_state`
  - `pump_state`
- `controlPlaneFailureObserved`
  - `failure_kind`
  - `failure_reason`
  - `linked_episode_id`
  - `linked_recovery_epoch`
  - `source_subsystem`
- `videoIngressTermination`
  - 保留 `kind=rxClosed`
  - 新增结构化 `upstream_cause`
  - 新增 `linked_control_plane_episode_id`

其中 `feedback target` 状态单独建模，至少覆盖：

- `unbound`
- `warming`
- `ready`
- `degraded`
- `unavailable`
- `lost`

定义：

- `unbound`
  - transport 已连接，video RTCP feedback target 还未建立绑定。
- `warming`
  - target 已出现，尚未完成稳定可发送验证。
- `ready`
  - PLI / NACK / TWCC 至少有一条发送路径已确认可用。
- `degraded`
  - target 仍存在，但最近发送失败、mapping 缺失或 pump 异常正在抬升。
- `unavailable`
  - 当前明确无 target，不能发送 video RTCP feedback。
- `lost`
  - 之前已 ready/degraded，之后发生解绑、通道坍缩或 sender drop。

上游 cause 统一枚举化，至少覆盖：

- `feedbackTargetUnavailable`
- `feedbackTargetLost`
- `twccReceiverMappingMissing`
- `connectionPumpExited`
- `peerEventsDrained`
- `stackStop`
- `rebuildPeerConnection`
- `unknown`

规则：

- `RtcVideoFrameSource rx closed cause=upstreamSenderDropped` 不再只写原始 cause；
- `feedback target` 状态变化进入独立 observation，而不是只在 command result 上留下 deferred/suppressed 文案；
- connection/service 在 sender drop 前就写入最近的控制面故障事实；
- `videoIngressTermination` 直接链接控制面 episode；
- `videoRtcpFeedbackTargetPending`、TWCC mapping 缺失、pump exit 形成同一条结构化因果链；
- trace 可以直接回答“rx closed 前控制面出口经历了什么”。

### 8.1 feedback target 可用性合同

`feedback target` 可用性不再只是恢复请求失败后的结果字符串，而是控制面的前置合同。

要求：

- `RequestPli`
- `NACK`
- `TWCC feedback`

三类动作统一依赖同一份 `feedback target availability` 事实。

实现目标：

1. `connection/service` 维护单一 target availability 状态机。
2. `transport_session`、`stream/video_source/nack`、TWCC sender 都读取同一状态，而不是各自推断。
3. `videoRtcpFeedbackTargetPending` 从“请求结果文案”升级为“状态机当前状态”。
4. `warming -> ready` 需要一次真实可发送证据，避免刚绑定就被误当成可用。
5. `ready/degraded -> lost` 需要明确事件，直接挂到 control-plane episode。

设计收益：

- PLI deferred、NACK send failed、TWCC mapping missing 可以共享同一可用性上下文；
- trace 可以区分“当前没有出口”和“出口已建立但正在退化”；
- `rx closed` 前的控制面衰退链路能提前暴露，而不是只在终点看到 `upstreamSenderDropped`。

### 9. 丢帧语义重命名

当前 `outputQueueOverflow`、`rendererQueueOverflow` 更像容量型故障，不适合作为 latest-only 主语义。

本 RFC 收口为以下 drop detail：

- `supersededAfterDecode`
- `supersededByRecoveryAnchor`
- `supersededDuringRampUp`
- `missedDisplayDeadline`
- `localHardCapProtection`
- `hostBusyRetainedCurrent`

规则：

- 真正资源耗尽时才记录容量型 drop；
- 常态 drop 优先表达“价值被覆盖”；
- trace 直接可见 latest-only 决策结果。

### 10. recovery 语义约束

latest-only 只改变 decode 后显示链，不改变 recovery 完成判据：

- `Decoded`
- `CleanAnchorCommitted`
- `DisplayStable`

clean anchor 与 display gate 继续保持两层语义：

- `CleanAnchorCommitted`：media gate
- `DisplayStable`：display gate

clean anchor 提交必须绑定“真正被下游接住”的那一帧，不能在候选阶段提前消费。

### 10. 与现有策略的关系

- 保留 `post-IDR climbing`
- 保留 clean-anchor 宽容
- 保留 remote profile / phase policy
- 保留 host cadence telemetry

这些策略统一只做两类事情：

- 调整比较规则与短时保护窗口
- 调整恢复完成判据与 ramp-up 宽容

这些策略不再用来维持普通 backlog。

## Current vs Target

改造前：

- decode 后以浅队列串联运行
- 旧帧先入队，后淘汰
- `outputQueueOverflow` / `staleAfterDecode` 承担主诊断标签
- runtime/host 仍会跨 tick 处理历史积压
- `rx closed`、feedback target pending、TWCC mapping 缺失、pump exit 分散存在

改造后：

- decode 后以 mailbox 串联运行
- 旧帧优先被新帧覆盖
- 主诊断标签变成 `superseded*` / `missedDisplayDeadline`
- runtime/host 每拍只处理最新候选
- 控制面出口退化进入统一 episode 与结构化因果链

## Implementation Plan

### Phase 1: decode mailbox 化

1. 删除 decode 普通输出队列主语义。
2. 引入 `current_submitted` + `latest_decoded_candidate`。
3. 新增帧价值比较函数，统一比较 epoch / anchor / rtp_ts / deadline。
4. 将 `outputQueueOverflow` 主路径改成 `supersededAfterDecode`。

### Phase 2: pacer mailbox 化

1. 删除 pacer 历史待发队列主语义。
2. 收敛为 `current_release` + `latest_release_candidate`。
3. `HoldForRecovery` 只保留极短恢复保护语义。
4. steady backlog 不再通过 sleep / queue depth 维持。

### Phase 3: render/runtime latest-only 化

1. 删除普通 pending render frames 语义。
2. runtime tick 改成单拍取 latest。
3. `runtime_port` 改成 latest-only handoff。
4. host busy 时只保留一个 `latest_present_candidate`。

### Phase 4: trace / stats / policy 收口

1. 新增 `superseded*` 系列 trace/detail。
2. 将 `outputQueueOverflow` 退回真正 hard-cap 保护场景。
3. 区分：
   - `localDisplaySuperseded`
   - `localDeadlineMissed`
   - `localHardCapProtection`
4. 让 policy 消费新的本地显示语义，减少把 latest-only 覆盖误判成恢复失败。
5. 新增控制面故障链 observation：
   - `controlPlaneEpisode`
   - `controlPlaneFailureObserved`
   - `videoIngressTermination.upstream_cause`
6. 新增 `feedbackTargetAvailabilityObserved`，统一表达 `unbound / warming / ready / degraded / unavailable / lost`。
7. 将 `videoRtcpFeedbackTargetPending`、TWCC mapping 缺失、pump exit 与 `rx closed` 统一建立 episode 关联。
8. 将 PLI/NACK/TWCC 三条发送路径改为共享同一 `feedback target availability` 合同。

## Validation

- [ ] `cargo test -p xbxengine media::video::decode::actor -- --nocapture`
- [ ] `cargo test -p xbxengine media::video::decode::video_decode -- --nocapture`
- [ ] `cargo test -p xbxengine media::video::pacer::actor -- --nocapture`
- [ ] `cargo test -p xbxengine media::video::render -- --nocapture`
- [ ] `cargo test -p xbxengine api::runtime::sync -- --nocapture`
- [ ] `cargo test -p xbxengine transport::rtc::connection::service -- --nocapture`
- [ ] `cargo test -p xbxengine transport::rtc::stream::video_source::nack -- --nocapture`
- [ ] 新增 decode mailbox 定向测试：
  - 新 recovery anchor 覆盖旧 continuation
  - steady 帧只保最新
- [ ] 新增 pacer mailbox 定向测试：
  - host 忙时只保留最新候选
  - `HoldForRecovery` 不会演化成普通 backlog
- [ ] 新增 render/runtime latest-only 定向测试：
  - 单个 runtime tick 最多交付一帧
  - 历史帧不会跨 tick 排空
- [ ] 用新 trace 验证：
  - `outputQueueOverflow` 显著下降
  - `supersededAfterDecode` 成为常态本地淘汰语义
  - `staleAfterDecode` 不再是恢复期主 drop reason
  - clean anchor 到达后首帧显示时延缩短
  - `feedback target availability` 具备清晰状态跃迁
  - PLI/NACK/TWCC 共享同一 target availability 语义
  - `rx closed` 可直接关联到 `feedbackTargetUnavailable / twccReceiverMappingMissing / connectionPumpExited`
  - `videoRtcpFeedbackTargetPending`、TWCC mapping 缺失、pump exit 具备同一 episode id

## Risks

- 价值比较规则收得过粗，会误伤某些短窗口内的恢复保护。
- host presenter 如果仍隐藏历史 pending 语义，会把 backlog 转移到平台侧。
- 某些统计仍按“队列溢出”理解显示链，迁移期间会出现口径错位。
- connection/service 如果只补局部 observation、不补 episode 关联，控制面链仍会继续断裂。
- 如果 `feedback target` 状态机没有成为单一事实源，PLI/NACK/TWCC 仍会各自漂移，最终继续回到字符串驱动。

## Progress

- [ ] Step 1: 明确 decode mailbox 状态与比较规则
- [ ] Step 2: 明确 pacer mailbox 状态与短暂恢复保护
- [ ] Step 3: 明确 render/runtime latest-only handoff
- [ ] Step 4: 明确控制面出口故障链 observation 与 episode 关联
- [ ] Step 5: 明确 feedback target availability 状态机与共享合同
- [ ] Step 6: 明确 trace/stats/drop detail 新口径
- [ ] Step 7: 代码实现与定向测试
- [ ] Step 8: 新 trace 回归

## Execution Notes

- Date: 2026-04-24 | Status: planned
- Update: 基于最新 recovery trace 与当前代码行为，确认问题核心是 decode 后链路仍采用排队系统语义。
- Decision: 新执行主线采用“decode 前保顺序、decode 后 latest-only mailbox”。
- Decision: local display chain 的常态淘汰语义从容量溢出改成价值覆盖。
- Decision: clean anchor / DisplayStable 两层 gate 保持不变，latest-only 只改变 decode 后交付方式。
- Decision: `upstreamSenderDropped`、`videoRtcpFeedbackTargetPending`、TWCC mapping 缺失、pump exit 一并纳入同一控制面出口故障链。
- Decision: `feedback target availability` 升级为控制面单一事实源，PLI/NACK/TWCC 共用同一状态机。
- Risk/Blocker: host presenter 如果仍保留隐藏 backlog，系统会把积压从 core 转移到平台侧，需同步收口 host handoff 语义。
