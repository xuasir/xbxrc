# 帧到显示：局部能力梳理

更新时间：2026-04-15

本文梳理从 `AssembledVideoFrame` 离开 source 层到最终呈现这段链路的现有实现与待优化项。与 `packet-to-frame-pipeline.md` 衔接：入口是 `recv_frame_inner` 返回的 `AssembledVideoFrame`，出口是 Renderer 的 present 回调。

## 1. 链路全貌

```
AssembledVideoFrame
  → materialize_ingress_frame()          [ingress/budget.rs]   — 物化调度元数据
  → VideoIngress.submit()                [ingress/scheduler.rs] — 队列门控、迟到丢弃
  → drain_ingress_to_decode()            [pipeline/ingress.rs]  — 背压感知的解码提交
  → DecodeActorHandle.submit()           [decode/actor.rs]      — 解码
  → PacerActorHandle.submit()            [pacer/actor.rs]       — 时序调度
  → RendererActorHandle.submit()         [render/actor.rs]      — 呈现
```

并行路径：
- `IngressDecision` → `VideoIngressSignal` → `diagnose_ingress_signal` → `TransportFact`
- `RecoveryCoordinator` 消费 `TransportFact`，做跨层恢复升级决策

## 2. 各阶段分析

### 2.1 Ingress 物化（ingress/budget.rs）

这一层把 `AssembledVideoFrame` 转化为 `EncodedFrame`，核心工作是计算 `target_playout_time`——帧应该被送入解码器的目标时刻。入口是 `materialize_ingress_frame(frame, min_delay, max_delay)`，出口是带 `target_playout_time` 的 `EncodedFrame`。

#### 现有能力

**`FrameBudgetContext` 的语义**

`FrameBudgetContext` 是贯穿整个链路的帧级调度上下文，由 source 层（NACK 路径）写入 `AssembledVideoFrame.budget`，在物化阶段直接使用。包含五个维度：

| 字段 | 类型 | 语义 |
|------|------|------|
| `recovery_phase` | `Steady / Repairing / AwaitingKeyframe / Reconfiguring` | 当前帧所处的恢复阶段 |
| `link_value` | `Disposable / Supply / Anchor` | 帧在参考链上的价值等级 |
| `rtt_slack` | `Unknown / Ample / Tight / Exhausted` | deadline 与预计到达时间之间的余量 |
| `failure_cost` | `LocalDrop / WaitKeyframe / Reconfigure / ChainBroken` | 丢弃这帧的代价 |
| `window_source` | `Playout / Transport / Recovery / Reconfigure` | deadline 窗口的来源 |

**`link_value` 的推导规则**

`resolve_link_value` 基于 `FrameValue`（`is_sync_point` / `refresh_boost`）和 `recovery_phase` / `failure_cost` 推导：

- `is_sync_point`（IDR）→ 始终 `Anchor`
- `refresh_boost`（reference 帧）+ `AwaitingKeyframe` → 提升为 `Anchor`
- `Disposable`（delta）+ `AwaitingKeyframe/Repairing` + `ChainBroken` → 提升为 `Supply`
- `Disposable` + `Reconfiguring` + `Reconfigure` → 提升为 `Supply`
- 其余保持基础值

**`target_playout_time` 的计算**

```
target_playout_time = playout_base_at.unwrap_or(assembled_at)
                    + resolve_playout_delay(value, min_delay, max_delay, context)
```

`resolve_playout_delay` 的逻辑：

- `link_value == Anchor` 或 `failure_cost == ChainBroken` → 直接用 `max_delay`（默认 30ms），给恢复帧最大等待窗口
- 其余：在 `[min_delay, max_delay]`（默认 20ms~30ms）区间内，按 `deadline_budget_ratio_per_mille` 线性插值

`deadline_budget_ratio_per_mille` 由 `FrameValue` 基础值（IDR=1000‰，reference=700‰，delta=450‰）加上 `FrameBudgetContext` 各维度的调整量叠加，最终 clamp 到 [250, 1600]‰。调整方向：

- `link_value` 越高 → ratio 越大（Anchor +280，Supply +120）
- `failure_cost` 越重 → ratio 越大（ChainBroken +260，Reconfigure +150）
- `rtt_slack` 越紧 → ratio 越小（Exhausted -240，Tight -120，仅 deadline_budget 路径）
- `window_source == Recovery` → +100，`Reconfigure` → +140

**`FrameBudgetContext` 的其他用途**

