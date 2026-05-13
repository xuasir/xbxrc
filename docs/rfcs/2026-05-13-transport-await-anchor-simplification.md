# Transport Await-Anchor Simplification RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成（核心路径已落地；线上 cloud trace 复核仍建议按 Validation 清单跑一轮）
- Current State: implemented
- Owner: Codex Supervisor
- Last Updated: 2026-05-13

## Background

- 最新 cloud runtime trace 表明，系统在 transport 仍持续收包、TWCC 仍有健康信号时，仍会长期停在 `transportAwaitRecoveryAnchor`，并反复落到 `coalesced:keyframeInFlight`。
- 当前链路里，显示供给压力、锚点缺失、episode 在飞、连接域升级这四类事实仍然靠得太近：
  - `present_age / decode_age / no_pending` 很容易把 owner 推入恢复叙事。
  - `transportAwaitRecoveryAnchor` 同时承载“显示吃紧”“等待 usable IDR”“等待 clean anchor”“等待 display stable”多层语义。
  - `keyframe in-flight` 主要表达“还有一个 episode 在飞”，缺少“这个 episode 还有没有价值”的健康度表达。
  - 本地显示问题虽然最终没有直接抬成 reconnect，但会过早借道 `PLI/FIR` 控制动作打到远端。
- 现有 RFC 已经确定几条长期约束：
  - `display` 域事实只负责本地保供给与完成判据，不能继续主导 recovery 主动作。
  - `transportAwaitRecoveryAnchor` 属于 local recovery 域，reconnect 继续只留给 connectivity 硬证据。
  - `PLI` 是图片级恢复主路径，`FIR` 是长时间缺锚点时的重保底。

## Goal

- 把恢复判定收成少量主状态，压住判定域复杂度。
- 让显示供给压力先停留在本地怀疑层，只有明确锚点证据时才进入 `AwaitAnchor`。
- 让长时间拿不到 usable IDR 的 episode 更快结束 `coalesced:keyframeInFlight` 占位，继续刷新 `PLI` 或升级 `FIR`。
- 明确区分三类现场：
  - 本地显示吃紧
  - 远端持续回 continuation 但不给 usable IDR
  - 连接域真实恶化

## Scope

- In scope:
  - [`crates/xbxengine/core/src/transport/rtc/session/facts.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/facts.rs)
  - [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs)
  - [`crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs)
  - [`crates/xbxengine/core/src/runtime_stats_sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/runtime_stats_sink.rs)
  - [`crates/xbxengine/core/src/transport/rtc/session/expensive_recovery_gate.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/expensive_recovery_gate.rs)
  - 相关 trace / diagnostics / tests
- Out of scope:
  - 改远端编码器行为
  - 放宽 H264 对 usable IDR / clean anchor 的硬语义
  - 新增平行 transport 或新的 recovery 动作集合
  - 重做渲染主线

## Design

### 1. 主状态只保留五个

- `Stable`
- `Suspect`
- `AwaitAnchor`
- `LocalRecovery`
- `ConnectivityRecovery`

约束：

- 状态机只允许这五个主状态参与动作分叉。
- 其余细节全部下沉为 observation/tag，不新增平行顶层 lane。
- `ConnectivityRecovery` 继续只由连接域硬证据触发。

### 2. 把“显示压力”和“锚点缺失”拆成两段

当前问题是 `present_age / decode_age / no_pending` 太容易直接进入 `transportAwaitRecoveryAnchor`。

目标改造：

1. `Stable -> Suspect`
   - 只由显示供给压力触发。
   - 不发 `PLI/FIR`。
   - 优先吸收恢复爬升余波、短时 host no-pending、短时 decode/present 失配。

2. `Suspect -> AwaitAnchor`
   - 必须同时满足锚点证据：
     - `bootstrapMissingIdr`
     - 当前 recovery epoch 持续无 clean anchor
     - decoder 进入 waiting-keyframe
     - anchor candidate reject 持续出现
   - 必须再满足短窗内 decode/present 没有有效前进。

这样显示吃紧只负责“怀疑”，锚点证据才负责“进入等待 IDR 语境”。

#### 2.1 `Suspect` 停留合同

`Suspect` 的职责是吸收“显示供给吃紧，但恢复锚点缺失还没有被证明”的窗口。

进入 `Suspect` 后必须满足以下规则：

