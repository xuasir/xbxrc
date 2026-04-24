# 阶段化恢复进度与动态修复价值策略 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: in-progress
- Owner: Codex
- Last Updated: 2026-04-22

## Background

- 最近多份播放期 trace 已经证明：同一 recovery epoch 内，远端返回顺序可以是“先 Non-IDR，后 IDR，再 decoded，再 clean anchor committed”。
- 当前系统仍然把 `NonIdrVcl`、`transportDeferred`、部分 NACK 过期结果直接提升成升级倾向，缺少“当前阶段下这条事实代表什么”的中间层。
- 当前 `transportAwaitRecoveryAnchor` 同时承载了远端回流、anchor 建链、decode 成功、clean anchor 提交、播放稳定五类语义，导致：
  - 恢复期 continuation 样本容易被误判为失败；
  - NACK 价值判断偏静态，容易在错误阶段堆砌理想化逻辑；
  - decode 后显示域问题仍会反向污染 media 恢复动作。
- 现有 [`docs/rfcs/2026-04-18-post-decode-display-scheduling-and-media-recovery-decoupling.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-18-post-decode-display-scheduling-and-media-recovery-decoupling.md) 已经定义了显示域解耦方向；本 RFC 聚焦 recovery 主链本身的长期语义收敛。

## Goal

- 将 recovery 主链改成“阶段化恢复进度 + 动态修复价值”的简单模型，替代对 `NonIdrVcl` / NACK 结果的直接动作映射。
- 明确 `media recovery complete` 与 `display recovery complete` 的边界，避免同一条诊断链同时表达 transport、decode、display 三层完成态。
- 让 NACK、RFI、decoder reset 都依赖当前阶段与当前进度缺口决策，不再依赖孤立静态标签。
- 将控制面动作边界收敛到浏览器/WebRTC 常见主线：`NACK -> PLI -> IDR落地`，把 `clean anchor / owner / display` 收回阶段调参与完成判据。
- 图片级恢复动作的单轨切换、`RequestKeyframe` 删除、display 退出图片级恢复主链，由配套 RFC [`docs/rfcs/2026-04-22-recovery-action-single-track-cutover.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-22-recovery-action-single-track-cutover.md) 承接。

## Scope

