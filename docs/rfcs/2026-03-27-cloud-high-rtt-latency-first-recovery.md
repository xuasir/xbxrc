# Cloud 高 RTT 低延迟优先恢复 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 最新多份 Cloud runtime trace 已确认：标准 SDP 字段和 answer 接受项已基本对齐，但在 `RTT≈200ms + NACK` 场景下，视频链路仍会把大量“已无显示价值”的补包和坏帧继续送入后续阶段，导致 backlog 累积、`packet_to_present_ms` 偏高、主视频 track 推进异常。
- 现有恢复逻辑更偏向“尽量补齐画面”，主要止损点落在 decode/pacer/render 末端；这与 Cloud 高 RTT 下的低延迟目标冲突。当前需要把恢复目标切到“优先避免积压，尽快回到最新可显示画面”。

## Goal

- 为 Cloud 高 RTT 路径引入 `latency-first` 恢复模式，在仍启用 NACK 的前提下，优先跳过无价值补包和不可恢复帧，避免继续堆积到 decode/render。
- 把“恢复无望”的包级、帧级、链级状态显式建模并接入现有 runtime stats / trace projection，让后续 trace 能直接看出 `nack skipped`、`frame abandoned`、`wait-keyframe entered` 等关键决策。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/stream/nack_scheduler.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/adapter_types.rs`
  - `crates/xbxengine/core/src/media/video/ingress/scheduler.rs`
  - `crates/xbxengine/core/src/transport/rtc/pipeline/session_loop.rs`
  - `crates/xbxengine/core/src/transport/rtc/pipeline/observation.rs`
  - 现有 runtime stats / diagnostics / trace projection 链路中与 NACK / frame drop / recovery 相关的类型与投影
- Out of scope:
  - Home/直连默认恢复策略
  - 主动 decoder reset
  - 新建独立观测系统或新的 UI 主结构
  - 再次调 BWE/TWCC 参数作为主方案

## Plan

1. 在 NACK scheduler / video source 上实现 Cloud 高 RTT `latency-first` admission，按 deadline 与帧价值决定是否值得追包，并把 skipped/expired/late recovered 的恢复语义显式输出到 observation。
2. 在 frame assembly -> ingress 交界层引入 unrecoverable frame 语义，确保无恢复价值帧在 decode 前被丢弃，并在参考链污染时切到 `wait-keyframe`。
3. 补齐 runtime stats / trace projection / recovery 诊断字段，让 trace 能直接看到 `nackDisposition`、`frameRecoveryDisposition`、`estimatedRecoveryArrivalMs`、`framePlayoutDeadlineAtMs`、`frameUnrecoverableReason` 与 `recoveryStrategyMode=latency-first`。

## Validation

- [x] `cargo check -p xbxengine`
- [x] 新增或更新定点单测，覆盖高 RTT delta admission skip、expired reference -> wait-keyframe、unrecoverable frame 不进 decode
- [ ] 回归现有 TWCC / remoteTrack / cloud 相关测试，确认不回退

## Risks

- 当前恢复语义横跨 transport、ingress、diagnostics；如果边界处理不清，容易把新状态打散到多个模块并形成补丁式逻辑。
- `wait-keyframe` 现有恢复链会进一步升级到 decoder reset；本轮必须限制第一版只把主动作停在 wait-keyframe，避免意外放大恢复动作。

## Progress

- [x] Step 1: 已完成代码勘察，确认 NACK admission 主落点在 `nack_scheduler.rs` / `video_source/nack.rs`
- [x] Step 2: 已在 `nack_scheduler.rs` / `video_source/nack.rs` 实现 Cloud 高 RTT `latency-first` admission，并补齐 `skipped/expired/recovered` 的结构化 NACK observation 字段
- [x] Step 3: 已在 `ingress/scheduler.rs` / `pipeline/session_loop.rs` 实现 `DropUnrecoverable`、decode 前放弃与 `wait-keyframe` 接线
- [x] Step 4: 已补齐 runtime stats / protocol / trace projection 的新增字段，并完成 `cargo check -p xbxengine`、`cargo check -p xbxrc` 与 3 条定点测试
- [x] Step 4.5: 已在 `video_source` 真实组帧路径接入帧级恢复账本，把 `skipped/expired/recoveredLate` 的 Cloud 高 RTT 恢复结论按 `frame_rtp_timestamp` 映射为 `frame_playout_deadline_at_ms` / `frame_recovery_disposition` / `frame_unrecoverable_reason`，不再停留在 ingress 单测路径
- [x] Step 4.6: 已统一 `actualVideoKbps` 观测口径为“优先 local TWCC receive bitrate，否则回退 transport inbound video bitrate”，并新增 `frameRecoveryObserved` trace 事件承载 `ledgerWrite/ledgerConsume`
- [x] Step 4.7: 已补 `observabilitySnapshot.bitrate.actualVideoKbps` 镜像字段与 `observabilitySnapshot.latest.frameRecovery`，让 runtime trace 在同一 snapshot 上直接回看统一实际码率与最新帧恢复状态
- [ ] Step 5: 用新的 Cloud runtime trace 做真实回归，确认 backlog / present latency / wait-keyframe 行为符合预期

## Execution Notes

- Date: 2026-03-27 | Status: in-progress
- Update: 已确认本轮不再围绕 64 family、TWCC 反馈频率或 BWE 参数继续拧策略，而是前移放弃点，改造 Cloud 高 RTT 下的恢复语义。
- Decision: 第一版仅覆盖 Cloud 高 RTT 路径，最强动作限定为 `wait-keyframe`，不主动触发 decoder reset。
- Decision: 继续沿用现有 runtime stats / trace projection 链路新增结构化字段，不新起旁路观测系统。
- Update: 本轮已恢复并扩展 `nack_scheduler` 模块，打通 `PacketRecoveryDisposition` / `FrameRecoveryDisposition` / `estimatedRecoveryArrivalMs` / `framePlayoutDeadlineAtMs` / `frameUnrecoverableReason` 到 `runtime stats -> protocol dto -> trace projection`。
- Update: `video_source` 现已维护按 `frame_rtp_timestamp` 的恢复账本，并在真实 `AssembledVideoFrame` 出口写入 `frame_playout_deadline_at_ms`、`frame_recovery_disposition`、`frame_unrecoverable_reason`；Cloud 高 RTT 下 `delta` 会映射为 `UnrecoverableLate`，`reference/keyframe` 会映射为 `UnrecoverableReferenceChain`。
- Update: `actualVideoKbps` 对外观测口径已收敛到单一来源链路：`diagnostics/stats` 与 `observation_bus` 都优先取 `latest_video_twcc_observation(source=local-feedback).receive_bitrate_kbps`，否则回退 transport metrics 的 `inbound_video_bitrate_kbps`；`trace_projection` 的 `bweUpdated.actualVideoBitrateKbps` 也已改为直接输出顶层 `stats.video_actual_bitrate_kbps`，避免和 `observabilitySnapshot.bwe.actualVideoKbps` 长期分叉。
- Update: 真实帧级恢复账本现已通过 `observation -> runtime stats -> protocol dto -> trace projection` 原链路新增 `frameRecoveryObserved` 事件，可直接在 runtime trace 中检索 `action=ledgerWrite/ledgerConsume` 来验证真实路径是否命中。
- Update: 为了避免继续靠事件和嵌套结构反推，`observabilitySnapshot` 现已额外镜像 `bitrate.actualVideoKbps`，并把最新 `frameRecovery` 放入 `latest.frameRecovery`；后续核对同一时刻 snapshot 时不需要再手工拼 `statsSnapshot + frameRecoveryObserved`。
- Update: `cargo check -p xbxengine`、`cargo check -p xbxrc` 已通过；定点验证已通过 `skipped_too_late_does_not_enter_pending`、`unrecoverable_late_frame_is_dropped_before_decode`、`unrecoverable_reference_chain_enters_wait_keyframe_path`。
- Risk/Blocker: 当前主阻塞已从“真实路径未打通”收敛为“缺新的 Cloud trace 端到端回归”；在完成 backlog / `packet_to_present_ms` / `wait-keyframe` 行为回归前，还不能宣称低延迟目标已在真实链路完全收敛。
