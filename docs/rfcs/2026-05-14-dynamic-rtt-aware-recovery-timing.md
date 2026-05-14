# 动态 RTT 感知恢复时序 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 部分完成（核心 timing / NACK 首发与 survival / trace 字段已落地；runtime trace 实网回归仍为待办）
- Current State: implemented (core)
- Owner: Codex / rtc recovery
- Last Updated: 2026-05-14

## Background

- 当前恢复时序里仍存在多处固定阈值与静态 profile：
  - `nack_timeout_ms`
  - `pli_refresh_interval_ms`
  - `fir_retry_interval_ms`
  - `nack_retry_interval_ms`
  - 部分 `decoded_pending_commit_hold_ms`
- 现网目标链路 RTT 差异很大：
  - Home LAN 常见约 `10ms`
  - Home WAN 常见约 `100ms`
  - Cloud Gaming 常见约 `200ms`
- 在这种 RTT 分布下，固定时序会稳定制造两类偏差：
  - 低 RTT 场景被保守阈值拖慢，恢复动作滞后；
  - 高 RTT 场景被短阈值误判，`NACK` 还未自然闭环就过早升级到 `PLI/FIR`。
- 当前系统还存在三条耦合过紧的路径：
  - `NACK` 生产主路径近似单发，且首发等待窗偏紧；很多高价值缺口还没走完首轮自然闭环就被统计为 miss；
  - `PLI` 已部分承担上层状态机节拍职责，导致 `IDR` 压力偏大；
  - `FIR` 在 `transportAwaitRecoveryAnchor` 路径里仍偏重，距离浏览器常见接收恢复主线偏远。
- 此外，`H264 bootstrapMissingSps/bootstrapMissingPps` 仍过早向 recovery 主链上抬，缺少 codec/depacketizer 层的窄补救。

相关现状代码：

- [`crates/xbxengine/core/src/transport/rtc/recovery/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/policy.rs)
- [`crates/xbxengine/core/src/transport/rtc/recovery/state_machine.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/state_machine.rs)
- [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs)
- [`crates/xbxengine/core/src/transport/rtc/connection/service.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/connection/service.rs)
- [`crates/xbxengine/core/src/transport/rtc/stack/media_pipeline.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack/media_pipeline.rs)
- [`crates/xbxengine/core/src/transport/rtc/stream/nack_scheduler.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/nack_scheduler.rs)
- [`crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs)

## Goal

- 把恢复时序从“静态档位 + 分散条件”收成“动态 RTT 感知 + 场景只提供边界参数”的单一模型。
- 保留低延迟优先，但避免高 RTT 场景过早升级到 `PLI/FIR/reconnect`。
- 将 `NACK` 主策略从“先发短窗 + 补一次重试”收成“首发等待窗按价值与 RTT 放宽优先，重试只做窄补充”。
- 将 `NACK`、`PLI`、`FIR`、`H264 bootstrap salvage` 四层职责重新拆开：
  - `NACK` 负责包级本地修复；
  - `PLI` 负责图片级恢复主路径；
  - `FIR` 负责受控发送端上的重保底；
  - `codec/depacketizer salvage` 负责可闭合的参数集补救。
- 把“缺包太多直接要 keyframe”从数量阈值改为“高价值影响阈值 + repairability 阈值”。

## Scope

- In scope:
  - `recovery/policy.rs` 中恢复 timing 参数结构
  - `session/policy.rs` 中 `PLI/FIR` 升级门控
  - `state_machine.rs` 中 `LocalRepair/FrameRecovery` 超时
  - `stream/nack_scheduler.rs` 与 `video_source/nack.rs` 中 RTT 感知 retry/admission
  - `connection/service.rs` 中 `PLI/FIR` 执行边界
  - `H264 bootstrap` 入口处的 codec/depacketizer salvage 能力规划
  - runtime stats / diagnostics / trace 中 timing 与 decision 的结构化输出
- Out of scope:
  - 重新设计整个 recovery-first 架构
  - 引入无限重试或大缓存保画质策略
  - 修改 BWE/TWCC 主体算法
  - 非 H264 编码格式的 bootstrap 补救

## Non-Goals

- 不把恢复目标改成“尽量补齐所有帧”。
- 不让 `FIR` 重新成为通用主路径动作。
- 不在第一版引入复杂评分器或连续优化器。
- 不把 display/render 问题重新耦合回 transport 恢复主线。

## Design Principles