- In scope:
  - [`crates/xbxengine/core/src/transport/rtc/recovery/contract.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/contract.rs)
  - [`crates/xbxengine/core/src/runtime_stats_sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/runtime_stats_sink.rs)
  - [`crates/xbxengine/core/src/transport/rtc/session/facts.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/facts.rs)
  - [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs)
  - [`crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs)
  - [`crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs)
  - [`crates/xbxengine/core/src/media/video/ingress/budget.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/ingress/budget.rs)
  - [`src-tauri/src/mods/xbxengine/trace_projection.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/xbxengine/trace_projection.rs)
- Out of scope:
  - 重写整个 recovery-first 架构
  - 引入新的复杂评分器、机器学习模型、连续浮点调参系统
  - 重做 renderer / pacer 主链；显示域动作边界延续既有 RFC

## Design

### 0. 标准对齐原则

- `NACK` 保持 transport repair 主路径，职责是修小洞与短时乱序。
- `PLI` 保持图片级恢复主路径，职责是“当前图像不可继续，需要新的解码锚点”。
- `FIR` 收敛为重动作保底，只保留给切流、订阅恢复、编码器状态异常、PLI 长时间无响应等更硬场景。
- `decoder reset` 保持本地 decode 域动作，只在 `AnchorSeen/Decoded` 之后仍无可用输出时允许升级。
- `display` 域事实只影响本地保供给与恢复完成判据，不再直接主导 `PLI/FIR/requestKeyframe`。

这组边界与 RFC 4585、RFC 5104 的主语义一致，目标是让控制面动作更短、更稳，同时继续保留现有低延迟目标与远端画像收益。

### 1. 恢复进度只保留六级

- `WaitingResponse`：RFI/NACK 已触发，尚未看到有效回流
- `ContinuationSeen`：已看到可继续当前 epoch 的样本，例如 `NonIdrVcl`
- `AnchorSeen`：已看到可起锚样本，例如 IDR 或同等 clean bootstrap
- `Decoded`：恢复样本已进入解码成功
- `CleanAnchorCommitted`：当前 epoch 已提交 clean anchor
- `DisplayStable`：present/output 连续恢复，显示域确认稳定

规则：
- `NonIdrVcl` 在 recovery 期默认进入 `ContinuationSeen`
- `NonIdrVcl` 不单独构成升级证据
- media 动作只由“当前缺失哪一级进度”决定
- `cleanAnchorCommitted` 定义为 media recovery complete
- `DisplayStable` 定义为 display recovery complete

### 1.1 当前 trace 归因口径

- “远端没发 IDR”对应 `WaitingResponse/ContinuationSeen` 长时间停留，且 `response-observed` summary 没有 `firstKeyframePacketSeq`
- “本地窗口接住了包但 admission/inspection 仍拒绝”对应 `ContinuationSeen` 停留，同时 summary 已有 `firstVideoPacketSeq`、`admissionAccepted=true`、`deltaContinuationReady=true`，但没有 `AnchorSeen`
- “usable IDR 已到达但未接入当前恢复轮次”对应 `AnchorSeen` 已出现，`boundEpisodeId=null` 或当前 active episode/clean anchor 未推进
- `RtcVideoFrameSource rx closed` 作为 media ingress 硬断点，统一通过 `rebuildPeerConnection` / `stackStop` 因果标签进入 lifecycle 域，而不是再写成恢复链噪声

### 2. 修复价值只保留四档，并按阶段重映射

- `Anchor`：直接影响建链
- `Continuation`：推动当前恢复序列向前
- `Supply`：影响恢复后的短期供给稳定
- `Disposable`：局部修复收益低，可直接放弃

规则：
- 同一丢包/样本的价值允许随阶段变化
- `startup / priming` 优先 `Anchor`
- `recovering / transportAwaitRecoveryAnchor` 优先 `Anchor + Continuation`
- `sustaining-recovery` 优先保护刚提交的 clean anchor 周边
- `steady` 允许更多 `Disposable` 与本地丢弃

### 2.1 画像策略的保留方式

- 保留远端画像与动态子画像，位置从“控制面主决策节点”收敛为“phase policy 参数层”
- 保留 `post-IDR climbing` 宽容，位置从“再解释一次是否要 requestKeyframe”收敛为“调 NACK 窗口、PLI 节流、恢复完成门槛”
- 保留 `clean anchor`，位置从“是否允许发恢复动作的总闸门”收敛为“恢复阶段调参器 + completion gate”

建议冻结一组统一 phase policy 参数：

- `idr_protection_window_ms`：IDR 后保护窗，放宽 continuation / supply repair
- `post_idr_repair_budget`：IDR 后允许的额外 NACK / repair 预算
- `clean_anchor_confidence_window_ms`：clean anchor 未确认前的宽容窗
- `transport_reorder_window_packets`：transport 乱序窗
- `transport_repair_deadline_ms`：NACK 截止时间
- `pli_min_interval_ms`：PLI 节流
- `fir_escalation_delay_ms`：PLI 无响应后才允许进入 FIR
- `decoder_reset_after_anchor_stall_ms`：AnchorSeen/Decoded 后 decode 域升级门限

这些参数继续允许按 `HomeLanGaming / CloudGaming / RelayGaming` 与动态子画像重映射。

### 3. 动作边界按缺口分配

- 控制动作链固定为 `NACK -> PLI`
- `FIR` 属于重保底升级，不属于常规控制动作链
- `IDR/AnchorSeen -> Decoded -> CleanAnchorCommitted -> DisplayStable` 属于恢复进度链与完成判据
- `PLI/FIR`：只处理 `WaitingResponse`、`ContinuationSeen` 长时间缺 `AnchorSeen`
- NACK：只处理当前阶段仍有修复收益的 gap
- decoder reset：只处理 `AnchorSeen` 或 `Decoded` 之后仍卡在 decode 侧的情况
- display 本地动作：只处理 `CleanAnchorCommitted` 之后的队列积压、present 停滞、renderer/presenter 抖动

### 3.1 目标动作拓扑

1. `NACK`
2. `PLI`
3. `IDR/AnchorSeen`
4. `Decoded`
5. `CleanAnchorCommitted`
6. `DisplayStable`

扩展规则：

- `FIR` 只在 `PLI` 达到节流上限后仍没有 `AnchorSeen` 时参与
- `requestDecoderReset` 只在 `AnchorSeen/Decoded` 之后仍无 decode progress 时参与
- `reconnect` 继续只留给 lifecycle/connectivity 域，不再由纯 media-domain `transportAwaitRecoveryAnchor` 直接抬升

### 3.2 当前代码事实到新拓扑的映射

- [`crates/xbxengine/core/src/transport/rtc/connection/service.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/connection/service.rs)
  - 当前已有 `PLI -> FIR -> control keyframe` 梯子
  - 目标是让图片级恢复出口符合 `PLI` 主路径与 `FIR` 重保底
- [`crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs)
  - 当前已有 `DynamicRepairValueTier`、`FrameBudgetWindowSource::Recovery`、clean-anchor 供给窗
  - 目标是保留这些能力，并把输入统一改成 `phase policy + remote profile`
- [`crates/xbxengine/core/src/media/video/ingress/budget.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/ingress/budget.rs)
  - 当前 `Anchor / Continuation / Supply / Disposable` 已经成型
  - 目标是把 `FrameBudgetRecoveryPhase` 与 `RecoveryProgressLevel` 对齐，避免再次引入平行阶段语言