`FrameBudgetContext` 不只用于物化，还被后续层消费：

- `backlog_priority_score`：ingress 队列积压时的保留优先级评分
- `repair_priority`：NACK 调度的优先级（1~4）
- `retry_budget`：NACK 最大重试次数
- `prefers_chain_broken / prefers_wait_keyframe / prefers_reconfigure`：ingress 调度的门控判断
- `frame_importance`：`FrameBudgetContext` 在 NACK/准入策略侧生成的重要性标签字符串（`"keyframe" / "reference" / "delta"`）；它不是 `AssembledVideoFrame`/`EncodedFrame` 的结构字段

#### 待优化项

**1. `assembled_at` 包含 SampleBuilder 固有延迟**

当前优先使用 `playout_base_at`（若存在）作为基准，缺失时回退到 `assembled_at`。`assembled_at` 是 `sample_builder.pop()` 返回时记录的时刻，已包含 `SampleBuilder` 等待下一帧第一个包的固有延迟（约一个帧间隔，60fps ≈ 16.7ms）；因此在无 `playout_base_at` 回退路径下，`target_playout_time` 仍可能相对媒体时间偏晚。

### 2.2 Ingress 调度（ingress/scheduler.rs）

这一层是帧进入解码队列前的第二道门控，负责准入判断、迟到丢弃、积压控制。入口是 `VideoIngress.submit(frame, now)`，出口是 `IngressDecision` 枚举值，同时把帧放入内部队列等待 `drain_ingress_to_decode` 取走。

#### 现有能力

**`IngressDecision` 的七种结果**

| 结果 | 语义 |
|------|------|
| `Submit` | 帧入队，可以提交给解码器 |
| `DropLate` | 帧已超过 playout deadline，丢弃 |
| `DropBacklogIncoming` | 队列积压且新帧价值不高于队内最低值，新帧丢弃 |
| `DropBacklogEvictQueued` | 队列积压且新帧价值更高，替换队内最低值后新帧入队 |
| `DropUnrecoverable` | 参考链不可恢复或帧已标记为 late，直接丢弃 |
| `WaitKeyframe` | 等待下一个干净 keyframe，当前帧丢弃 |
| `Reconfigure` | 检测到编解码器/分辨率/参数集变化，清空队列等待 keyframe |

**`submit` 的完整决策流程**

```
1. 计算 config_mismatch（codec/宽高与当前状态不符）
2. 重新计算 context（FrameBudgetContext.for_ingress_admission）：
   - UnrecoverableReferenceChain → waiting_keyframe=true
   - waiting_keyframe=true → 重算 context
   - config_mismatch/config_changed/parameter_sets_changed → 重算 context
3. UnrecoverableReferenceChain 或 prefers_chain_broken → DropUnrecoverable，清空队列，waiting_keyframe=true
4. UnrecoverableLate → DropUnrecoverable
5. bootstrap_ready（SPS+PPS+IDR 齐全）→ Submit，清空 backlog，更新当前状态，waiting_keyframe=false
6. can_exit_waiting_keyframe_with_recovery_continuation → Submit（恢复期 delta continuation）
7. is_keyframe（`bootstrap_ready=false` 的 keyframe，步骤 5 已处理 `bootstrap_ready=true` 的情况）：
   - 无论如何先更新 observed_codec/width/height（记录最新流参数，但帧不入队）
   - `waiting_keyframe=true` → `WaitKeyframe`（keyframe 但 SPS/PPS 不完整，无法建立 committed 参数集）
   - 否则 → `Submit`，清空 backlog，`commit()`
8. config_changed 或 config_mismatch：
   - prefers_wait_keyframe → WaitKeyframe
   - prefers_reconfigure → Reconfigure，清空队列
9. prefers_wait_keyframe → WaitKeyframe
10. is_frame_too_late → DropLate
11. drain_expired_queued_frames（清掉队列里已过期的帧）
12. 队列满（`>= backlog_drop_threshold_packets`，来自运行时配置）→ 按 `backlog_priority_score` 替换最低价值帧（`DropBacklogEvictQueued`）或丢新帧（`DropBacklogIncoming`）
13. 入队 → Submit
```

**迟到判断（`is_frame_too_late`）**

```
frame_late_threshold = late_frame_drop_threshold（默认 500ms）
                       × late_budget_ratio_per_mille（IDR=1000‰，reference=800‰，delta=500‰）
                       / 1000
                       （floor: 33ms）
```