- 只要仍满足任一条件，继续停留在 `Suspect`
  - decode 或 present 在最近 fresh 窗口内仍有前进
  - 当前 recovery epoch 已有 clean anchor
  - 当前 observation 只体现 display pressure，没有 fresh anchor blocker
  - 当前仍处于 post-anchor 爬升窗
- 只有同时满足以下条件，才允许升级到 `AwaitAnchor`
  - `Suspect` 停留超过最小 dwell 窗口
  - 最近一个短窗内 decode/present 都没有新前进
  - 当前 recovery epoch 仍缺 clean anchor
  - 出现 fresh 的 `anchor_evidence`

推荐 dwell 窗口直接复用既有 profile 的恢复进展 freshness，避免再造一套新时钟：

- `HomeLanGaming`: 180ms
- `RelayGaming`: 240ms
- `CloudGaming`: 320ms

理由：

- 这组值已经在 [`policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/policy.rs) 里作为 `playback_recovered_track_progress_fresh_ms` 存在，现有系统已经拿它表达“轨道进展是否仍然新鲜”。
- 过往文档已经反复确认短抖动、display pressure、clean-anchor 后余波需要先吸收，尤其 cloud 侧需要更宽的观察窗。
- 这样 `Suspect` 不会因为单拍 no-pending、短 present lag、renderer shadow stall 直接升级成 `AwaitAnchor`。

### 3. `AwaitAnchor` 里的 episode 只保留三档健康度

当前 `keyframe in-flight` 更接近布尔占位，导致 `coalesced:keyframeInFlight` 容易持有过久。

目标改造：

- `WaitingResponse`
  - 已发 `PLI/FIR`
  - 还没有看到有效回流
- `ContinuationOnly`
  - 已有 packet/inspection 前进
  - 仍持续停在 `bootstrapMissingIdr / NonIdrVcl`
- `Stalled`
  - packet、decode、present、clean-anchor 都没有形成新推进
  - 当前 in-flight episode 已失去继续占位价值

动作规则固定为：

1. `WaitingResponse` 超过短 refresh 窗口，重开 `PLI`
2. `ContinuationOnly` 持续，升级 `FIR`
3. `Stalled` 持续，结束当前 in-flight 占位，转入 `LocalRecovery`

约束：

- `coalesced:keyframeInFlight` 只保留给 `WaitingResponse`
- `ContinuationOnly` 与 `Stalled` 必须允许 `Refresh` 或更重本地动作

#### 3.1 `ContinuationOnly -> FIR` 与 `Stalled -> release in-flight` 时窗

这两组时窗优先复用现有 profile 参数和历史 trace 结论。

##### A. `ContinuationOnly -> FIR`

推荐规则：

- 必须先经历至少 1 次 `PLI`
- 必须至少经过 1 次 `PLI refresh`
- 当前 episode 持续命中：
  - `continuationAcceptedWhileAwaitingIdr`
  - `bootstrapMissingIdr` 或 `NonIdrVcl`
  - packet recent
  - decode/present 仍停在旧输出
- 满足以上条件后，达到 `fir_retry_interval_ms` 即可升级 `FIR`

推荐值直接沿用现有 profile：

- `HomeLanGaming`: 260ms
- `RelayGaming`: 360ms
- `CloudGaming`: 420ms

理由：

- 这组值已经在 [`policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/policy.rs) 中存在，并且明确高于 `pli_refresh_interval_ms`。
- 过往 `continuation-heavy` 黑盒合同已经证明：首拍 `PLI`、短窗 `PLI refresh`、再升级 `FIR` 是稳定主线。
- cloud trace 长期存在“持续收包但只有 continuation”的场景，420ms 足够给一轮 `PLI + refresh`，又不会把 in-flight 长时间挂死。

##### B. `Stalled -> release in-flight`

推荐规则：

- `Stalled` 不是“完全无包”，而是“当前 episode 对恢复主线已经没有正向推进价值”
- 满足以下任一组即可释放当前 in-flight：
  - `requested/sent` 之后，超过 `decoded_pending_commit_hold_ms` 仍无 decode、present、clean-anchor 前进
  - `response-observed / packet-seen / decoded` 后，超过 `decoded_pending_commit_hold_ms` 仍未形成 clean-anchor 或新的播放前进
  - 当前 blocker 持续停在 continuation-only / invalid bootstrap，且最近一个 `fir_retry_interval_ms` 内没有更高质量进展

推荐值优先沿用 `decoded_pending_commit_hold_ms`：

- `HomeLanGaming`: 180ms
- `RelayGaming`: 240ms
- `CloudGaming`: 320ms

