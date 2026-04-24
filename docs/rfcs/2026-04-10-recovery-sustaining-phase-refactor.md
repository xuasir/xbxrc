# 恢复保活阶段重构 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: in-progress
- Owner: codex supervisor
- Last Updated: 2026-04-10

## Background

- 当前 `transportAwaitRecoveryKeyframe / bootstrapInFlight / soft reentry` 组合将“拿到 clean keyframe”近似视为“恢复已经基本完成”，只用一个很短的 `soft reentry` 窗口兜住恢复后抖动；这类旧 budget 式补丁已经不足以表达 clean anchor 之后仍需继续正式建链与保活的阶段语义。
- 运行日志显示，真实故障模式是“恢复锚点已出现，但参考链尚未稳定”，短暂恢复输出后会立即重新跌回 `referenceChainUnrecoverable / awaitingRecoveryKeyframe`。
- 现有状态机缺少“恢复保活阶段”这一正式阶段，导致 timeline/source、owner、coordinator 三层长期依赖临时窗口/grace 做局部补丁，无法表达“clean anchor 后先进入正式建链/保活阶段，失败时显式回退并重新请求关键帧，再回稳态”的策略。

## Goal

- 将恢复链从“等待关键帧”重构为“等待恢复链稳定成立”。
- 为媒体恢复引入正式的恢复保活阶段，使系统在拿到恢复锚点后优先保证连续可活，而不是立即按稳态策略判坏。
- 让 timeline/source、owner、coordinator、诊断输出统一理解该新阶段，避免短暂恢复后马上重回 `WaitKeyframe` / `referenceChainUnrecoverable`。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs`
  - `crates/xbxengine/core/src/media/video/ingress/scheduler.rs`
  - `crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`
  - 相关 diagnostics/stats、runtime stats sink 与测试
- Out of scope:
  - 新增独立 transport 路径或替代恢复协议
  - 重写 H264 inspection / NACK 基础机制
  - 前端 UI 文案大改，仅在必要时补诊断映射

## Plan

1. 在 timeline/source 层引入正式的恢复保活阶段；旧的 budget 语义彻底删除，仅保留必要的短窗观测作为阶段内部信号，不再承担 admission 配额职责。
2. 将 owner/coordinator 的 `bootstrapInFlight` 从“短时软护栏”升级为正式恢复阶段信号，并按阶段重写升级/抑制规则。
3. 更新 diagnostics/stats 与测试，验证“先活下来，再活得漂亮”的状态迁移。

## Validation

- [x] `cargo fmt --all`
- [x] 针对 `video_source::timeline` / `video_source::source` 的单测通过
- [x] 针对 `recovery::coordinator` / `session::policy` / `video_scheduling_owner` 的单测通过
- [ ] 日志/状态投影字段与新阶段语义一致

## Risks

- 新阶段若边界定义不清，可能把真实不可恢复链路错误地长期留在“保活”状态，延后必要重连。
- timeline/source 与 owner/coordinator 若状态不同步，可能出现 diagnostics 与实际恢复动作不一致。
- 当前工作区已有相关文件改动，集成时需要避免覆盖现有未提交修改。

## Progress

- [x] Step 1: timeline/source/scheduler 引入恢复保活阶段
- [x] Step 2: owner/coordinator/stats 接入新阶段语义
- [x] Step 3: 格式化、测试、文档回填

## Execution Notes

- Date: 2026-04-10 | Status: in-progress
- Update: 新建 RFC，确认本次问题不是“PLI 未发出”，而是“恢复后缺少正式保活阶段，短暂恢复后立刻再断链”。
- Decision: 采用“先活下来，再活得漂亮”的恢复链模型；`SOFT_REENTRY_BUDGET` 一类 submit/admission budget 已彻底删除，clean anchor 后统一进入正式建链/保活阶段，若建链失败则必须显式回退并重新请求关键帧。
- Risk/Blocker: 当前工作区已存在 recovery/source/timeline 相关未提交修改，后续实现需严格做增量改造并复查冲突点。
- Date: 2026-04-10 | Status: in-progress
- Update: 补齐 sustaining phase 语义下的 timeline/source/coordinator 测试期望；clean anchor 后统一先进入 `SustainingRecovery`，并接受 `ProbeKeyframe + RequestKeyframe` 的本地恢复动作投影。
- Validation: 已执行 `cargo fmt --all`、`cargo test --manifest-path crates/xbxengine/core/Cargo.toml video_source::timeline -- --nocapture`、`cargo test --manifest-path crates/xbxengine/core/Cargo.toml video_source::source -- --nocapture`、`cargo test --manifest-path crates/xbxengine/core/Cargo.toml recovery::coordinator -- --nocapture`、`cargo test --manifest-path crates/xbxengine/core/Cargo.toml session::policy -- --nocapture`、`cargo test --manifest-path crates/xbxengine/core/Cargo.toml policy::video_scheduling_owner -- --nocapture`。
- Date: 2026-04-10 | Status: in-progress
- Update: 已完成 Recovery Contract 一次切换主链改造：新增统一判据模块 `recovery/contract.rs`，owner/coordinator/session 统一改为硬门语义；`bootstrapInFlight` clean-break 为 `recoverySustaining`；`session policy` 已写入 `recovery_phase`、`recovery_exit_gate`、`recovery_ingress_waiting`、`recovery_transport_await_unresolved` 观测字段。
- Validation: `policy::video_scheduling_owner` 与 `recovery::coordinator` 目标测试已通过；`timeline` 测试仍在收口（主要为 `Healthy`/`SustainingRecovery` 历史期望混用导致的断言漂移），待统一期望后再更新 Completion。
- Date: 2026-04-10 | Status: in-progress
- Update: 已补 ingress 最后一处旧语义闸门：[`scheduler.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/ingress/scheduler.rs) 现在保持“冷启动必须 clean bootstrap”，但在恢复期若已存在 committed SPS/PPS、当前 codec/尺寸上下文稳定且 delta continuation 可承接，则允许退出 `waiting_keyframe` 并继续 `Submit`，把“先活下来”的语义真正下沉到 ingress 准入层。同步补了两条回归测试：`recovery_continuation_can_exit_waiting_keyframe_after_committed_bootstrap` 与 `cold_start_delta_without_committed_context_stays_in_wait_keyframe`。
- Decision: 恢复完成的唯一真值必须包含 ingress 真实退锁；owner/coordinator 继续保留较宽松的“恢复保活/可服务”判据，但不能再让 scheduler 停留在“只有 bootstrap_ready 才能活”的旧世界观。
- Risk/Blocker: 当前环境缺少 `cmake` / `pkg-config`，`cargo test -p xbxengine ...` 在 `audiopus_sys` 与 `ffmpeg-sys-next` 构建阶段失败；代码已 `cargo fmt --all`，但本轮 Rust 测试只能完成静态补点，无法在本机跑通。 
- Date: 2026-04-10 | Status: in-progress
- Update: 继续把“恢复保活”从“仍在等 keyframe”里剥离：[`contract.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/contract.rs) 现在把 `sustaining-recovery / recoverySustaining` 视为恢复保活态，不再把它当作 unresolved transport-await；[`video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs) 新增“恢复保活证据 -> ServingReady”收口，使 `RebuildingSupply` 在 clean anchor 后只要已有 serviceable output 就先退到 `DegradedServing`，避免播放器卡在启动期；[`coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 则允许 `recoverySustaining` 走 `BootstrapInFlight/WaitForBurst` 本地等待，而不是立刻再发一次 keyframe。
- Validation: `cargo fmt --all` 已通过；尝试执行 `cargo test -p xbxengine bootstrap_in_flight_signal_stays_in_local_probe_domain -- --nocapture` 与 `cargo test -p xbxengine sustaining_recovery_with_serviceable_output_exits_rebuilding_supply_as_degraded -- --nocapture`，仍因环境缺少 `cmake` / `pkg-config` 失败，分别阻塞在 `audiopus_sys` 与 `ffmpeg-sys-next` 构建阶段。
- Date: 2026-04-10 | Status: in-progress
- Update: 已继续收口真正的底层主闸门：[`timeline.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.rs) 现在不再把 `SustainingRecovery` 视作 `waiting_for_recovery_keyframe`；[`source.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs) 新增 `sustaining_recovery_continuation_allowed`，使 clean anchor 后、committed SPS/PPS 已在位且 `deltaContinuationReady=true` 的 healthy continuation 即便发生在 `first_frame_acquired` 之前，也不会再被旧 bootstrap 语义打回 `frame-inspection-rejected-await-anchor`。同时 source 侧已彻底删除 `SOFT_REENTRY_BUDGET`/`capacity=3` 这类 submit budget 语义；clean anchor 后进入正式建链/保活阶段，若 continuation 无法维持建链，则会显式回退到等待恢复关键帧并重新请求关键帧，而不是继续消耗所谓 3 帧预算。
- Validation: `cargo fmt --all` 已通过；尝试执行 `cargo test --manifest-path crates/xbxengine/core/Cargo.toml video_source::source -- --nocapture` 与 `cargo test --manifest-path crates/xbxengine/core/Cargo.toml video_source::timeline -- --nocapture`，仍因环境缺少 `cmake` / `pkg-config` 失败，阻塞在 `audiopus_sys` 与 `ffmpeg-sys-next` 构建阶段。
- Date: 2026-04-10 | Status: in-progress
- Update: 已补完 owner 对 sustaining 失败新事件的兼容：[`video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs) 现将 `frame-inspection-rejected-trigger-recovery-anchor` 统一纳入 startup/recovery 判据；[`source.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.test.rs) 新增 source 级回归，明确 sustaining 期间若 inspection reject，必须退出正式建链/保活阶段、重新落回 `waiting_for_recovery_keyframe` 并再次发出 `RecoveryKeyframeRequested`，不能卡死在“拿到过 clean anchor”后的半恢复状态。
- Validation: `cargo fmt --all` 已通过；本轮新增回归测试因本机仍缺少 `cmake` / `pkg-config` 无法完成 `cargo test -p xbxengine ...` 实跑，当前已知阻塞仍在 `audiopus_sys` 与 `ffmpeg-sys-next` 构建阶段。
