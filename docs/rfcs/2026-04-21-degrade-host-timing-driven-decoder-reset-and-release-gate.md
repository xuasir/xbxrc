# 退化 Host Timing 驱动的 Decoder Reset 与 Release Gate RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: in-progress
- Owner: Codex
- Last Updated: 2026-04-21

## Background

- 新 trace [`runtime-trace-1776737338599-1.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1776737338599-1.jsonl) 已确认消费链可以打通，`renderer submit -> host accepted -> prepare_sample_ready -> sample_presented` 连续成功。
- 恢复刚显示两帧后，`external-decoder-reset-requested` 触发本地 decoder reset，decode 端随即回到 `waiting-keyframe`，并进入连续 `bootstrap-gate-rejected / NonIdrVcl`，供给再次枯竭。
- 当前控制环把 host timing 动态信号同时喂给 local decoder reset 和 pacer release gate，恢复期正常抖动会被放大成供给中断。

## Goal

- 让 host timing 保持观测信号身份，退出 decoder reset 与 release gate 的控制决策。
- 让 pacer 回到更固定的 cadence 提交，优先维持恢复后的连续供给。
- 保留 renderer latest-slot、host bounded queue、host telemetry 与 queue pressure 的现有保护。

## Scope

- In scope:
  - [`crates/xbxengine/core/src/transport/rtc/stack/runtime_port.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack/runtime_port.rs)
  - [`crates/xbxengine/core/src/media/video/decode/video_decode.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/video_decode.rs)
  - [`crates/xbxengine/core/src/media/video/decode/video_decode.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/decode/video_decode.test.rs)
  - [`crates/xbxengine/core/src/media/video/pacer/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/pacer/actor.rs)
  - [`crates/xbxengine/core/src/media/video/pacer/actor.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/pacer/actor.test.rs)
  - [`docs/project-task.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/project-task.md)
- Out of scope:
  - render / host 架构重写
  - latest-slot 与 host bounded queue 结构调整
  - recovery owner / episode / value 模型重写

## Plan

1. 新增 RFC 与任务跟踪，固定本轮退化边界与验证口径。
2. 让 `update_host_video_timing()` 只写入 host timing 与 timing shift 观测，不再触发 local decoder reset。
3. 让 pacer 停止等待 host release gate，回到固定 cadence 提交，并更新测试覆盖。
4. 把 decoder continuation no-output 恢复收口到 decoder 本地：`Nominal + NonIdrVcl + backendNoOutput` 连续成立时，由 decoder 自己执行 soft fallback / local reset。
5. 给 recovery burst 增加本地保供给退化：decode queue 对 `window_source=recovery` 放宽容量与 stale slack，pacer 在 recovery burst 期间退出 `Priming/Starved` 动态 queue 收紧。
6. 跑定向测试，确认 release gate 与 host timing reset 已退出控制环，decoder 已接管 continuation no-output 坏链，recovery burst 不再被 decode/pacer 首层限流直接吞掉。

## Validation

- [x] `cargo test -p xbxengine runtime_port -- --nocapture`
- [x] `cargo test -p xbxengine media::video::pacer -- --nocapture`
- [ ] `cargo test -p xbxrc native_video::scheduling::tests`
- [x] `cargo test -p xbxengine video_decode -- --nocapture`
- [x] `cargo check -p xbxengine`

## Risks

- host release gate 退出后，latest-slot overwrite 与 queue pressure 可能升高，需要继续观察新 trace。
- 固定 cadence 提交会把更多保护责任压回 queue pressure 和 native host bounded queue，需要确认没有新的 presenter 压力尖峰。
- decoder continuation no-output 恢复阈值偏紧时，短时 backend 抖动会更早切换到 software / reset，需要用新 trace 观察误触发率。

## Progress

- [x] Step 1: 已从 trace 与代码确认主坏链位于 host timing 控制环。
- [x] Step 2: `update_host_video_timing()` 已只保留 timing shift 观测，退出 local decoder reset 控制。
- [x] Step 3: pacer host release gate 已退化为固定 cadence 提交，相关测试已更新。
- [x] Step 4: decoder-owned continuation no-output 恢复与测试已完成。
- [x] Step 5: recovery burst 保供给退化已完成，decode queue 与 pacer queue 对 `window_source=recovery` 放宽。
- [ ] Step 6: 等待 `xbxrc` host scheduling 定向验证与新 trace 复核。

## Execution Notes

- Date: 2026-04-21 | Status: in-progress
- Update: 新建本轮 RFC，按方案 2 退化 host timing 驱动的 decoder reset 与 pacer release gate。
- Decision: 本轮优先缩短控制环，只保留 host timing 的观测价值和 queue pressure 的有限保护。
- Risk/Blocker: 若 fixed cadence 提交后 overwrite 快速升高，需要继续收紧 renderer / host 衔接策略。
- Date: 2026-04-21 | Status: in-progress
- Update: `runtime_port` 已改为记录 `videoHostTimingShiftObserved`，不再发送 local decoder reset；`pacer` 已退出 host release gate，`next_wait_duration` 与 `drive_ready_frames` 统一回到 deadline/cadence 驱动。
- Decision: 保留 `resolve_host_release_wait_duration()` 兼容接口和 host telemetry 字段，方便下一轮 trace 对比与更大范围的 host/presentation 退化设计。
- Date: 2026-04-21 | Status: in-progress
- Update: 新 trace [`runtime-trace-1776740205734-1.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1776740205734-1.jsonl) 显示 host/pacer 已退出主坏链，尾段故障前移到 decoder continuation 消费：`cleanAnchorCommitted` 后连续 `backend-no-output + bootstrapReject=NonIdrVcl`，host 仅 retained 旧帧。
- Decision: continuation no-output 恢复由 decoder 自治处理，优先用本地证据 `latestDecodedSeq + backendNoOutputStreak + inputFramesSinceLastDecoded + NonIdrVcl` 触发 soft fallback / reset。
- Date: 2026-04-21 | Status: in-progress
- Update: 新 trace [`runtime-trace-1776742790364-1.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1776742790364-1.jsonl) 显示坏链继续前移到 post-decode 消费层；本轮已把 `window_source=recovery` 作为局部信号，decode queue 对 recovery burst 放宽到 5 帧并增加 stale slack，pacer 在 recovery burst 期间保持默认 queue cap，不再随 `Priming/Starved` 收紧。
- Decision: 恢复窗口放宽只绑定 `budget.window_source=Recovery`，避免 `FrameRecoveryDisposition::Repairing` 的宽覆盖面把常态 playout 路径一起放大。
- Validation: `cargo test -p xbxengine video_decode -- --nocapture`、`cargo test -p xbxengine media::video::pacer -- --nocapture`、`cargo check -p xbxengine` 已通过。
- Date: 2026-04-21 | Status: in-progress
- Update: 针对新 trace 中持续出现的 `decode:drop:outputQueueOverflow` 与 `pacer:drop:queuePressure`，进一步收紧“何时背压”和放宽“recovery 本地缓冲深度”两条节奏控制：decode 仅在输出队列打满时才切到 `PullOutputFirst`；pacer 的 `window_source=recovery` 本地缓冲上限提升到 5，并让普通 `queuePressure` 先由本地缓冲吸收，保留 aggressive pressure 的快速收紧。
- Decision: `window_source=recovery` 继续作为唯一放宽信号；常态 playout 仍保持 decode 3 帧、pacer 3 帧和既有 pressure 策略，避免把常态路径一起放大。
- Validation: `cargo test -p xbxengine workload_snapshot_keeps_accepting_input_until_output_queue_is_full -- --nocapture`、`cargo test -p xbxengine recovery_window_frames_allow_deeper_local_buffer_before_release -- --nocapture`、`cargo test -p xbxengine recovery_window_frames_bypass_non_aggressive_queue_pressure_tightening -- --nocapture`、`cargo test -p xbxengine video_decode -- --nocapture`、`cargo test -p xbxengine media::video::pacer -- --nocapture` 已通过。