delta 帧的有效迟到阈值约 250ms，IDR 帧约 500ms。`now > target_playout_time + frame_late_threshold` 时判定为迟到。

**积压控制（backlog）**

队列满时，遍历队列找 `backlog_priority_score` 最低的帧，与新帧比较：
- 新帧分数 ≤ 最低分 → `DropBacklogIncoming`（丢新帧）
- 新帧分数 > 最低分 → 替换最低分帧，新帧入队，返回 `DropBacklogEvictQueued`

`backlog_priority_score` 综合 `FrameValue` 基础分（IDR=1000，reference=300+250=550，delta=300，另加最多 256 的 size_bonus）和 `FrameBudgetContext` 各维度加权（Anchor +700，ChainBroken +600，AwaitingKeyframe +260 等）。

**恢复期 delta continuation 的特殊路径**

`can_exit_waiting_keyframe_with_recovery_continuation`：在 `waiting_keyframe=true` 时，如果帧满足以下全部条件，允许跳过 keyframe 等待直接提交：
- 无 config_mismatch，无 config_changed，无 parameter_sets_changed
- committed SPS/PPS 已存在（`committed_sps_present && committed_pps_present`）
- `delta_continuation_ready`（有 VCL NAL 且 slice header 有效）

这是恢复链路的关键路径：干净 IDR 建立 committed 参数集后，后续 delta 帧可以在 `waiting_keyframe` 状态下直接通过，不需要再等一个完整 IDR。

**`waiting_keyframe` 的状态管理**

- 初始化为 `true`（冷启动必须等 IDR）
- `bootstrap_ready` 帧到达 → `false`
- `can_exit_waiting_keyframe_with_recovery_continuation` 满足 → `false`
- `UnrecoverableReferenceChain` 到达 → `true`，清空队列
- `start_reconfigure()` 调用 → `true`，清空队列

#### 待优化项

无待优化项。backlog 决策拆分、过期双触发、continuation 契约收敛、observed/committed 双状态均已落地；其余为既有架构权衡。

### 2.3 恢复诊断与 RecoveryCoordinator（recovery/）

这一层把 ingress/transport 产出的 `VideoIngressSignal` / `VideoRecoverySignal` 映射成 `VideoEscalationDecision`，决定下一步执行 `RequestKeyframe`、`RequestDecoderReset` 还是 `RequestReconnectCandidate`。入口是 `RecoveryCoordinator.propose_from_owner_signal()`，出口是 `VideoEscalationDecision.action`。

#### 现有能力

**信号 → 诊断 → 升级的三层结构**

```
VideoIngressSignal / VideoRecoverySignal
  → diagnose_ingress_signal / diagnose_transport_signal   [diagnosis.rs]
  → VideoEscalationReason
  → RecoveryCoordinator.propose_from_owner_signal()       [coordinator.rs]
  → VideoEscalationDecision { action: RecoveryAction }
```

`diagnosis.rs` 只做标签映射（`ingressWaitKeyframe` → `WaitKeyframe`），不含策略逻辑。策略全部在 `coordinator.rs` 和 `escalation.rs`。

**`VideoEscalationController` 的单调梯子**

`escalation.rs` 实现 RFC CostCeiling 单调梯子：

| 层级 | 动作 |
|------|------|
| Absorb | `WaitForBurst` / `CooldownSuppressed` / `CoalescedKeyframeInFlight` / `CoalescedDecoderResetInFlight` |
| LocalRecover | `RequestKeyframe` / `RequestDecoderReset` |
| TransportRecover | `RequestReconnectCandidate` |

每个 `RecoveryAction` 有对应的 `RecoveryActionContract`，记录 owner、budget_kind 和 budget_recorded_on_execution。每个 recovery epoch 内有独立的 keyframe/decoder_reset/reconnect 预算上限（由 `RecoveryScenarioProfile` 按场景配置）。

**场景化参数（policy.rs）**

三种场景各有独立参数：

| 场景 | escalation_cooldown_ms | hard_fallback_timeout_ms |
|------|------------------------|--------------------------|
| HomeLanGaming | 260ms | 2400ms |
| CloudGaming | 420ms | 4500ms |
| RelayGaming | 360ms | 3600ms |

**`RecoveryCoordinator` 的决策流程**

`propose_from_owner_signal` 按以下优先级依次检查：