1. 时序参数优先由 RTT 决定，场景只提供 `multiplier / bias / floor / ceiling`。
2. `NACK` 优先修高价值可救小洞，不承担图片级或连接级主决策。
3. `PLI` 是媒体恢复动作，不再兼任高频状态机节拍器。
4. `FIR` 默认降级为 Cloud/受控发送端专用的重动作。
5. codec 能解决的问题留在 codec 层，不提早上抬到 recovery 主链。

## Design

### 1. 统一动态 RTT timing 模型

所有恢复时序参数统一走：

`value_ms = clamp(effective_rtt_ms * multiplier + bias_ms, floor_ms, ceiling_ms)`

其中：

- `effective_rtt_ms` 优先使用平滑 RTT，而不是单个瞬时样本；
- `clamp` 用于限制低 RTT 不被拖慢、高 RTT 不被无限拉长；
- profile 不再直接存固定阈值，改存：
  - `*_rtt_multiplier`
  - `*_bias_ms`
  - `*_floor_ms`
  - `*_ceiling_ms`

建议运行时来源优先级：

1. `smoothed_rtt_ms`
2. `latest_rtt_ms`
3. 场景默认 RTT

建议做两层稳定化：

- RTT 上升快、下降慢，避免短时间“恢复过于乐观”；
- 单次尖峰 capped，避免某次异常抖动把后续节奏拖到过长。

第一版可新增统一解析器：

- `resolve_effective_rtt_ms(stats, profile)`
- `resolve_dynamic_nack_timeout_ms(stats, profile)`
- `resolve_dynamic_nack_retry_interval_ms(stats, profile, value_tier)`
- `resolve_dynamic_pli_refresh_interval_ms(stats, profile)`
- `resolve_dynamic_fir_retry_interval_ms(stats, profile)`
- `resolve_dynamic_decoded_pending_commit_hold_ms(stats, profile)`

### 2. NACK 层：首发等待窗放宽优先，重试只做窄补充

当前生产路径偏向单发；这对 `10ms` LAN 可接受，但对 `100ms/200ms` 过硬。  
同时，单纯把 `1` 次重试补进来，常见结果是：

- 首发 deadline 仍偏短；
- 首轮自然闭环机会已经丢掉；
- 第二次发送虽然存在，但总耗时已经晚于“首发就多等一拍”的更优路径。

本 RFC 采用的第一原则是：

- 对高价值缺口，优先放宽首发等待窗与 admission 存活窗；
- 让第一次 `NACK` 有完整自然闭环机会；
- 重试只作为窄补充能力，不作为主收益来源。

第一版改成三档：

- `Disposable / LowValue`
  - 保持 `0` 次重试
- `Supply / Reference`
  - 首发等待窗按 RTT 感知放宽
  - 默认 `0` 次重试
  - 仅当 `estimated_recovery_arrival_ms` 仍明显早于 `frame_playout_deadline_at_ms` 时允许 `1` 次补充重试
- `Anchor / Keyframe`
  - 首发等待窗按 RTT 感知放宽，并使用更保守的 admission
  - 默认 `0` 次重试
  - 仅当 `repairability` 仍高、且本地闭环收益明显高于直接升级 `PLI` 时允许 `1` 次补充重试

建议重试间隔公式保留为补充路径：

`nack_retry_interval_ms = clamp(0.75 * effective_rtt_ms + 8, 12, 140)`

建议 `LocalRepair` 首发超时公式：

`nack_timeout_ms = clamp(1.6 * effective_rtt_ms + 40, floor, ceiling)`

建议场景边界：

- Home LAN:
  - `floor=45`
  - `ceiling=90`
- Home WAN:
  - `floor=120`
  - `ceiling=240`
- Cloud:
  - `floor=240`
  - `ceiling=420`

效果目标：

- `10ms` RTT 不拖慢本地 repair；
- `100ms` RTT 至少容纳一次真实往返重试；
- `200ms` RTT 下避免 `NACK` 还未自然闭环就被上层提前判死。

#### 2.1 首发放宽优先的判定合同

为避免实现再次滑回“逻辑上支持重试，统计上仍是单发短窗”，新增以下硬合同：

1. `Supply / Reference / Anchor` 的首发 `nack_timeout_ms` 必须先按动态 RTT 解析。
2. `NACK` 是否允许进入补充重试，必须发生在首发 deadline 已完整展开之后。
3. 若 `first_attempt_survival_window` 内仍满足以下条件，则优先继续等待首轮自然闭环：
   - `repairability` 仍高；
   - `estimated_recovery_arrival_ms <= frame_playout_deadline_at_ms`;
   - 当前未出现更高价值坏链证据。
