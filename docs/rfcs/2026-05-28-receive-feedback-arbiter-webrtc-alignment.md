# RFC：Receive Feedback Arbiter 与 WebRTC 接收侧对齐

**状态：** Done（代码与单测闭合；live healthy 新采见 [`reports/2026-05-29-receive-feedback-arbiter-trace-validation.md`](../reports/2026-05-29-receive-feedback-arbiter-trace-validation.md)）
**范围：** `xbxengine` receive / recovery contract / session policy / trace projection
**接续：** [2026-05-27 恢复链减法与 libwebrtc 对齐](2026-05-27-recovery-subtraction-libwebrtc-alignment.md)

## 背景

上一轮减法已经把 picture recovery 主权从 session 收回 receive：图片级 PLI/FIR 委托 `DelegatedToReceive`，`RequestDecoderReset` 收窄到 backend / reconfigure / local maintenance，InsertGate 也开始按 reference-complete 纪律 Hold/Drop 无望 delta。

剩余差异集中在责任边界：标准 WebRTC 的视频接收链以 RTP packet、packet repair、frame dependency、decode、render 为主线；当前代码仍有 `MediaSupplyPhase`、displayed-IDR、host mailbox、session owner 等派生事实参与恢复解释。它们对诊断有价值，但不应继续主导图片级恢复动作。

近期 trace 的卡死形态也指向同一问题：RTP 持续入站，NACK/PLI 有动作或合并，但远端长期没有形成 usable IDR；H264 检查持续 `bootstrapMissingIdr` / `outOfRecoveryContextContinuation`，host 只能保留旧帧。架构上需要一个 receive 层的单一 feedback 仲裁点，并让 trace 直接回答“请求是否发出、响应是否到达、是否可解码、是否完成显示闭环”。

## 目标

1. 将图片级恢复动作统一收口到 receive 层的 `ReceiveFeedbackArbiter`。
2. 将 Insert/Decode 准入改为以 packet/frame/reference/decoder 事实为主，display/host 事实只做诊断和投影。
3. 将稀疏 IDR 优化定位为 receive feedback pacing 策略，避免成为新的恢复分支。
4. 让 session 只处理 transport connectivity、backend/reconfigure reset、reconnect candidate，不再拥有 picture recovery。
5. 优化 trace，形成可量化验收：`request -> response -> decoded -> cleanAnchorCommitted -> displayStable`。

## 非目标

- 不照搬 libwebrtc 类名和线程模型。
- 不引入新的 transport / signaling / media pipeline。
- 不把 host mailbox 或 display 事件从诊断中删除。
- 不放宽无参考链 delta 的 Insert/Decode 准入。
- 不把远端长期不给 IDR 的兜底做成 session keyframe epoch。

## 目标架构

```text
RTP AU / PacketBuffer facts
        |
        v
ReferenceChainGate  ---- gap / H264 / decoder facts ----+
        |                                               |
        v                                               |
InsertGate (Emit / HoldRepair / Drop)                   |
        |                                               |
        v                                               |
DecodeGate -> Decoder -> Pacer -> Render/Host           |
                                                        |
NackRequester + KeyframeRequester + SparseIdrRhythm <---+
        |
        v
ReceiveFeedbackArbiter
        |
        v
RTCP feedback: NACK / PLI / FIR

Session: DelegatedToReceive | RequestDecoderReset(backend/reconfigure/local maintenance) | ReconnectCandidate(connectivity)
```

## 后续对齐约束

本 RFC 的第一轮实现已经完成 receive feedback arbiter 与 reference-chain gate 的骨架。下一轮继续向标准 WebRTC 接收侧靠近，按以下约束推进：

- `ReferenceChainState` 从 stats 派生推进到 receive-local ledger 派生，逐步靠近 frame dependency 模型：由 packet gap、H264 inspection、NACK disposition、decoder result 直接推进状态；runtime stats 只作为迁移期 projection fallback。
- `MediaSupplyPhase` 保持为 projection / UI / diagnostics 语言；Insert/Decode 核心准入只读 reference、packet、decoder 事实。
- `ReceiveFeedbackArbiter` 输出保持为唯一图片级 feedback 决策口；旧 `request_receiver_local_keyframe` 入口只提供 source / reason，所有 keyframe / NACK 入口都转成 arbiter 输入。
- trace 验收固定按闭环链路：`sent -> response observed -> decoded -> clean anchor -> display stable`，验收脚本以结果链为准，避免只看请求次数。
- `displayed_idr_*` 继续从 Insert/Decode 主判断里退出，只服务于“用户看见了什么”和 host-present 验收。
- “远端长期不给 usable IDR”的 terminal policy 进入 receive 层：N 次 sent / N RTT 后输出明确 diagnostic，再交给 session 判断 connectivity / reconnect。