1. `sync_keyframe_transport_feedback` — 同步 keyframe 在途状态（UnsentPending / SentPending / Terminal）
2. `sync_decoder_reset_transport_success` — 同步 decoder reset 已执行
3. `release_stale_transport_await_keyframe_family` — 释放过期 keyframe epoch
4. `track_await_recovery_keyframe_streak` — 累计 transport-await 连续失败次数
5. `resolve_decoder_backend_failure_recovery` — 硬件解码器连续失败的专项处理
6. `resolve_persistent_stall_recovery` — 长时间 0kbps 硬停滞的专项处理
7. `resolve_recent_repeat_suppression` — 短窗口内抑制重复 reason
8. `resolve_recent_nack_outcome` — 消费最近 NACK 结果（已追回则抑制，已过期则升级）
9. `on_reason_with_policy` — 主策略路径

**transport-await 的分阶段恢复**

`TransportAwaitRecoveryKeyframe` 是最复杂的 reason，内部分四个阶段：

| 阶段 | 语义 |
|------|------|
| `ProbeKeyframe` | 初始探测，发送 keyframe 请求 |
| `BootstrapInFlight` | clean anchor 已提交，等待解码输出推进（sustaining 窗口） |
| `AwaitDecodeProgress` | keyframe 已到达，等待解码成功 |
| `AwaitDecoderResetProgress` | decoder reset 已发出，等待重置完成 |

sustaining 窗口（`BootstrapInFlight`）期间会抑制升级，避免把正在恢复中的帧误判为失败。若 inspection 给出「无效恢复 bootstrap」（NonIDR 等），sustaining 被击碎，重新允许升级。

**hard fallback 超时机制**

`resolve_transport_await_hard_fallback` 在以下条件下启动计时：
- transport-await 仍未解
- 没有 bootstrap-in-flight 保护
- 有明确的 stall 证据（renderer/decoder stalled、present 超龄、no-pending 连续高压）

超时后（按场景 2.4s~4.5s）强制推进到 `RequestDecoderReset` 或 `RequestReconnectCandidate`，打破本地恢复回路。

**burst rollback 机制**

`VideoEscalationBurstRollbackSnapshot` 在 `on_reason_with_epoch_policy` 之前快照 burst 相关状态。若 coordinator 最终把动作压成 `WaitForBurst` / `CoalescedDecoderResetInFlight` 等非执行动作，则回滚快照，避免"未执行仍吃掉 burst 计数"。

#### 待优化项

**1. `DropBacklogEvictQueued` 映射语义**

拆分前 `DropBacklog` 统一映射到 `WaitKeyframe`，会让 recovery 诊断把"队列内替换"误判为帧丢失，可能触发多余的 keyframe 请求。拆分后新增 `FrameQueued` 变体，`DropBacklogEvictQueued` 映射到 `FrameQueued`，不触发恢复升级。

**2. `FrameQueued` 实际上是死代码路径**

`diagnosis.rs` 的 `FrameQueued` 变体保留了穷举兜底实现（映射到 `ingressFrameQueued` 标签），但这段代码实际上不会被执行。真正的保护在 `session_loop.rs` 的上游过滤：`VideoIngressSignal::from_decision` 只在 `WaitKeyframe | DropUnrecoverable | Reconfigure` 三种决策上触发 `diagnose_ingress_signal` 调用，`DropBacklogEvictQueued` 根本不进入该路径。

---

### 2.4 解码（decode/actor.rs）

这一层把 `EncodedFrame` 送入硬件/软件解码器，输出 `DecodedFrame` 给 Pacer。入口是 `DecodeActorHandle.submit()`，出口是 `PacerActorHandle.submit()`。

#### 现有能力

**actor 结构**

`DecodeActorHandle` 持有一个容量为 2 的 `SyncSender<DecodeMsg>`，解码在独立线程 `XbxDecodeActor` 里同步运行。`available_slots`（AtomicUsize）和 `pending_output_backpressure`（AtomicBool）通过原子变量暴露给上游的 `drain_ingress_to_decode`，上游据此决定是否继续提交。

**背压机制**

解码输出队列（容量 3）满时，`ingress_demand().should_pull_output_first()` 返回 true，decode loop 进入背压等待（4ms 轮询），同时把 `pending_output_backpressure` 置 true，通知上游暂停提交。背压解除后恢复正常。

**decoder stall 判定**

`derive_decoder_stalled` 基于两个条件：
- 最近一次包到达时间在 400ms 内（说明有包在来）
- 最近一次解码成功时间超过 1000ms 前

