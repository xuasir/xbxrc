# 播放期恢复单线收敛与 keyframe family gate 修复 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: planned
- Owner: Codex
- Last Updated: 2026-04-29

## Background

- 近期播放期 runtime trace 反复出现同一类长尾：
  - 远端已经返回 keyframe response packet
  - 本地也可能已经 decode 过一帧
  - render/host 侧已经具备继续出图的条件，甚至已经出现有价值呈现
  - recovery 主线仍停在 `bootstrapMissingIdr / NonIdrVcl / noCleanAnchorCommit`
- 当前系统内部已经出现两类事实：
  - codec / bootstrap 事实：当前 recovery epoch 是否拿到 fresh usable IDR
  - playback / render 事实：当前是否已经形成可服务输出
- 当前 owner/pacer/host 已经部分承认“committed SPS/PPS + delta continuation ready”具有服务价值：
  - `NonIdrVcl + committed SPS/PPS + delta ready` 在有 clean anchor 时允许退出恢复或继续 serving
  - render/host 已经在 latest-only 链路里消费这类 continuation
- 当前 recovery facts 仍然将主线强绑定到 `fresh usable IDR -> cleanAnchorCommitted -> DisplayStable`：
  - `facts.rs` 会在 decode 之后再次看到同 episode 的 `bootstrapMissingIdr / NonIdrVcl` 时，把 `Decoded` 回写成 `ContinuationSeen / WaitingResponse`
  - 这让“播放已经恢复”和“恢复主线仍未恢复”长期并存
- 当前 `transport_session` 里还有两条已识别但未收口的 family gate 风险：
  - `decoded_keyframe_without_clean_anchor_does_not_hold_family_gate_after_hold_window`
  - `non_idr_vcl_keyframe_response_does_not_hold_family_gate`
  - 这说明 continuation-only 响应、或者 decoded 但 commit 未落账之后，后续 keyframe 请求的放行逻辑仍有收口缺口
- 当前主问题已经从“单点条件太严”演化为“恢复主线缺少 render 价值证据，且 keyframe family gate 与主线语义没有完全对齐”。

## Problem Statement

- 当前恢复系统对“当前 recovery epoch 拿到 fresh usable IDR”与“用户已经重新看到可服务画面”采用了混合主线。
- render/host 已经在执行价值判断，recovery facts 却没有把这类价值证据提升为主线正式阶段。
- 如果继续在 recovery 之外单独增加一条 `soft-serving` 分叉，会制造新的并行主线。
- 如果直接把 serviceable continuation 伪装成 `CleanAnchorCommitted`，会污染 `first-frame latency`、recovery success rate、故障归因与后续升级门槛。
- 如果 family gate 继续按“已见过 keyframe response / 已 decoded”粗放压制同 family 新请求，系统会低估“仍缺 fresh usable IDR”的持续压力。

## Goal

- 将播放期恢复收敛为一条单线恢复主线，显式纳入 render/host 的“有价值呈现”证据。
- 保留 `bootstrap` 与 `cleanAnchorCommitted` 的硬语义，不把 continuation 伪装成 fresh anchor。
- 在同一轮改造中收口两条 keyframe family gate 修复，避免主线改完后继续被旧 gate 压制。
- 在同一轮改造中补齐 `PLI -> FIR` 的升级线路，让“播放已恢复但 fresh anchor 长期缺失”与“response seen 但持续 continuation-only”都能进入更强关键帧压力路径。
- 让 owner、session、coordinator、transport_session、stats、trace 统一消费同一条恢复主线。
- 让“播放恢复”和“fresh anchor 恢复”成为同一主线中的不同阶段，而不是两条并行状态机。

## Non-Goals

- 不放松 H264 bootstrap 对 fresh usable IDR 的定义。
- 不把 `PlaybackRecovered` 等价成 `CleanAnchorCommitted`。
- 不回退到双轨恢复实现。
- 不在本 RFC 中重写 pacer / render / host 的 latest-only 比较器。
- 不重写整套 escalation controller；本轮只补齐与播放期单线主线对齐的 `PLI -> FIR` 升级合同。

## Scope