4. 只有以下条件同时满足，才允许补充重试：
   - 首发已发出；
   - 首发未闭环；
   - 当前仍存在经济上的本地 repair 价值；
   - 补充重试不会把总时间推迟到比直接升级 `PLI` 更差。

新增建议观测字段：

- `firstAttemptSurvivalWindowMs`
- `firstAttemptDeadlineAtMs`
- `firstAttemptStillEconomical`
- `retryAllowedReason`
- `retrySuppressedReason`

### 3. too many missing：从数量阈值改成高价值影响阈值

当前“缺包太多直接要 keyframe”的合理部分要保留，但必须改语义。

不再使用单一“缺包数量过多”阈值；改成三因子组合：

- `missing_span_score`
  - 缺包是否连续
  - 是否跨 frame
  - 是否落在 `Anchor / Supply / Reference`
- `chain_repairability_score`
  - 当前 gap 是否仍可能通过本地 repair 闭环
- `deadline_survival_score`
  - 即便补回，是否还赶得上当前 playout / bootstrap / anchor deadline

新动作原则：

- `LowValue` 缺包再多，也不直接触发 keyframe 请求；
- 只有“高价值缺包 + repairability 已低 + 截止时间已不经济”才允许从 `NACK` 进入 `PLI`。

这要求 `NackScheduler` 与 `video_source/nack.rs` 继续输出结构化上下文：

- `frame_importance`
- `estimated_recovery_arrival_ms`
- `frame_playout_deadline_at_ms`
- `frame_unrecoverable_reason`
- `budget_context`

### 4. PLI：从状态机节拍器降回图片恢复动作

当前 `PLI` 已承担部分“推进状态机”的职责，导致 IDR 压力偏大。

目标改造：

- 状态机可频繁评估；
- `PLI` 不随每次评估直接重发；
- 进入 `ContinuationSeen` 后默认先观察 codec/bootstrap/clean-anchor 前进，而不是立即刷新 `PLI`。

建议把 `transportAwaitRecoveryAnchor` 路径的 keyframe in-flight 健康度收成三态：

- `waiting-response`
- `continuation-only`
- `awaiting-clean-anchor-commit`

动态 RTT 下，`PLI` 刷新公式建议：

`pli_refresh_interval_ms = clamp(0.8 * effective_rtt_ms + 20, 40, 220)`

目标效果：

- `10ms` RTT 时不被固定阈值拖慢；
- `100ms` RTT 时接近 `100ms` 级别刷新；
- `200ms` RTT 时允许 `180ms` 左右观察窗，避免过快重复施压编码器。

同时增加两个 patience 窗：

- `continuation_patience_window_ms`
- `clean_anchor_commit_patience_window_ms`

两者也改为 RTT 感知解析；它们决定“继续观察”与“刷新 PLI”的分界，而不是直接写死静态值。

### 5. FIR：收成 Cloud/受控发送端专用重动作

`FIR` 不再视为常规图片恢复主路径。

建议进入 `FIR` 必须同时满足：

1. `session_target_type == Cloud` 或远端明确受控；
2. 已经发生过至少一次 `PLI`；
3. 已观察到 `continuation-only` 或 `awaiting-clean-anchor-commit` 的持续停滞；
4. codec salvage 已尝试或不适用；
5. 当前未处于 `FIR` 冷却窗内。

动态 RTT 下，`FIR` 重试间隔建议：

`fir_retry_interval_ms = clamp(2.2 * effective_rtt_ms + 40, 140, 650)`

更保守可选：

`fir_retry_interval_ms = clamp(2.5 * effective_rtt_ms + 60, 180, 700)`

第一版建议采用更保守版本，只在 Cloud/受控发送端启用。

### 6. H264 codec/depacketizer salvage：补最近已提交的 SPS/PPS

当前对 `bootstrapMissingSps` / `bootstrapMissingPps` 的处理偏早上抬。

建议新增窄能力：

- 缓存最近一次已提交的 `SPS/PPS`
- 当接收到恢复期 `IDR` 或 bootstrap AU 时，如果：
  - `bootstrap_ready == false`
  - reason 属于 `bootstrapMissingSps/bootstrapMissingPps`
  - `committed_sps_present && committed_pps_present`
  - `parameter_sets_changed == false`
  - `config_changed == false`
  - `slice_headers_valid == true`
- 则在 depacketizer / access unit assembler 层把缓存的 `SPS/PPS` prepend 到本次 AU，再交给 decoder

显式限制：