两者同时满足才判定为 stalled，避免把"没有包"误判为解码器卡死。

**本地 decoder reset**

`request_local_decoder_reset` 通过 channel 发送 `LocalDecoderReset` 消息，decode loop 调用 `decode_state.request_local_decoder_reset()`。reset 成功返回 `Ok(true)`，已有 reset 在途时返回 `Ok(false)`（合并），失败时记录 warn log。

**输出队列溢出处理**

`process_encoded_frame` 返回被挤出的帧时，decode loop 调用 `record_pipeline_frame_drop` 记录 `outputQueueOverflow`，不静默丢弃。

#### 待优化项

**1. 解码输出队列容量固定为 3**

`DECODE_OUTPUT_QUEUE_CAPACITY = 3` 是硬编码常量。在 Pacer 因 renderer 背压暂停消费时，decode loop 会持续产出帧直到队列满，然后进入背压等待。这个容量在高帧率（60fps）下约 50ms 的缓冲，基本合理，但没有动态调整能力。

**2. 背压轮询的 CPU 开销**

背压期间 decode loop 每 4ms 轮询一次（`PENDING_PACER_RETRY_TIMEOUT_MS`），没有条件变量通知机制。在 Pacer 长时间背压（如 renderer 卡住）时会产生不必要的 CPU 消耗。

---

### 2.5 Pacer（pacer/actor.rs）

这一层把解码后的 `DecodedFrame` 按 `pts`（即 `target_playout_time`）调度送给 Renderer。入口是 `PacerActorHandle.submit()`，出口是 `RendererActorHandle.submit()`。

#### 现有能力

**队列结构**

Pacer 维护两个队列：
- `pacing_queue`（容量 3）：等待调度的解码帧
- `render_queue`（容量 1）：已就绪、等待 Renderer 消费的帧

**`FramePacingPolicy.decide()` 的调度逻辑**

| 条件 | 动作 |
|------|------|
| `now > pts + catch_up_threshold`（默认 500ms） | `Drop`，进入 catch-up 模式 |
| catch-up 模式且 `now > pts + catch_up_threshold` | `Drop`，保持 catch-up |
| catch-up 模式且帧未超期 | `SubmitNow`，退出 catch-up |
| `now >= pts` | `SubmitNow` |
| `pts - now <= long_sleep_guard`（默认 ≤ 刷新间隔，最大 20ms） | `Sleep(pts - now)` |
| `pts - now > long_sleep_guard` | `SubmitNow`（target playout 异常偏大时快速追帧） |

**宿主机节拍对齐（`resolve_host_release_wait_duration`）**

Pacer 用宿主机 present 回调的 epoch（`host_display_tick_epoch`）和 `latest_video_host_present_time_ms` 实现节拍对齐：

- 若 `display_tick_epoch` 有新 tick（epoch 推进），立即允许出帧
- 若同一 tick 内已消费过帧，计算距下一个 present 节拍的剩余时间作为 `host_release_wait`
- `HostCadencePhaseHint::Starved` 时跳过等待（host 已进入 no-pending，需要尽快补帧）
- `HostCadencePhaseHint::Priming` 且首帧未到时，限制同一 tick 内的连续出帧

这与 Moonlight 的 V-sync 拉模型语义相近，但信号来源不同——我们跟宿主机 present 节拍走，是有意为之（云游戏帧率由服务端决定，不能强绑本机刷新率）。

**`QueueHistoryController.decide_drop_target()` 的多维度丢帧决策**

`decide_drop_target` 综合以下维度决定 `drop_target`（1 或 3）：

| 维度 | 触发条件 | 效果 |
|------|----------|------|
| `sustained_backlog` | 近期历史队列深度持续 >1 | 收紧到 1 |
| `overwrite_degraded` | present overwrite ratio ≥ 5% | 收紧到 1 |
| `cadence_degraded` | present fps 比 display fps 低 25% 以上 | 收紧到 1 |
| `host_critical` | no-pending 连续 ≥120 帧且 level=critical | 收紧到 1 |
| `phase_starved` | `HostCadencePhaseHint::Starved` | 收紧到 1，aggressive=true |
| `overwrite_critical` | overwrite ratio ≥ 12% | 收紧到 1，aggressive=true |
| `cadence_critical` | cadence lag ≥ 55% | 收紧到 1，aggressive=true |