补充约束：

- `requested but unsent` 继续沿用既有 220ms unsent grace，超窗后直接终结为 `expired-unsent`
- `FIR` 已经发出时，不在每个 tick 重复释放；必须等 `fir_retry_interval_ms` 或 episode 明确终态

理由：

- 过往文档已经确认 continuation-only response 不能长期占住 same-family keyframe in-flight 门位。
- 现有 profile 里的 `decoded_pending_commit_hold_ms` 本来就在表达“decoded/packet-seen 之后还能再等多久”；直接复用可以保证行为和现有 replay/commit 合同一致。
- 这组值也与 `playback_recovered_track_progress_fresh_ms` 成对出现，能把“短暂恢复余波”和“episode 已经失去价值”区分开。

### 4. reconnect 边界保持不变，控制动作边界继续收紧

继续保留当前大方向：

- `AwaitAnchor` 属于 local domain
- `PLI/FIR` 属于 transport control action
- `Reconnect` 属于 connectivity domain

新增明确边界：

- RTP bytes、video packets、TWCC、feedback target 仍持续前进时，系统只能停留在 `AwaitAnchor` 或 `LocalRecovery`
- 只有连接 freshness、feedback availability、transport deadline 证据共同恶化时，才允许进入 `ConnectivityRecovery`

### 5. 观测层只补轻量标签，不扩顶层 reason 域

为了压住复杂度，主状态之外只补 4 个字段：

- `owner_surface_state`
  - `stable`
  - `suspect`
  - `await-anchor`
  - `local-recovery`
  - `connectivity-recovery`
- `anchor_evidence`
  - `none`
  - `bootstrapMissingIdr`
  - `decoderWaitingKeyframe`
  - `anchorReject`
- `keyframe_episode_health`
  - `waiting-response`
  - `continuation-only`
  - `stalled`
- `recovery_escalation_basis`
  - `local_supply`
  - `anchor_missing`
  - `connectivity_bad`

约束：

- 这些字段只服务 trace、diagnostics、test contract。
- 顶层 `VideoEscalationReason` 不新增大批枚举。
- `runtime_reason_domain` 继续保持 `Local / ConnectivityTransport` 两域。

### 6. 额外高风险入口也要收口到 `Suspect`

除了显示供给压力，当前还有几条入口会把系统过快推入 IDR 依赖态。这些入口都应先停在 `Suspect`，而不是直接落到 `AwaitAnchor` 或 `WaitKeyframe` 主叙事。

#### 6.1 bootstrap reject 直映射入口

当前：

- `bootstrapMissingSps`
- `bootstrapMissingPps`
- `inspectionRejectInvalidSliceHeader`

会直接映射成 `TransportAwaitRecoveryKeyframe`。

问题：

- 这组信号里既有“确实缺恢复锚点”的场景，也有首帧期、参数切换期、局部 bootstrap 检查失败的场景。
- 直接映射会让系统在证据还不完整时就进入 `AwaitAnchor` 叙事。

目标改造：

- 这三类信号先只写成 `anchor_evidence`
- 默认只触发 `Suspect`
- 只有与“当前 epoch 无 clean anchor + decode/present 停滞 + dwell 超窗”组合成立时，才允许升级到 `AwaitAnchor`

#### 6.2 ingress `WaitKeyframe` 入口过宽

当前 ingress 会在以下情况直接返回 `WaitKeyframe`：

- `ingress_awaiting_bootstrap`
- `config_changed`
- `config_mismatch`
- dropping 期间的 `prefers_wait_keyframe()`

问题：

- `config_changed/config_mismatch` 更接近“当前参数上下文未收敛”或“局部 admission 不满足”
- 这组情况并不天然等价于“已经需要新的恢复锚点”

目标改造：

- `config_changed/config_mismatch` 优先停留在 `Suspect` 或 `Reconfigure`
- 只有 `hard_recovery_gap_risk` 持续成立时，才升级到 `AwaitAnchor`
- 首帧期 `ingress_awaiting_bootstrap` 继续保留保护，但不提前宣布 steady-state 的 IDR 依赖态

#### 6.3 `frameAbandoned / DropUnrecoverable` 被直接贴成 `WaitKeyframe`

当前 `DropUnrecoverable` 会直接映射成 `WaitKeyframe` 家族。

问题：

- 单帧局部不可恢复不等价于全链已经进入“等关键帧”语义
- 这条映射会把 frame-local 失败放大成 episode 级恢复压力

目标改造：