- [`crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs)
  - 当前 owner 已承担 `SeekingAnchor / RebuildingSupply / StableServing`
  - 目标是让 owner 只负责“阶段与完成判据”
- [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs)
  - 当前已有 ramp-up 吸收、transport-await reconnect fallback、clean anchor 后余波吸收
  - 目标是把这里收成 orchestrator：只负责 `progress gap + fault domain + phase policy` 编排
- [`crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs)
  - 当前仍混有 clean anchor、decoded、reconnect in-flight 的多层解释
  - 目标是把 coordinator 收成动作协调器，输入统一来自结构化 `RecoveryProgressLevel`

### 4. 现有关键标签的新语义

- `NonIdrVcl`：continuation 事实
- `transportDeferred`：请求层事实，单独记录，不直接代表恢复失败
- `cleanAnchorCommitted`：media 恢复完成
- `outputQueueOverflow`：display 域阻断
- `presentAge stale` / `hostPresentStalled`：display completion 未达成

### 5. 落地方案

#### 5.1 Workstream A: 固定控制面主链

- `session/policy` 与 `recovery/coordinator` 统一改为先判定 `RecoveryProgressLevel`
- `progress < AnchorSeen` 时，常规控制动作只允许 `NACK/PLI`
- `FIR` 只在 `PLI` 节流后持续缺 `AnchorSeen` 时参与
- `progress >= AnchorSeen && < Decoded` 时，只允许 `decoder-local` 类动作，不再继续打 transport-heavy 升级
- `progress >= CleanAnchorCommitted` 后，media 恢复动作收口，display 进入独立保供给路径
- 单轨动作枚举与删除项见配套 RFC [`docs/rfcs/2026-04-22-recovery-action-single-track-cutover.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-22-recovery-action-single-track-cutover.md)

#### 5.2 Workstream B: 把画像策略收成 phase policy

- `remote_profile_runtime` 输出统一 phase policy 参数，不直接输出控制动作偏好
- `nack.rs`、`budget.rs`、`session/recovery_ramp_guard.rs` 统一读取 phase policy
- `clean anchor` 继续参与 `post-IDR climbing` 保护，重点影响：
  - NACK 是否继续给 `Continuation/Supply`
  - NACK deadline 与 repair budget 是否临时放宽
  - PLI 是否在爬升期被压住
  - `DisplayStable` 完成前是否吸收轻微信号

#### 5.3 Workstream C: episode 与 trace 语义去耦

- `keyframe episode` 保留 trace / response correlation 价值
- `episode` 不再作为“是否承认这次 IDR 可用”的唯一闸门
- `response-observed` summary 继续保留：
  - `firstVideoPacketSeq`
  - `firstKeyframePacketSeq`
  - `oosDepthP75`
  - `headMissingActive`
  - `gapExpiredBeforeKeyframe`
- 以此稳定区分三类情况：
  - 远端只回 delta
  - 本地收到了包但仍停在 continuation
  - 锚点已到但语义绑定/阶段推进没有前移

#### 5.4 Workstream D: lifecycle 硬断点单独收口

- `RtcVideoFrameSource rx closed` 统一标记为 ingress/lifecycle 硬断点
- `rebuildPeerConnection`、`stackStop` 继续作为标准因果标签
- 这些信号直接进入 lifecycle/connectivity 恢复链，不与 `NonIdrVcl`、NACK、clean anchor 混写

### 6. 模块改造顺序

1. [`crates/xbxengine/core/src/transport/rtc/recovery/contract.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/contract.rs)
   - 固化 `RecoveryProgressLevel` 与 `progress -> allowed actions`
2. [`crates/xbxengine/core/src/transport/rtc/connection/service.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/connection/service.rs)
   - 收口为 `PLI/FIR` 主图片恢复出口
3. [`crates/xbxengine/core/src/media/video/ingress/budget.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/ingress/budget.rs)
   - 将动态修复价值绑定到 phase policy
4. [`crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs)
   - 落地 post-IDR climbing、clean-anchor 宽容与 profile 参数
5. [`crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs)
   - 收成阶段 owner 与 completion gate
6. [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs)
   - 收成 orchestrator
7. [`crates/xbxengine/core/src/runtime_stats_sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/runtime_stats_sink.rs) 与 [`src-tauri/src/mods/xbxengine/trace_projection.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/xbxengine/trace_projection.rs)
   - 对齐 trace summary、lifecycle cause、progress projection

## Plan