各维度都有实际效果，覆盖不同的压力来源（队列积压、渲染覆写、帧率落后、host 饥饿）。`aggressive=true` 时 `enforce_queue_budget` 会用 `queuePressureAggressive` 标签记录丢帧，便于区分普通压力和极端压力。

**render_queue 的帧替换策略**

`render_queue` 容量为 1。新帧到达时若队列已满，`should_replace_render_queue_head` 按以下规则决定是否替换：
1. 现有帧已过期（`render_frame_is_stale`）→ 替换
2. 新帧优先级更高（keyframe=3 > reference=2 > delta=1）→ 替换
3. 同优先级时新帧 pts 更新 → 替换

被替换的帧记录为 `rendererQueueReplaceStale` 或 `rendererQueueOverflow`，被拒绝的新帧记录为 `rendererQueueRejectLowerValue`。

"过期"的判定有分档：`render_frame_stale_slack` 按帧重要性给出不同的宽限时间（delta=4ms，reference=8ms，keyframe=12ms），重要帧有更长的等待窗口再被替换。

**`cadence_lag_ratio` 驱动的 sleep guard 动态调整**

`resolve_cadence_sleep_guard_override_ms` 在 cadence lag ≥ 55% 时把 sleep guard 压到 0（立即出帧），在 25%~55% 时压到半个刷新间隔，避免帧率落后时 Pacer 还在等待 sleep guard 到期。

#### 待优化项

**1. `catch_up_threshold` 固定为 500ms，不感知当前网络 RTT**

catch-up 阈值是 `host_frame_age_budget_ms`（来自 runtime stats）或默认 500ms。`host_frame_age_budget_ms` 由宿主机侧写入，计算公式为 `display_interval_ms × HOST_RENDER_FRAME_AGE_MULTIPLIER`，是纯本地显示器刷新率计算，不感知 RTT（已通过 Q4 采集日志确认，见第 3 节）。在云游戏高 RTT 场景下，500ms 的阈值可能导致正常的高延迟帧被误判为需要 catch-up 丢弃。

**2. `pacing_queue` 容量固定为 3**

ingress 最多缓冲 10 帧，但 pacing_queue 只有 3 帧。在 ingress 积压快速释放时，decode → pacer 的突发可能导致 pacing_queue 频繁触发 `queueCap` 丢帧。

---

### 2.6 Renderer（render/actor.rs）

这一层把 `DecodedFrame` 的像素数据写入宿主机渲染槽（`XbxRenderState.present_frame`），触发实际显示。入口是 `RendererActorHandle.submit()`，出口是宿主机 present 回调。

#### 现有能力

**actor 结构**

`RendererActorHandle` 持有容量为 1 的 `SyncSender<RendererMsg>`，渲染在独立线程 `XbxRendererActor` 里同步运行。容量为 1 意味着 Pacer 最多有一帧在 Renderer 的 mailbox 里等待，超出时 Pacer 进入 `render_backpressure_active` 状态。

**present 流程**

每帧到达时：
1. 记录 `rendered_at_ms`（系统时间）
2. 调用 `state.present_frame(render_frame)`
3. 若 `outcome.overwritten_previous_latest`，记录 `latestSlotOverwrite` 丢帧（render 层的覆写）
4. 若 present 失败，记录 `presentError` 丢帧并 warn log

**`latest_render_candidate_decision` 的状态追踪**

`XbxRenderState` 维护 `latest_render_candidate_decision`，记录最近一次 present 的决策（state/action/detail/frame_seq）。Renderer actor 在 decision_id 变化时同步到 runtime stats，供 recovery 诊断消费。

#### 待优化项

**1. Renderer mailbox 容量为 1，背压时无法区分慢和卡死**

Pacer 在 `render_backpressure_active` 时每 4ms 重试一次，没有超时机制。若 Renderer 线程因 `render_state.lock()` 阻塞（宿主机 present 回调耗时过长），Pacer 会持续等待，无法主动丢帧或上报 stall。`video_renderer_stalled` 的判定依赖 recovery 层的外部观测，不是 Renderer 自身上报。

值得注意的是，`render_state.lock()` 失败（mutex 中毒）也会触发 `video_renderer_drop_count_total` 计数并跳过当前帧，这条路径在正常运行中不会触发，但与 present 失败路径共用同一个 drop 计数器，无法从统计上区分。

**2. present 失败只记录 drop，不区分错误类型**

`present_frame` 返回 `Err` 时统一记录 `presentError` 并继续运行。没有连续失败计数，无法触发 decoder reset 或 reconnect 升级。