- 不跨 profile/resolution/config 变更复用旧参数集；
- 不对非 IDR continuation 硬塞旧参数集；
- salvage 失败必须产出结构化观测，不允许静默吞掉。

收益目标：

- 减少 `bootstrapMissingSps/Pps` 直接升级到 `PLI/FIR`；
- 提高 `continuation-only -> clean-anchor` 的成功率；
- 把可闭合问题留在 codec 层。

### 7. RecoveryTimingResolver：单一解析入口

为避免把 RTT 感知逻辑撒在 `policy.rs`、`state_machine.rs`、`nack.rs` 多处，新增单一解析器：

- 建议位置：
  - `crates/xbxengine/core/src/transport/rtc/recovery/timing.rs`

职责：

- 根据 runtime stats + profile 解析所有动态 timing；
- 输出单一结构体，如：
  - `nack_retry_interval_ms`
  - `nack_timeout_ms`
  - `pli_refresh_interval_ms`
  - `fir_retry_interval_ms`
  - `continuation_patience_window_ms`
  - `decoded_pending_commit_hold_ms`

使用点：

- `session/policy.rs`
- `recovery/state_machine.rs`
- `stream/nack_scheduler.rs`
- `stream/video_source/nack.rs`

## Proposed Parameter Shape

建议将 [`recovery/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/policy.rs) 中固定 timing 字段改为：

- `nack_timeout_rtt_multiplier`
- `nack_timeout_bias_ms`
- `nack_timeout_floor_ms`
- `nack_timeout_ceiling_ms`
- `nack_retry_rtt_multiplier`
- `nack_retry_bias_ms`
- `nack_retry_floor_ms`
- `nack_retry_ceiling_ms`
- `pli_refresh_rtt_multiplier`
- `pli_refresh_bias_ms`
- `pli_refresh_floor_ms`
- `pli_refresh_ceiling_ms`
- `fir_retry_rtt_multiplier`
- `fir_retry_bias_ms`
- `fir_retry_floor_ms`
- `fir_retry_ceiling_ms`
- `continuation_patience_rtt_multiplier`
- `continuation_patience_bias_ms`
- `continuation_patience_floor_ms`
- `continuation_patience_ceiling_ms`
- `decoded_pending_commit_hold_rtt_multiplier`
- `decoded_pending_commit_hold_bias_ms`
- `decoded_pending_commit_hold_floor_ms`
- `decoded_pending_commit_hold_ceiling_ms`

兼容策略：

- 第一阶段保留旧固定字段并加 `resolve_*` 适配；
- 第二阶段删除旧固定字段，统一由解析器输出。

## Plan

1. 收口 timing 模型：
   - 在 `recovery/policy.rs` 引入 RTT 参数结构；
   - 新增 `RecoveryTimingResolver`；
   - 把 `PLI/FIR/NACK timeout` 读取改成统一解析。
2. 收口 `NACK` 本地修复主策略：
   - 高价值缺口采用“首发等待窗放宽优先”；
   - 重试降为窄补充路径；
   - `too many missing` 从数量阈值改成高价值影响阈值。
3. 收口 repair 与 recovery 语义边界：
   - 明确 `drop -> nack_pending -> nack_missed -> wait_keyframe -> request_idr`；
   - trace 与状态机拆开“本地 repair 未成”和“恢复主线升级”。
4. 收口 `PLI/FIR` 与 codec salvage 边界：
   - `PLI` 退出节拍器角色；
   - `FIR` 降为 Cloud/受控发送端的重动作；
   - 在 codec/depacketizer 层闭合可补救的 bootstrap 缺口。
5. 补齐 stats/trace/validation：
   - 让 trace 直接回答“首发等待窗为何放宽、为何继续等首轮、为何抑制或允许 retry、为何升级 PLI/FIR、是否做过 salvage、当前 effective RTT 是多少”。

## Validation

- [x] `cargo test -p xbxengine --lib`（覆盖 `transport::rtc::stream::video_source::nack`、`nack_scheduler`、`session::policy`、`connection::service` 等模块；本仓库 `cargo test` 单参数过滤等价于对上述路径的回归）
- [x] 新增：`cloud_high_rtt_reference_gap_prefers_wider_first_attempt_window_before_retry_or_pli`
- [x] 新增：`home_wan_supply_gap_does_not_escalate_before_dynamic_first_attempt_timeout`
- [x] 新增：`continuation_only_waits_dynamic_patience_window_before_pli_refresh`
- [x] 新增：`fir_is_cloud_only_and_requires_failed_pli_progress`
- [x] 新增：`bootstrap_missing_sps_uses_cached_parameter_sets_when_config_unchanged`
- [x] trace 字段（`src-tauri/.../trace_projection.rs` 已投影；由 `publish_recovery_timing_to_stats` + NACK poll 写入 stats）：
  - `effectiveRttMs`（`recovery_effective_rtt_ms`）
  - `dynamicPliRefreshIntervalMs` / `dynamicFirRetryIntervalMs` / `dynamicNackTimeoutMs`
  - `firstAttemptSurvivalWindowMs` / `firstAttemptDeadlineAtMs` / `firstAttemptStillEconomical`
  - `retryAllowedReason` / `retrySuppressedReason`
  - `codecBootstrapSalvageApplied`（及失败原因字段）
- [ ] 用真实 runtime trace 回归以下链路：
  - `RTT≈10ms` 下恢复不被静态阈值拖慢
  - `RTT≈100ms` 下 `PLI` 次数下降但恢复完成不回退
  - `RTT≈200ms` 下 `FIR` 次数下降，`NACK -> PLI` 升级更晚但更准

## Risks

- 如果 `effective_rtt_ms` 稳定化做得不对，恢复节奏会跟着 RTT 抖动。
- 如果只补一次 retry，不放宽首发等待窗，仍会出现“第二次动作存在，但第一次自然闭环机会已丢失”的节奏错位。
- 如果只改 `PLI/FIR`，不改 `NACK timeout/retry`，仍会出现“上层变聪明、底层仍过早判死”的节奏错位。
- 如果 `SPS/PPS salvage` 判定边界放太宽，可能把旧参数集错误复用到新配置。
- 如果 `FIR` 收得过严，而 Cloud 发送端对 `PLI` 响应不稳定，极端场景首轮恢复可能变慢。

## Progress

- [x] Step 1: 已完成现状勘察，确认静态 timing 是当前恢复偏差主因之一。
- [x] Step 2: 已形成统一方向：动态 RTT timing、`NACK` 首发等待窗放宽优先、`PLI/FIR` 收边界、`SPS/PPS salvage` 下沉。
- [x] Step 3: `recovery/timing.rs` 统一解析 + `RecoveryTimingRttParams`；`timing_rtt == None` 时 `PLI`/`FIR` 回退 profile 静态 `*_ms`，`decoded pending hold` 仍走 RTT 公式（与 transport await 门控对齐）。
- [x] Step 4: NACK 首发 deadline 与 `recovery_dynamic_nack_timeout_ms` 对齐（修复 transport merge 误用 playout 抵消下限；cloud floor 与动态 NACK 取 max）；`nack_scheduler` §2.1 survival + `retryAllowed`/`retrySuppressed` 观测；具名合同测试在 `policy_tests/recovery_dynamic_timing_contract.rs`。
- [x] Step 5: H264 salvage 与合同测试 `bootstrap_missing_sps_uses_cached_parameter_sets_when_config_unchanged`（实网 trace 回归仍待办）。

## Execution Notes

- Date: 2026-05-14 | Status: planned
- Update: 新建 RFC，目标是把恢复时序从静态阈值切换为动态 RTT 感知模型，并同步收紧 `PLI/FIR` 边界。
- Decision: 本 RFC 第一优先级不是改更多恢复状态，而是先把 timing 解析与动作时机统一。
- Decision: `NACK` 第一版采用“首发等待窗放宽优先，重试窄补充”的策略；高价值缺口不再以“一次短窗首发 + 一次补发”作为默认主路径。
- Decision: `FIR` 第一版默认只在 Cloud/受控发送端启用；Home 场景优先收口为 `PLI`。
- Decision: `H264 bootstrap salvage` 只做窄能力，不做跨 config 复用，不改变 `clean anchor` 语义。
- Date: 2026-05-14 | Status: planned
- Update: 根据低 RTT trace 复核，当前主问题更像“首发 survival window 偏紧”，不是“缺少第二次重试”本身；RFC 已改为首发放宽优先。
- Decision: repair 统计与 recovery 统计必须拆开；后续验收同时看 `packet repair success` 与 `recovery chain advance`。
- Date: 2026-05-14 | Status: implemented (core)
- Update: 落地首发 survival、`admission_deadline_floor_at_ms` 合并、`cloud_startup_head_hole` 与动态 NACK floor 取 max、trace 中 `retryAllowedReason`（含 `firstAttemptWindowElapsed`）等；RFC Validation 中单元测试与 trace 映射已勾选，实网 trace 三档 RTT 仍为待办。