- `frame-local unrecoverable` 先停留在 `Suspect`
- 只有连续 frame abandon，且同时伴随 fresh `anchor_evidence`，才升级到 `AwaitAnchor`

#### 6.4 owner 在 `RebuildingSupply` 下默认贴 `transportAwaitRecoveryAnchor`

当前 owner 只要处于 `RebuildingSupply`，就很容易带着 `transportAwaitRecoveryAnchor` 标签往下游走。

问题：

- `RebuildingSupply` 表达的是“供给正在重建”
- 它本身不等价于“当前已经确认缺恢复锚点”

目标改造：

- `RebuildingSupply + 无强 anchor evidence` 输出 `Suspect`
- `RebuildingSupply + 强 anchor evidence` 才输出 `AwaitAnchor`

#### 6.5 transport observation 的 `AwaitRecoveryKeyframe` 直出 label

当前 admission/loss 两路 observation 只要给出 `AwaitRecoveryKeyframe`，label 就会直接写成 `transportAwaitRecoveryAnchor`。

问题：

- 这保留了底层 observation 的原始语义
- 但缺少“局部修补升级”与“已确认缺恢复锚点”的二次门

目标改造：

- 保留底层 observation label
- 在 owner / session policy 层先统一落到 `Suspect`
- 再由事实门决定是否真正升级为 `AwaitAnchor`

#### 6.6 优先级

实现优先级固定为：

1. bootstrap reject 直映射入口
2. `DropUnrecoverable -> WaitKeyframe`
3. owner `RebuildingSupply` 默认贴 `transportAwaitRecoveryAnchor`
4. ingress `config_changed/config_mismatch -> WaitKeyframe`
5. transport observation 的 `AwaitRecoveryKeyframe` 二次门

## Plan

1. 收口 owner 入口
   - 在 `facts.rs` / `session::policy` 明确 `Stable -> Suspect -> AwaitAnchor` 两段入口
   - 让显示供给压力只负责进入 `Suspect`
   - 让 bootstrap reject、frame abandon、owner rebuilding-supply、transport await observation 也先统一进入 `Suspect`
2. 收口 in-flight 语义
   - 在 `recovery/coordinator` 为 `transportAwaitRecoveryAnchor` episode 增加 `waiting-response / continuation-only / stalled`
   - 改写 `coalesced:keyframeInFlight` 的解锁和 refresh 触发
3. 收口观测与域边界
   - 在 `runtime_stats_sink`、trace、tests 补齐轻量标签
   - 保持 reconnect gate 只认 connectivity 硬证据

## Validation

- [x] `display pressure` 单独出现时只进入 `Suspect`，不直接发 `PLI/FIR`（`session::policy` + coordinator 合同；见 `cargo test -p xbxengine --lib`）
- [x] `Suspect` 在 dwell 窗口内只吸收显示供给压力，不因为单拍无 pending / 短 present lag 直接升级（`suspect_anchor_gate` + profile `playback_recovered_track_progress_fresh_ms`）
- [x] `bootstrapMissingSps/Pps/inspectionRejectInvalidSliceHeader` 默认只进入 `Suspect`（`connectivity_reason` / `observation` 标签与 `LocalSupplySuspect` 映射）
- [x] `DropUnrecoverable/frameAbandoned` 单次出现时不直接升级成 `AwaitAnchor`（ingress / session_loop 先 suspect 路径）
- [x] `RebuildingSupply` 在无强 anchor evidence 时只输出 `Suspect`（`video_scheduling_owner` 分支）
- [x] ingress `config_changed/config_mismatch` 不直接把 steady-state 拉进 IDR 依赖态（先 suspect，`hard_recovery_gap_risk` 再升 await-anchor）
- [x] `bootstrapMissingIdr + decode/present 无前进` 时进入 `AwaitAnchor`（升级门 `upgrade_local_supply_suspect_signal_if_ready`）
- [x] `continuation-only` 长时间持续时不会长期停在 `coalesced:keyframeInFlight`（`coalesced_transport_await_should_unlock_for_stall` + continuation PLI/FIR 时钟）
- [x] `ContinuationOnly` 在 `fir_retry_interval_ms` 内最多经历一轮 `PLI refresh` 后升级 `FIR`（`should_upgrade_transport_await_refresh_to_fir`）
- [x] `Stalled` 在 `decoded_pending_commit_hold_ms` 超窗后释放 in-flight，不继续长期占住 keyframe family（同上 stall 解锁路径）
- [x] transport 持续收包但无 usable IDR 时，系统停留在 local recovery 域（`resolve_runtime_reconnect_reason_domain` / expensive gate 回归单测）
- [x] 只有 connectivity 硬证据成立时才放行 reconnect（既有矩阵单测 + 本 RFC 未改 connectivity 判定入口）
- [x] trace 能直接回答“为什么进入 await-anchor”“为什么仍在 in-flight”“为什么没有 reconnect”（`XbxEngineMediaRuntimeStats` 四标签 + `record_recovery_decision_ledger`；host stall 下 `cleanAnchorInvalidatedAwaitingIdrHostStall` 见 `runtime_port::update_host_video_present_metrics`）