## 代码对齐方案

### 1. 新增 ReceiveFeedbackArbiter

位置建议：`crates/xbxengine/core/src/transport/rtc/receive/feedback_arbiter.rs`。

输入：

- `InsertGateDecision` 与 reason。
- `NackRequester` 的 due/exhausted/escalation 结果。
- `SparseIdrRhythm`。
- `FeedbackTargetAvailability`。
- 当前 `ReferenceChainState`。
- 最近一次 receive-local keyframe request 时间与 outcome。

输出：

- `ReceiveFeedbackDecision`：
  - `action`: `None | SendNack | RequestPli | RequestFir`
  - `reason`: `gap-repair | gap-too-large | need-keyframe | sparse-idr | decoder-needs-keyframe | feedback-target-unavailable`
  - `coalescing`: `fresh-sent | same-interval | target-unavailable | rate-limited`
  - `sparse_active`
  - `reference_state`
  - `feedback_target_state`

约束：

- keyframe request 优先级高于 NACK 批量发送。
- NACK 只处理仍有修补价值的 gap。
- PLI/FIR 只由 receive arbiter 发出或合并。
- session 看到 picture recovery 信号时只记录 `DelegatedToReceive` 和 hint。

### 2. 收敛 ReferenceChainState

位置建议：`recovery/contract/insert.rs` 或新模块 `recovery/contract/reference_chain.rs`。

建议状态：

- `Unknown`: 未建立首个可用参考。
- `Continuous`: 当前 decoder/reference chain 可续播。
- `Repairing`: gap 可通过 NACK 修补，delta 暂按价值和 reference 状态裁决。
- `NeedKeyframe`: 无法证明参考链连续，必须等待 IDR。

驱动事实：

- H264 inspection：`bootstrap_ready`、`is_idr`、SPS/PPS、slice header。
- packet gap / NACK：active gap、exhausted、stuck sequence。
- decoder fact：`decoder_reference_synced`、backend no-output、waiting-keyframe。
- runtime age：`submit_age_ms` 只作为 supply starvation 诊断和 `NeedKeyframe` 的辅助证据。

对齐动作：

- `MediaSupplyPhase` 降级为对外投影，不再作为 Insert/Decode 准入的主要输入。
- `displayed_idr_host_hint` 只保留 trace 解释，不参与 `decodable_to_feed` 的核心判断。
- `PacketRecoveryActionStage::Drop` 拆成 `Steady` / `Drop` 或改名为 `Steady`，避免默认稳态与真实丢弃混用。

### 3. 稀疏 IDR 作为 pacing 策略

当前 `SparseIdrRhythm` 方向保留，但归属收进 `ReceiveFeedbackArbiter`。

保留行为：

- `pli_interval_ms = clamp(0.5 * rtt, 12ms, 40ms)`。
- `receive_keyframe_last_sent_at_ms` 控制 coalescing。
- NACK stuck/exhausted 在 sparse active 时 `arm_immediate`。

收敛点：

- 删除 `KeyframeRequester` 中未进入主路径的 `sparse_pli_retry_interval`、`should_request_keyframe`、`request_if_due`。
- `KeyframeRequester` 只负责实际发送和本地退避计数。
- sparse active 的判定由 `ReferenceChainState::NeedKeyframe` 与 `MediaSupplyPhase::MustIdr` 双向校验，trace 中两者不一致时输出 mismatch。

### 4. Session 边界

保留：

- connectivity / liveness / reconnect candidate。
- backend failure / reconfigure / local decoder maintenance reset。
- receive hint 投影：`DelegatedToReceive`、`recovery_picture_recovery_authority=receive`。

收紧：

- session 不生成 PLI/FIR。
- session 不维护 picture keyframe in-flight。
- session 不用 display/host stall 推导 picture recovery action。
- `RequestDecoderReset` 不处理 reference-chain / sparse-IDR / supply-starved 证据链。

## Trace 优化方案

### 1. 新增 receiveFeedbackDecision

事件：`receiveFeedbackDecision`

关键字段：