- In scope:
  - [`crates/xbxengine/core/src/transport/rtc/recovery/contract.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/contract.rs)
  - [`crates/xbxengine/core/src/transport/rtc/session/facts.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/facts.rs)
  - [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs)
  - [`crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs)
  - [`crates/xbxengine/core/src/runtime_stats_sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/runtime_stats_sink.rs)
  - [`crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs)
  - 恢复相关集成测试与 trace contract 测试：
    - `session/policy_tests/recovery_integration.rs`
    - `session/facts.test.rs`
    - `policy/video_scheduling_owner.test.rs`
    - `stack/transport_session.rs` 内测试
- Out of scope:
  - 重写 decode/pacer/render/host latest-only 主链
  - 新增平行恢复状态机
  - TWCC/BWE 画像重构
  - FIR 升级策略的大规模重写

## Design

### 1. 统一恢复主线

当前恢复主线：

- `WaitingResponse`
- `ContinuationSeen`
- `AnchorSeen`
- `Decoded`
- `CleanAnchorCommitted`
- `DisplayStable`

目标恢复主线：

- `WaitingResponse`
- `ContinuationSeen`
- `AnchorSeen`
- `Decoded`
- `PlaybackRecovered`
- `FreshAnchorRecovered`
- `DisplayStable`

语义约束：

- `AnchorSeen` 继续表示当前 episode 已看到关键恢复响应包或等价锚点响应证据
- `Decoded` 继续表示当前 episode 已完成 decode 级进展
- `PlaybackRecovered` 表示 render/host 已形成有价值呈现，用户侧已经恢复观看价值
- `FreshAnchorRecovered` 表示当前 recovery epoch 已拿到 fresh usable IDR，并完成 clean-anchor 提交
- `DisplayStable` 继续表示 host/display 收尾完成

映射规则：

- 现有 `CleanAnchorCommitted` 在 contract 命名上可保留，也可在内部语义上作为 `FreshAnchorRecovered` 对应阶段
- `PlaybackRecovered` 是主线正式阶段，同时不替代 `AnchorSeen / Decoded`
- `ContinuationSeen` 继续表达“continuation 已到，但尚无 render/host 级恢复证据”
- `AnchorSeen / Decoded` 继续作为升级门槛、episode 生命周期与 trace 叙事的稳定中间态

### 2. `PlaybackRecovered` 的证据来源

`PlaybackRecovered` 直接复用现有 render/host 侧已经使用的事实，不重新发明第二套定义。

最小证据集合：

- `continuation_verdict == continuationAcceptedWhileAwaitingIdr`
- `committed_sps_present == true`
- `committed_pps_present == true`
- `delta_continuation_ready == true`
- 当前 recovery epoch 已出现以下至少一项纯底层事实：
  - host 已 first present / visible present
  - render 已提交当前 recovery epoch 的 serviceable continuation
  - pacer 已接受当前 recovery epoch 的 serviceable continuation 并进入可交付链
  - decode 后出现与当前 episode 绑定的 serviceable continuation，并伴随新鲜的 track/video bytes 进展

补充约束：

- `PlaybackRecovered` 的判定只允许读取底层事实：
  - h264 inspection
  - decode/pacer/render/host runtime facts
  - track/bytes/timeline 进展
- `PlaybackRecovered` 禁止读取 owner 的状态结论、diagnostics、reason label 作为入证条件
- owner 只能消费 `PlaybackRecovered`，不能反向成为 `PlaybackRecovered` 的证据源

freshness 约束：

- `PlaybackRecovered` 只接受新鲜底层证据
- 新鲜度窗口由 phase policy 显式提供，至少包括：
  - `playback_recovered_host_present_fresh_ms`
  - `playback_recovered_render_submit_fresh_ms`
  - `playback_recovered_track_progress_fresh_ms`
  - `playback_recovered_decode_progress_fresh_ms`
- 超过 freshness window 的旧 host/render/track/decode 证据不得继续单独维持 `PlaybackRecovered`
- `PlaybackRecovered` 的维持需要持续出现至少一种新鲜底层证据，或在短宽容窗内等待下一拍证据补齐

phase policy 参数面：

- `PlaybackRecovered` 的 freshness 参数属于统一 phase policy 的子集，禁止游离在第二套阈值系统之外
- 最低要求纳入同一参数面：
  - `playback_recovered_host_present_fresh_ms`
  - `playback_recovered_render_submit_fresh_ms`
  - `playback_recovered_track_progress_fresh_ms`
  - `playback_recovered_decode_progress_fresh_ms`
  - `decoded_progress_fresh_ms`
  - `decoded_pending_commit_hold_ms`
  - `pli_refresh_interval_ms`
  - `fir_retry_interval_ms`
  - `post_anchor_continuation_grace_ms`

phase 子画像要求：

- phase policy 需要支持至少以下档位：
  - startup
  - recovering
  - post-anchor
  - cloudHighRtt
- `PlaybackRecovered` freshness 在不同子画像下允许分档，但必须仍由同一参数表提供

优先级规则：

- `decoded_pending_commit_hold_ms` 优先于 `playback_recovered_decode_progress_fresh_ms`
- `post_anchor_continuation_grace_ms` 优先于 continuation-only 的立即回退
- `display_supply_thresholds` 继续承担 owner/display supply 判据，不直接替代 `PlaybackRecovered` 的 freshness 参数
- 若现有 `degraded/critical present/decode age` 阈值与 `PlaybackRecovered` freshness 同时出现，phase policy 必须明确由哪一组驱动 facts、哪一组驱动 owner

统一入口要求：

- 上述 freshness / hold / interval 参数统一进入现有 phase policy 入口
- 禁止在 `facts.rs`、`owner.rs`、`policy.rs`、`transport_session.rs` 各自定义本地常量形成第二套阈值系统
- phase policy 需要显式提供给以下模块：
  - `session/facts.rs`
  - `policy/video_scheduling_owner.rs`
  - `session/policy.rs`
  - `transport_session.rs`

模块消费边界：

- `session/facts.rs`
  - 负责消费：
    - `playback_recovered_*_fresh_ms`
    - `decoded_progress_fresh_ms`
    - `decoded_pending_commit_hold_ms`
- `video_scheduling_owner.rs`
  - 负责消费：
    - `display_supply_thresholds`
    - `post_anchor_continuation_grace_ms`
    - `PlaybackRecovered` 已产出的主线阶段
- `session/policy.rs`
  - 负责消费：
    - `pli_refresh_interval_ms`
    - `fir_retry_interval_ms`
    - `PlaybackRecovered / FreshAnchorRecovered` 主线阶段
- `transport_session.rs`
  - 负责消费：
    - `decoded_pending_commit_hold_ms`
    - `fir_retry_interval_ms`
    - unresolved episode gate 释放相关参数

子画像分档要求：

- startup
  - `PlaybackRecovered` freshness 窗口可略宽于 steady，避免首帧阶段过早回退
- recovering
  - 使用标准 freshness/hold 参数
- post-anchor
  - 优先应用 `post_anchor_continuation_grace_ms`
- cloudHighRtt
  - 允许放宽 `playback_recovered_track_progress_fresh_ms`
  - 允许放宽 `decoded_pending_commit_hold_ms`
  - `fir_retry_interval_ms` 必须显式高于非 cloud 默认值

约束：

- `PlaybackRecovered` 证据必须从现有 render/host/owner 原始事实复用
- `session/facts` 不允许另造一套只存在于 session 的“软恢复”定义
- `session/facts -> owner -> session/facts` 禁止形成闭环依赖

### 3. `bootstrap` 与 `clean anchor` 的边界

保留以下硬边界：

- `bootstrap_ready == true` 继续表示当前 access unit 可作为 fresh usable IDR / bootstrap anchor
- `bootstrapMissingIdr / NonIdrVcl` 继续表示当前轮次仍缺 fresh usable IDR
- `cleanAnchorCommitted` 继续只表达当前 recovery epoch 的 fresh anchor 已被消费并落账

主线变化：

- `PlaybackRecovered` 晚于 `Decoded`，早于 `FreshAnchorRecovered`
- `FreshAnchorRecovered` 不再承担“播放是否已经恢复”的全部职责
- `first-frame latency` 主指标继续只认 `FreshAnchorRecovered` 和 `DisplayStable`
- 新增播放恢复指标时单列 `PlaybackRecovered`

### 4. `facts.rs` 改法

当前问题点：

- `downgrade_recovery_progress_from_current_bootstrap()` 会把 `Decoded` 回写成 `ContinuationSeen / WaitingResponse`
- 这条逻辑只表达“fresh anchor 仍缺失”，没有表达“播放已经恢复”
- `Decoded` 当前既可能被当作历史事实，也可能被当作当前阶段，freshness 边界不清楚

改法：

- 当 `Decoded` 后看到同 episode 的 `bootstrapMissingIdr / NonIdrVcl`：
  - 如果 continuation 不可服务，继续回写到 `ContinuationSeen` 或保留 `Decoded` 附带“fresh anchor missing”侧证据，具体由 contract 明确
  - 如果 continuation 已满足纯底层 `PlaybackRecovered` 证据，推进到 `PlaybackRecovered`
- `facts` 层不再使用“是否缺 fresh IDR”去覆盖“播放是否已恢复”
- `AnchorSeen / Decoded` 继续保留为正式 progress stage，不允许只在局部实现里私下存在

`Decoded` freshness 合同：

- `Decoded` 既是 episode 历史事实，也是当前主线阶段
- 作为“当前主线阶段”时，必须受 `decoded_progress_fresh_ms` 约束
- 作为“历史事实”时，可继续保留 `first_keyframe_decoded_at_ms` 等 episode 字段，不自动代表当前仍处于 `Decoded`
- `decoded-no-commit sustained` 的判定必须同时读取：
  - `first_keyframe_decoded_at_ms`
  - `decoded_pending_commit_hold_ms`
  - `decoded_progress_fresh_ms`
- 超过 `decoded_pending_commit_hold_ms` 且 decode 证据已不新鲜时，不应继续停在 `Decoded` 当前阶段
- `PlaybackRecovered -> Decoded` 的回退只允许落到“新鲜 Decoded”
- 若 decode 证据已过 freshness window，回退应直接进入 `ContinuationSeen`

统一消费约束：

- 凡是“当前阶段决策”，必须读取 `RecoveryProgressLevel`
- 凡是“历史事实 / side evidence / trace 细节”，才允许直接读取：
  - `first_keyframe_packet_at_ms`
  - `first_keyframe_decoded_at_ms`
  - `response_verdict`
  - 其他 episode 原始时间戳

具体约束：

- `session/policy.rs`
  - 使用 `RecoveryProgressLevel` 决定：
    - 是否继续 `PLI refresh`
    - 是否升级 `FIR`
    - 是否降低 reconnect / decoder reset 压力
- `video_scheduling_owner.rs`
  - 使用 `RecoveryProgressLevel` 决定 serving/rebuilding 回退
- `transport_session.rs`
  - 使用 `RecoveryProgressLevel` 或等价 freshness 语义决定 family gate 解锁
  - episode 原始时间戳只用于计算 hold window 是否到期
- `runtime_stats_sink.rs`
  - `terminal_phase` 继续可从 episode 原始时间戳生成
  - 但新的 `PlaybackRecovered` / `FreshAnchorRecovered` 口径必须显式区分，不得从原始时间戳反推当前阶段

禁止事项：

- 禁止模块绕过 `RecoveryProgressLevel` 直接把 `first_keyframe_decoded_at_ms` 当成“当前仍在 Decoded”
- 禁止 owner / policy / transport_session 各自对 `Decoded` 新鲜度做不一致解释

规则：

- `PlaybackRecovered` 与 `FreshAnchorRecovered` 必须可共存于同一 episode 的不同时间点
- 从 `PlaybackRecovered` 进入 `FreshAnchorRecovered` 只认 fresh anchor 事实
- 从 `FreshAnchorRecovered` 回落，仍按现有 post-anchor / continuation-only 退化合同处理
- `FIR` 升级门槛可以读取 `AnchorSeen / Decoded / PlaybackRecovered` 的不同阶段，不需要另造平行状态

状态回退规则：

- `PlaybackRecovered -> Decoded`
  - 当 playback 证据不再新鲜，但 decode 级进展仍新鲜存在
- `PlaybackRecovered -> ContinuationSeen`
  - 当 playback 证据与 decode 级进展都不再新鲜，但 continuation-only 事实仍存在
- `FreshAnchorRecovered -> PlaybackRecovered`
  - 仅当 fresh anchor 后续失去有效性，但 render/host 侧仍持续存在新鲜可播证据
- `FreshAnchorRecovered -> Decoded / ContinuationSeen`
  - 仅当既失去 fresh-anchor 有效性，又失去 playback recovered 所需新鲜可播证据

禁止规则：

- 单次 continuation-only 观测不得把 `FreshAnchorRecovered` 立即打回强恢复
- 回退必须同时满足 freshness 失效与当前阶段证据失效
- 回退后重新进入更强恢复动作前，需要经过对应 hold window / refresh interval / unlock reason 门控

### 5. owner / session / coordinator 的职责

#### A. `video_scheduling_owner.rs`

- `PlaybackRecovered` 进入主线后，owner 允许从 `RebuildingSupply` 退出到 `DegradedServing` 或 `StableServing`
- owner 继续保留“fresh anchor 仍缺失”的诊断与恢复 intent
- owner 退出恢复态不等于 transport 停止维护 fresh-anchor pressure

#### B. `session/policy.rs`

- `PlaybackRecovered` 视为“播放恢复完成”的正式证据
- `FreshAnchorRecovered` 视为“媒体恢复完成”的正式证据
- 升级链和吸收链需要区分：
  - 播放已恢复后的 reconnect / decoder reset 压力应下降
  - fresh anchor 仍缺失时，PLI pressure 不能被完全撤掉

#### C. `recovery/coordinator.rs`

- coordinator 继续围绕 transport-local hard evidence 决策
- 本轮不要求 coordinator 直接理解 render 细节
- coordinator 只需要吃统一后的 `RecoveryProgressLevel`

### 6. keyframe family gate 修复并入同一轮改造

本 RFC 将以下两项列为硬交付，而不是后续补丁：

1. `decoded_keyframe_without_clean_anchor_does_not_hold_family_gate_after_hold_window`
2. `non_idr_vcl_keyframe_response_does_not_hold_family_gate`

原因：

- `PlaybackRecovered` 引入后，主线会明确出现“播放已恢复但 fresh anchor 未恢复”的阶段
- 如果 family gate 仍按“keyframe packet 到过 / decoded 过”压住同 family 新请求，主线语义会再次失真
- 浏览器侧之所以更稳，一个关键差异就是它会持续维持“仍缺完整恢复关键帧”的反馈压力

修复要求：

- continuation-only response 不得长期占用 `same-family keyframe in-flight` 门位
- decoded 但无 clean-anchor commit 的旧 episode，在短 hold window 之后不得继续压制当前轮新 PLI
- `bootstrapRejected:invalidBootstrap` 与 `decodedPendingCommitExpired` 都必须成为显式 unlock reason

episode 归属规则：

- 保持“单 unresolved recovery session + owner 前移”的总体策略不变
- family gate 放行时默认优先刷新当前 unresolved episode，而不是无条件新开 episode
- 仅当发生以下情况时才允许强制开新 episode：
  - recovery epoch 已切换
  - 当前 unresolved episode 已明确 retire / expired
  - 显式发生 family upgrade，需要把旧请求语义与新请求语义分离

同一 unresolved episode 内的行为：

- continuation-only response 只更新 blocker / response evidence / unlock reason，不长期持有 keyframe gate
- decoded-no-commit hold 过期后，当前 unresolved episode 允许被 refresh，并前移 owner / request attempt
- trace / stats 继续围绕当前 unresolved episode 记账，避免串单到平行 episode

### 7. 并入 `PLI -> FIR` 升级线路

当前问题：

- 当前恢复主路径以 `PLI refresh` 为主
- 很多发送端对 `PLI` 的响应较软，对 `FIR` 的执行更硬
- 浏览器在恢复窗口里通常会持续提供更强、更密集、更标准化的关键帧恢复压力
- 如果我们只停留在 `PLI refresh`，即使主线已经引入 `PlaybackRecovered`，仍然可能长期停在“可播但缺 fresh anchor”的尾段

本轮将 `FIR` 升级合同并入主线改造，避免后续再做平行补丁。

#### A. 升级目标

- 当系统已经确认：
  - 当前 recovery 仍缺 `FreshAnchorRecovered`
  - 且 `PLI` 已经至少经历一个有效响应窗口
  - 且远端返回的是 continuation-only / invalid bootstrap / decoded-no-commit 长尾
- 系统需要升级到更强的关键帧压力路径，即 `RequestFir`

#### B. 升级证据

`FIR` 升级仅建立在强证据上，不建立在单次超时或单次弱噪声上。

强证据集合：

1. `response seen but no fresh anchor`
- 已看到当前 episode 的 `first_keyframe_packet_at_ms`
- 当前仍无 `FreshAnchorRecovered`

2. `continuation-only sustained`
- `bootstrapMissingIdr / NonIdrVcl`
- `continuationAcceptedWhileAwaitingIdr`
- `committed_sps_present == true`
- `committed_pps_present == true`
- `delta_continuation_ready == true`

3. `decoded-no-commit sustained`
- 已出现 `first_keyframe_decoded_at_ms`
- 超过短 hold window 仍无 `FreshAnchorRecovered`

4. `same-episode replay persistence`
- 同一 recovery episode 内已经出现一次或多次 `PLI refresh`
- 后续仍反复落回 continuation-only / invalid bootstrap blocker

#### C. 分阶段升级条件

升级路径固定为：

- `PLI`
- `PLI Refresh`
- `FIR`

推荐条件：

1. 从 `PLI` 到 `PLI Refresh`
- 当前已有 `response seen` 或 `packets recent`
- 超过 `pli_refresh_interval_ms`
- 当前仍无 `FreshAnchorRecovered`

2. 从 `PLI Refresh` 到 `FIR`
- 满足以下任一强证据组：
  - `response seen + continuation-only sustained + 至少一次 PLI refresh 后仍缺 fresh anchor`
  - `decoded-no-commit sustained + hold window expired + 当前仍缺 fresh anchor`
  - `packet seen / response seen` 多次出现，但同一 recovery episode 一直没有进入 `FreshAnchorRecovered`

补充约束：

- `FIR` 升级要求已经经历过至少一轮 `PLI` 主路径
- `FIR` 不在“完全没 response”的首轮等待窗里直接触发
- `FIR` 不在 `PlaybackRecovered` 首次刚建立的短宽容窗里立即触发，避免刚续播就过度施压

#### D. episode / budget / in-flight 合同

`FIR` 需要并入现有 keyframe family 合同，不能做无预算升级。

episode 规则：

- `FIR` 默认附着在当前 unresolved recovery episode 上执行
- `FIR` 升级不会隐式创建第二条并行 keyframe episode
- `FIR` 发出后，当前 episode 的主请求种类升级为 `fir`
- 后续 `response seen / decoded / clean-anchor` 仍记到同一 unresolved episode，直到 retire 或 epoch 轮换

budget 规则：

- `FIR` 计入现有 keyframe budget family
- 每个 recovery episode 内 `FIR` 默认最多 1 次
- 跨 episode 的 `FIR` 次数继续受 `RecoveryActionBudgetState.keyframe_budget_*` 统一节流
- `FIR` 最小重试间隔必须显式配置，且大于 `PLI refresh interval`

in-flight 规则：

- `PLI in-flight` 可以升级为 `FIR in-flight`
- `FIR` 一旦发出，旧 `PLI in-flight` 不再作为阻止 `FIR` 的 merge 理由
- `FIR in-flight` 只允许：
  - 等待响应
  - 等待 decode
  - 等待 fresh anchor commit
- 若 `FIR` 后仍落回 continuation-only / decoded-no-commit，必须等待 `fir_retry_interval` 或 episode 轮换，禁止每个 tick 重复 `RequestFir`

解锁规则：

- `FreshAnchorRecovered`
- `episode retired / expired`
- `recovery epoch` 切换
- 明确的 `firResponseExpired` 或等价 unlock reason

成功分层：

- 弱成功：
  - `FIR` 后进入 `PlaybackRecovered`
  - 表示播放已恢复，但 keyframe 升级链仍保持可继续状态
- 强成功：
  - `FIR` 后进入 `FreshAnchorRecovered`
  - 表示当前 recovery epoch 已真正拿到 fresh usable IDR 并完成 anchor 提交

关闭规则：

- 弱成功关闭：
  - 关闭 reconnect / decoder reset 级压力
  - 保留 keyframe family 的 fresh-anchor pressure
- 强成功关闭：
  - 关闭 `FIR` 升级链
  - episode 回到常规 post-anchor / display settle 路径
- `FIR` 发出后仅出现弱成功时，不得把 `FIR` 视为已完全收敛

没有这些合同，`FIR` 升级不会稳定。

#### E. 与单线恢复主线的关系

- `PlaybackRecovered` 建立后：
  - reconnect / decoder reset 压力可以下降
  - fresh-anchor pressure 继续保留
  - 关键帧压力路径从“高频 PLI refresh”逐步切到“低频但更强的 FIR 保底”

- `FreshAnchorRecovered` 建立后：
  - `FIR` 升级链立即关闭
  - episode 回到常规 post-anchor / display settle 路径

这保证：

- 不把 `PlaybackRecovered` 当成恢复结束
- 不让“已可播但缺 fresh anchor”无限期停留在弱 `PLI` 路径
- 不把 `FIR` 的弱成功误判为强成功

#### F. 与 family gate 修复的耦合

这条升级线路必须依赖第 6 节的 gate 修复一起落地：

- continuation-only response 不再长期占用 same-family gate
- decoded-no-commit hold window 过期后允许新请求放行
- 放行后 coordinator / session policy 才能真正把动作从 `PLI Refresh` 升级到 `FIR`

否则会出现：

- 决策层已判定应升级 `FIR`
- transport session 仍被旧 same-family gate 压住
- trace 里只看到继续 coalesced，无法形成真实远端压力

#### G. 模块级改法

- `recovery/escalation.rs`
  - 保留 `RequestFir`
  - 为 `TransportAwaitRecoveryAnchor` 增加 continuation-only / decoded-no-commit 长尾后的升级入口
  - 将 `FIR` 接入现有 `RecoveryActionBudgetState` 与 keyframe family budget

- `session/policy.rs`
  - 将 `PlaybackRecovered but fresh anchor missing` 视为保留 keyframe pressure 的合法窗口
  - 将“持续 continuation-only”与“decoded-no-commit hold expired”提升为 `RequestFir` 候选条件

- `transport_session.rs`
  - 保证 `FIR` 与 `PLI` 共用 family gate 修复后的放行规则
  - 不允许旧 `PLI in-flight` 长期压住 `FIR` 升级动作
  - 明确 `FIR` 在当前 unresolved episode 上执行、升级、解锁与 retire 的记账方式

- `runtime_stats_sink.rs` / trace
  - 补齐 `requestFir` 的 episode 叙事
  - 区分：
    - `pliRefresh`
    - `firEscalatedAfterContinuationOnly`
    - `firEscalatedAfterDecodedPendingCommit`

### 8. stats 与 trace 收口

新增或调整的口径：

- 播放恢复指标：以 `PlaybackRecovered` 为主
- 链路修复指标：以 `FreshAnchorRecovered` 为主
- `first-frame latency` 继续只认：
  - `controlReadyToPliSentMs`
  - `pliSentToFirstIdrPacketMs`
  - `firstIdrPacketToFirstDecodeMs`
  - `firstDecodeToCleanAnchorCommittedMs`
  - `cleanAnchorCommittedToDisplayStableMs`

trace 侧新增要求：

- episode / blocker / latency 观测中能区分：
  - continuation-only but playback recovered
  - playback recovered but fresh anchor missing
  - fresh anchor recovered

兼容策略：

- 现有 `RecoveryProgressLevel` 字符串兼容优先，禁止直接改写已有 phase 名导致下游消费断裂
- `CleanAnchorCommitted`
  - 继续保留为现有 DTO/trace phase 名
  - 内部语义可映射为 `FreshAnchorRecovered`
- `DisplayStable`
  - 继续保留为现有 DTO/trace phase 名
- `firstFrameLatencyObserved`
  - 继续沿现有结构
  - 主指标仍只围绕 `CleanAnchorCommitted / DisplayStable`
- `continuationOnlyAwaitingIdr`
  - 不再作为已经进入 `PlaybackRecovered` episode 的终局结论
  - 允许保留为 detail / incomplete_reason / blocker 分类的一部分

字段级迁移规则：

| 观测面 | 字段/事件 | 兼容策略 |
| --- | --- | --- |
| DTO / trace phase | `CleanAnchorCommitted` | 保持原 phase 名，不直接重命名 |
| DTO / trace phase | `DisplayStable` | 保持原 phase 名，不直接重命名 |
| first-frame latency | `terminal_phase` | 只允许出现 `WaitingResponse / ContinuationSeen / AnchorSeen / Decoded / CleanAnchorCommitted / DisplayStable`，不新增 `PlaybackRecovered` |
| first-frame latency | `incomplete_reason` | 允许保留 `continuationOnlyAwaitingIdr`，但仅在未进入 `PlaybackRecovered` 的 episode 上作为终局解释 |
| picture recovery transition | `phase/detail` | 可新增 `PlaybackRecovered` 派生事件或 detail，不覆盖既有主 phase |
| runtime stats / diagnostics DTO | 新字段 | 新增显式字段承载 `playback_recovered` 与 `fresh_anchor_recovered` 口径 |
| frontend / trace consumer | 旧字段消费 | 继续能仅依赖 `CleanAnchorCommitted / DisplayStable` 工作 |

新增字段要求：

- runtime stats 至少新增以下可观测字段之一：
  - `recovery_playback_recovered_at_ms`
  - `recovery_playback_recovered_phase`
  - `recovery_fresh_anchor_recovered_at_ms`
- DTO / diagnostics / trace_projection 需要同步暴露这些新字段
- 新字段优先表达新增语义，旧字段继续表达兼容主链

新增要求：

- `PlaybackRecovered` 优先通过以下方式进入观测面：
  - 新 DTO 字段
  - 现有 event 的 detail / summary / blocker 分类扩展
  - 新增派生 event，但不替换已有主 phase 名
- 如果新增 phase 名，必须满足：
  - 老 phase 名仍可继续被旧消费者解析
  - `trace_projection` 与前端快照同步升级
  - 旧 consumer 至少能回退读到原有 `CleanAnchorCommitted / DisplayStable` 主链

禁止事项：

- 禁止把 `PlaybackRecovered` 直接覆盖写进现有 `CleanAnchorCommitted` phase
- 禁止后端仅修改 phase 字符串而不同步 DTO / trace / frontend consumer
- 禁止让新语义只存在于 trace detail，而 runtime stats / DTO 完全不可见

禁止事项：

- 禁止把 `PlaybackRecovered` 记成 `cleanAnchorCommitted`
- 禁止让 `continuationOnlyAwaitingIdr` 覆盖已经恢复播放的 episode 结论

## Implementation Plan

1. 在 `recovery/contract.rs` 为主线增加 `PlaybackRecovered` 阶段，并明确它与 `FreshAnchorRecovered`、`DisplayStable` 的边界。
2. 保留 `AnchorSeen / Decoded` 为正式主线阶段；修改 `session/facts.rs`，把 `Decoded + continuationAcceptedWhileAwaitingIdr + 纯底层 render/host serviceable evidence` 收敛到 `PlaybackRecovered`，替换当前一律回写 `ContinuationSeen / WaitingResponse` 的逻辑。
3. 修改 `video_scheduling_owner.rs` 与 `session/policy.rs`，让 `PlaybackRecovered` 成为退出 `RebuildingSupply` 的正式主线阶段，同时保留 fresh-anchor pressure。
4. 在 `recovery/escalation.rs`、`session/policy.rs`、`transport_session.rs` 补齐 `PLI -> FIR` 升级线路，并与 continuation-only / decoded-no-commit 长尾证据、episode 归属、budget/in-flight 合同对齐。
5. 修改 `runtime_stats_sink.rs` 与 trace 投影，拆开 playback recovered 与 fresh-anchor recovered 两类恢复完成口径，并补齐 `requestFir` 升级叙事、弱成功/强成功关闭语义。
6. 完成 `transport_session.rs` 的两条 family gate 修复，解除 continuation-only response 和 stale decoded-no-commit 对当前 unresolved episode 内新 PLI/FIR 的长期压制，并保持单 unresolved session 策略不漂移。
7. 补齐单测、集成测试、trace contract 测试，验证“单线主线 + FIR 升级 + gate 修复”同时成立。

## Validation

- `cargo test -p xbxengine transport::rtc::session::facts -- --nocapture`
- `cargo test -p xbxengine transport::rtc::policy::video_scheduling_owner -- --nocapture`
- `cargo test -p xbxengine transport::rtc::session::policy_tests::recovery_integration -- --nocapture`
- `cargo test -p xbxengine transport::rtc::stack::transport_session -- --nocapture`
- `cargo test -p xbxengine runtime_stats_sink::tests -- --nocapture`
- `cargo test -p xbxengine recovery_integration_trace_contract_continuation_heavy_stops_only_after_clean_anchor -- --nocapture`
- 新增并通过：
  - `decoded_keyframe_without_clean_anchor_does_not_hold_family_gate_after_hold_window`
  - `non_idr_vcl_keyframe_response_does_not_hold_family_gate`
  - `continuation_only_serviceable_output_advances_to_playback_recovered`
  - `playback_recovered_does_not_count_as_fresh_anchor_recovered`
  - `continuation_only_persistence_escalates_from_pli_refresh_to_fir`
  - `decoded_pending_commit_persistence_escalates_to_fir_after_hold_window`
  - `fir_upgrade_is_blocked_until_at_least_one_effective_pli_window_has_elapsed`
  - `fresh_anchor_recovered_does_not_immediately_fall_back_on_single_continuation_only_observation`
  - `stale_playback_recovered_evidence_drops_back_to_decoded_or_continuation_seen`
  - `fir_weak_success_keeps_fresh_anchor_pressure_while_disabling_reconnect_pressure`

## Risks

- `PlaybackRecovered` 若定义过宽，会把真正缺 fresh anchor 的硬故障过早“播放成功化”。
- 若 owner 在 `PlaybackRecovered` 后完全停止 keyframe pressure，fresh anchor 可能长期缺失。
- 若 stats/trace 继续混用 `PlaybackRecovered` 与 `FreshAnchorRecovered`，会让故障归因再次漂移。
- `transport_session` family gate 若只修一半，主线会出现“状态已允许继续恢复，动作链仍被旧 gate 压住”的新矛盾。
- `FIR` 升级若条件过宽，会在高 RTT 但可自愈的窗口里过早放大远端压力。
- `FIR` 升级若条件过窄，会继续保留“长期停在 PLI refresh 但永远拿不到 fresh anchor”的浏览器差距。
- 若 `PlaybackRecovered` 的底层事实定义不够纯，会重新引入 `session/facts -> owner -> session/facts` 闭环。
- 若删除或弱化 `AnchorSeen / Decoded`，升级门槛会重新依赖局部 side evidence，主线再次漂移。
- 若 `PlaybackRecovered` 缺少 freshness window，会被旧 render/host 证据粘住，播放恢复态无法正确回退。
- 若 `FIR` 只定义发起条件、不定义弱成功/强成功关闭条件，会在“可播但未 fresh-anchor recovered”窗口反复抖动。

## Progress

- [x] RFC 建立并登记 project-task
- [ ] recovery progress 主线加入 `PlaybackRecovered`
- [ ] 保留 `AnchorSeen / Decoded` 正式主线阶段
- [ ] `facts.rs` 用纯底层 render/host serviceable evidence 收口 `Decoded -> PlaybackRecovered`
- [ ] `PlaybackRecovered` 接入显式 freshness window
- [ ] `PlaybackRecovered` / `Decoded` freshness 并入统一 phase policy 参数面
- [ ] owner / session policy 改为消费单线主线
- [ ] `PLI -> FIR` 升级线路与单线主线对齐
- [ ] `FIR` 接入 episode / budget / in-flight 合同
- [ ] `FIR` 弱成功 / 强成功关闭条件落地
- [ ] stats / trace 拆出 playback recovered 与 fresh-anchor recovered 两类口径
- [ ] 观测兼容策略落地，保持现有 DTO/trace 主 phase 不破坏旧 consumer
- [ ] family gate 修复一：decoded-no-commit hold window 过期后放行新 PLI
- [ ] family gate 修复二：continuation-only / invalid bootstrap 响应不再长期占用 same-family keyframe gate
- [ ] family gate 修复三：旧 `PLI in-flight` 不长期压制 `FIR` 升级动作
- [ ] family gate 与 `latest_keyframe_request_episode` 单 unresolved 策略对齐
- [ ] 回归测试与 trace contract 验证

## Decision Notes

- 决策 1：保留 `bootstrap` 与 `cleanAnchorCommitted` 的硬语义，不以“体验恢复”为理由放松 codec/reference-chain 事实。
- 决策 2：不新增平行 `soft-serving` 状态机；render 价值证据直接进入 recovery 主线。
- 决策 3：两条 keyframe family gate 修复与主线改单线同轮推进，避免“主线修好了、动作链继续卡旧 gate”的半收敛状态。
- 决策 4：`FIR` 只作为 `PLI` 主路径后的强升级动作，不直接替代 `PLI` 成为首轮图片级恢复动作。