1. 收敛事实模型：在 `contract/stats/facts/trace` 中统一引入恢复进度分层和动态价值分层，替代当前对 `NonIdrVcl`、terminal deferred、NACK 过期的直接升级语义。
2. 收敛动作门控：让 `session/policy`、`video_scheduling_owner`、`nack/budget` 统一改为“按当前阶段缺失的进度级别决策”，并把 display 完成态从 media 完成态剥离。
3. 收敛观测与验证：补 trace 字段与定向测试，锁住“先 Non-IDR、后 IDR、再 clean anchor”的恢复路径，以及不同阶段下 NACK 价值重映射的合同。

## Validation

- [ ] `cargo test -p xbxengine transport_await_non_idr_grace_window_tracks_epoch_sample_positions -- --nocapture`
- [ ] `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
- [ ] `cargo test -p xbxengine video_scheduling_owner -- --nocapture`
- [ ] `cargo test -p xbxengine transport::rtc::stream::video_source::nack -- --nocapture`
- [ ] `cargo test -p xbxrc trace_projection -- --nocapture`
- [ ] 用新 runtime trace 验证 `NonIdrVcl -> IDR -> decoded -> cleanAnchorCommitted` 会被系统识别为同一恢复序列推进，而不是提前升级
- [ ] 用新 runtime trace 验证 `PLI -> firstKeyframePacketSeq -> first decode` 的三段时延
- [ ] 用新 runtime trace 验证 `rx closed` 会进入 `rebuildPeerConnection` / `stackStop` 因果链，不再污染 media progress
- [ ] 图片级恢复动作单轨切换的删除清单与 grep 验证，见配套 RFC [`docs/rfcs/2026-04-22-recovery-action-single-track-cutover.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-22-recovery-action-single-track-cutover.md)

## Risks

- 如果恢复进度层与现有 episode/coalesced 语义没有一起收敛，系统仍会保留“未发送但已 decoded”的混合观测，继续污染判断。
- 如果只改 NACK 价值映射，不同步改 owner/session 的完成态定义，display 域噪声仍会被翻译回 media 恢复。
- 如果阶段划分过细，规则会重新膨胀；本 RFC 只允许六级进度、四档价值，不继续扩表。
- 如果 `FIR` 仍保留在常规主链，远端编码压力与控制面噪声会继续偏大。
- 如果 post-IDR climbing 没有统一挂回 phase policy，现有画像经验会继续散落在 owner/session/source 多处局部 if-else 中。

## Progress

- [x] Step 1: 已确认现状问题是“恢复进度、修复价值、动作边界”三层语义混杂。
- [x] Step 2: 已完成 trace 证据补强与 transport-await/clean-anchor 语义收口，形成“标准主链 + phase policy”落地方向。
- [ ] Step 3: 完成控制面主链与 phase policy 的代码改造。
- [ ] Step 4: 完成 trace/测试与新 runtime trace 验证。

## Execution Notes

- Date: 2026-04-19 | Status: planned
- Update: 基于最新播放期 trace，确认 `NonIdrVcl` 属于恢复序列中的 continuation 事实，当前系统需要从“静态失败标签”转向“阶段化恢复进度”。
- Decision: 长期方案保持简单，只引入六级恢复进度和四档动态修复价值，不新增复杂评分器。
- Decision: `cleanAnchorCommitted` 定义为 media 恢复完成；`DisplayStable` 另作显示域完成态，避免两个完成条件继续混用。
- Risk/Blocker: 现有 `keyframeRequestEpisode` 存在 coalesced / unsent / decoded 混合语义，后续实现阶段需要先统一 episode 事实口径。
- Date: 2026-04-22 | Status: in-progress
- Update: 基于 `runtime-trace-1776837672744-1.jsonl` 与新增 trace summary，已确认一条关键坏链：同一窗口内存在可用 IDR 到达，但 active episode / clean-anchor 语义推进没有同步前移，随后 continuation delta 继续以 `NonIdrVcl` 被拒绝。
- Update: 已完成两项前置收口：1）`response-observed` summary 增加 `firstVideoPacketSeq/firstKeyframePacketSeq/oosDepthP75/headMissingActive/gapExpiredBeforeKeyframe`；2）`RtcVideoFrameSource rx closed` 增加 `rebuildPeerConnection/stackStop` 因果标签。
- Decision: 控制面主链按浏览器/WebRTC 常见职责收敛为 `NACK -> PLI -> IDR落地`，`FIR` 收为重保底，`clean anchor / owner / display` 收回 phase policy 与 completion gate。
- Decision: 现有远端画像、`post-IDR climbing`、clean-anchor 宽容全部保留，统一沉淀为 phase policy 参数层，不再作为请求关键帧主链的并行决策节点。
- Validation Note: 当前仓库存在 `ohmygamepad-sdl3` 无关编译阻塞，需在后续实现阶段绕开或修复后再跑完整验证矩阵。