### 与 05-12 / 04-29 RFC 的术语对照（别名，不引入平行顶层阶段）

| 本 RFC 主状态 / 标签 | 代码与 trace 中的主要落点 |
| --- | --- |
| `Suspect` | `VideoEscalationReason::LocalSupplySuspect`、`recovery_owner_surface_state` / `recovery_escalation_basis` |
| `AwaitAnchor` | `VideoEscalationReason::TransportAwaitRecoveryKeyframe`、`transportAwaitRecoveryAnchor`（reason label） |
| `anchor_evidence` | `recovery_anchor_evidence`（`suspect_anchor_gate::recovery_anchor_evidence_trace_code`） |
| `keyframe_episode_health` | `recovery_keyframe_episode_health`（`WaitingResponse` / `continuation-only` / `stalled` 字符串档） |
| `SessionRecoveryStage` / `VideoEscalationReason::label()` | 不替换；本 RFC 五主状态为 **await-anchor 收口** 的决策语言，与 04-29「单线收敛」阶段字符串并存时以上表为准对齐 |

## Risks

- `Suspect` 吸收窗口过宽，会延迟真正的 `PLI`
- `ContinuationOnly -> FIR` 过于积极，会放大远端编码压力
- trace 标签与主状态机脱节，会再次形成“观测说一套、动作做一套”
- 直接复用现有 profile 时钟会把旧语义一并带入；实现阶段需要用新 trace 再核一次 cloud 窗口
- 入口收口不完整时，旧的 `WaitKeyframe / transportAwaitRecoveryAnchor` 直映射仍会绕过 `Suspect`

## Progress

- [x] Step 1: 入口状态从 `display pressure -> await-anchor` 收成 `display pressure -> suspect -> await-anchor`
- [x] Step 2: `keyframe in-flight` 引入三档健康度并重写 refresh / unlock 规则
- [x] Step 3: trace / diagnostics / tests 对齐新的主状态与辅助标签（stats 四字段写入 `policy` / ledger；`runtime_stats_sink::invalidate_current_transport_clean_anchor` 在 host visibility stall + awaiting-IDR 时落盘；`cargo test -p xbxengine --lib` 全绿）

## Execution Notes

- Date: 2026-05-13 | Status: implemented
- Update: 新建 RFC，收口 `transportAwaitRecoveryAnchor` 的入口、episode 健康度、以及 reconnect 边界。
- Decision: 判定域只保留 `Stable / Suspect / AwaitAnchor / LocalRecovery / ConnectivityRecovery` 五个主状态，复杂细节下沉为轻量标签。
- Decision: 显示供给压力只负责进入 `Suspect`；锚点证据负责进入 `AwaitAnchor`；connectivity 硬证据负责进入 `ConnectivityRecovery`。
- Decision: `Suspect` 的 dwell、`Stalled` 的 release 优先复用 `playback_recovered_track_progress_fresh_ms` 与 `decoded_pending_commit_hold_ms`；`ContinuationOnly -> FIR` 优先复用 `fir_retry_interval_ms`，避免额外引入新时钟。
- Decision: `bootstrap reject`、`frame abandon`、`RebuildingSupply`、`ingress WaitKeyframe`、`transport await observation` 这些高风险入口统一先经 `Suspect`，只有事实门成立时才进入 `AwaitAnchor`。
- Risk/Blocker: 需要和现有 `2026-05-12-transport-repair-and-recovery-semantic-unification`、`2026-04-29-playback-recovery-single-line-convergence` 两份 RFC 保持术语一致，避免再次引入平行阶段语言。
- Update: `runtime_port` 在 host visibility stall 下作废 clean anchor 时，要求 `latest_host_present_time_ms` 与 wall `now_ms` 同域（`>= 1e12`），避免脚本时间线单测把 stall 误判为真 stall。