- `action`
- `reason`
- `source`
- `outcome`
- `coalescing`
- `feedbackTargetState`
- `referenceState`
- `sparseActive`
- `sparsePliIntervalMs`
- `lastKeyframeSentAgeMs`
- `gapSequence`
- `nackPacketCount`
- `h264Verdict`

投影要求：

- minimal trace 只记录状态变化、sent、terminal、mismatch。
- repeated coalesced/throttled 聚合计数，避免每拍刷屏。
- raw 诊断模式保留逐拍细节。

### 2. 新增 referenceChainState

事件：`referenceChainStateChanged`

关键字段：

- `state`: `unknown | continuous | repairing | need-keyframe`
- `cause`
- `decoderReferenceSynced`
- `bootstrapReady`
- `bootstrapRejectReason`
- `hasActiveGap`
- `nackExhausted`
- `submitAgeMs`
- `displayedIdrHostHint`

用途：

- 解释 InsertGate 为什么 Hold/Drop/Emit。
- 解释 sparse IDR 为什么 active。
- 解释 session 为什么只 `DelegatedToReceive`。

### 3. Keyframe effectiveness 观测

核心恢复对齐 receive ledger 链路：

```text
keyframe_required -> Pli/FirSent -> ResponseObserved/PacketSeen -> Decoded -> CleanAnchorCommitted -> DisplayStable
```

trace 必须直接回答：

- 请求被 sent、coalesced、throttled、target-unavailable 中哪一种。
- 首个响应是 IDR、non-IDR、invalid H264、无响应。
- IDR 是否进入 decode。
- clean anchor 是否 committed。
- display 是否 stable settled。
- 超过 N 个 RTT 后的 terminal reason。
- `episodeId` 只作为旧 projection / effectiveness grouping，恢复准入和 terminal 判定读取 receive ledger。

### 4. 验收脚本

建议新增：`scripts/trace_receive_feedback_report.py`。

输出：

- keyframe sent/coalesced/throttled/target-unavailable 计数。
- response observed rate。
- usable IDR rate。
- decoded rate。
- chain build success rate。
- NACK effective rate。
- sparse active P95 dwell。
- `NeedKeyframe` 下非 IDR delta feed 次数。
- session picture recovery ownership violation 次数。
- decoder reset violation 次数。

## 分阶段计划

### Phase 0：减法清理

- 清理 warning 和旧 API。
- `contract/mod.rs` 改 `pub(crate) use`。
- 删除 `KeyframeRequester` 未用字段/API。
- 拆分或改名 `PacketRecoveryActionStage::Drop`。

### Phase 1：trace-only arbiter

- 新增 `ReceiveFeedbackArbiter` 数据结构和 decision trace。
- 先旁路观察，不改变实际发送路径。
- 与当前 `keyframeRequestOutcome` 对照，确认 action/reason 可解释。

### Phase 2：发送路径切换

- 将 NACK escalation、InsertGate hold repair、media supply MustIdr 的 PLI/FIR 请求统一经 arbiter。
- 保留现有 `KeyframeRequester` 和 `NackRequester` 作为执行器。
- session picture recovery 继续只输出 `DelegatedToReceive`。

### Phase 3：ReferenceChainGate 收口

- 引入 `ReferenceChainState`。
- `decodable_to_feed` 使用 reference state 作为主输入。
- `MediaSupplyPhase` 变成 projection，不再吞掉 repairing / need-keyframe 的内部事实。

### Phase 4：trace gate 与旧 trace 回放

- 新增 `trace_receive_feedback_report.py`。
- 用 `runtime-trace-1779953007765-1.jsonl` 作为回归样本。
- 将结果回填到 report。

### Phase 5：Receive ledger 与闭环验收深化

- 新增或扩展 receive ledger：记录 RTP gap、recovered packet、H264 inspection、decoder reference sync、keyframe response 绑定关系。
- `ReferenceChainState` 优先从 receive ledger 派生；stats 只作为 projection fallback 与迁移期 mismatch 检查。
- 收紧 `request_receiver_local_keyframe`：仅保留 source/reason 入口，实际动作完全由 `ReceiveFeedbackArbiter` 决策。
- `MediaSupplyPhase` 从 Insert/Decode 准入路径退出，仅服务 diagnostics、UI、trace summary。
- `receiveFeedbackDecision` / `referenceChainStateChanged` 补齐 `feedbackTargetState`、gap、NACK、H264 verdict 与 sparse active 明确字段。
- `trace_receive_feedback_report.py` 只按真实链路阶段计数：sent、usable response、decode success、clean anchor、display stable。
- receive 层新增 terminal diagnostic：同一 unresolved picture recovery 在连续 N 次 sent 或 N 个 RTT 后仍没有 usable IDR / decode / clean anchor 时，输出 `remote-no-usable-idr` 一类明确原因。

## 验收标准

- `cargo test -p xbxengine --lib` 通过。
- `cargo test -p xbxrc --lib trace_projection -- --test-threads=1` 通过。
- warning 数量下降，receive/recovery 新增 warning 为 0。
- `RequestDecoderReset` 只在 backend/reconfigure/local maintenance 出现。
- `recovery_session_keyframe_in_flight=false` 在 picture recovery 场景保持成立。
- `NeedKeyframe` 下非 IDR delta `decodableToFeed` 为 0。
- sparse active 时 PLI interval 落在 12ms 到 40ms。
- 远端无 usable IDR 时输出明确 terminal diagnostic。
- trace 可给出 keyframe effectiveness、NACK effectiveness、chain build success rate。

## 风险

- `MediaSupplyPhase` 降级为投影会影响现有 diagnostics 面板解释，需要 trace projection 同步过渡。
- trace-only arbiter 与实际发送路径短期并存，可能出现字段 mismatch；Phase 1 必须显式记录 mismatch。
- 稀疏 IDR 过快会放大 RTCP 频率；必须依赖 coalescing 与 feedback target readiness。
- ReferenceChainState 过严会拉长首帧恢复；必须保留 bootstrap IDR 的快速通道。

## 进度

- [x] Phase 0：减法清理
- [x] Phase 1：trace-only arbiter
- [x] Phase 2：发送路径切换
- [x] Phase 3：ReferenceChainGate 收口
- [x] Phase 4：trace gate 与旧 trace 回放
- [x] Phase 5：Receive ledger 与闭环验收深化

## 执行记录

- 2026-05-28：创建 RFC。决策：按 WebRTC receive ownership 对齐，核心动作统一进入 `ReceiveFeedbackArbiter`；trace 从请求计数升级为 request/response/decode/display 闭环验收。
- 2026-05-28：实现完成。`PacketRecoveryActionStage::Steady`；`feedback_arbiter.rs` + `reference_chain.rs`；`receiveFeedbackDecision` / `referenceChainStateChanged` trace；`scripts/trace_receive_feedback_report.py`。验证：`cargo test -p xbxengine --lib`（996 passed）、`cargo test -p xbxrc --lib trace_projection -- --test-threads=1`（67 passed）。
- 2026-05-28：追加后续对齐约束。决策：第一轮 `ReferenceChainState` 骨架保留，下一轮把主事实源从 stats projection 推进到 receive ledger；`MediaSupplyPhase` 固定为 projection；trace gate 固定按 sent/response/decode/clean-anchor/display-stable 闭环验收。
- 2026-05-28：追加 Phase 5 明确合同。决策：receive ledger 驱动 reference state，`request_receiver_local_keyframe` 收敛为 source/reason，`displayed_idr_*` 仅作为 host-visible 验收，远端长期不给 usable IDR 的 terminal diagnostic 由 receive 层产出。
- 2026-05-28：Phase 5 落地。Insert/arbiter 统一 `trace_ledger.reference_chain_observation`；`MediaSupplyPhase`/`displayed_idr_*` 标为 projection-only；`receiveFeedbackDecision` 补 `outcome`/`lastKeyframeSentAgeMs`；`referenceChainStateChanged` 扩展字段；`receivePictureRecoveryTerminal`（`remote-no-usable-idr`，阈值 `sent≥5` 或 `3×effective_rtt`）；脚本 `rates` + `RECEIVE_FEEDBACK_GATE`；coordinator violation 计数。验证：`cargo test -p xbxengine --lib`（1000 passed）、`cargo test -p xbxrc --lib trace_projection`（68 passed）。
- 2026-05-29：简化改造清单重排。决策：`episode` 从核心恢复绑定降级为 diagnostics / effectiveness projection；下一步实现以 receive-local `keyframe_required`、packet/frame/decode/anchor ledger、terminal policy 为主线，任意 usable keyframe / clean anchor 都可关闭恢复链。
- 2026-05-29：补充 WebRTC 源码借鉴与改造顺序。决策：按 `RtcpFeedbackBuffer` batching、`keyframe_required_`、PacketBuffer continuity、NackRequester receive-local escalation、VideoStreamBufferController keyframe release discipline 设计 6 步切换门，优先拆除 coordinator / owner 的图片级跨边界调度。
