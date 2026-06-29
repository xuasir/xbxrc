# RFC Checklist：Receive Feedback Arbiter 简化落地修复

**关联 RFC：** [2026-05-28 Receive Feedback Arbiter 与 WebRTC 接收侧对齐](2026-05-28-receive-feedback-arbiter-webrtc-alignment.md)
**关联 Report：** [2026-05-29 Receive Feedback Arbiter 简化落地修复](../reports/2026-05-29-receive-feedback-arbiter-landing-fix.md)
**状态：** Done（代码与单测已闭合；live trace 回放待补采）
**目标：** 按 WebRTC-like receive 模型完成减法：核心恢复由 receive-local ledger、`keyframe_required`、packet/frame dependency、decoder result 驱动；`episode` 降级为 trace effectiveness / legacy projection。

## 完成情况摘要

| 范围 | 状态 | 说明 |
|------|------|------|
| Step 1–6 主改造 | Done | ledger / arbiter / decode 回写 / coordinator / owner / trace gate |
| Recovery Epoch 闭环六点 | Done | epoch 回收、统一 complete、response observed、display 单入口、insert 单轨、闭环单测 |
| 审查修复（第五轮） | Done | decode-sync epoch 清场；owner release 只认 receive complete |
| P1 Supply Starved 上游 blocker | Partial | terminal / no-clean-anchor 已落地；`displaySupplyStarved` 细粒度归因字段待补 |
| Live trace 验收 | Partial | 本地 replay/continuation/healthy 已跑脚本；continuation terminal=38；healthy 待当前构建新采 |

**单测（2026-05-29）：** `cargo test -p xbxengine --lib` → 1032 passed

## 当前失败画像

新 trace `runtime-trace-1779961935840-1.jsonl` 暴露出 Phase 5 落地缺口：

- `keyframeRequestOutcome`: `sent=98`，`coalesced=1`
- `receiveFeedbackDecision`: 23 条，少于 keyframe outcome
- `keyframeChain`: `responseObserved=0`，`decoded=0`，`cleanAnchorCommitted=0`，`displayStable=0`
- `referenceChainStateChanged`: 只出现 `unknown -> repairing`
- `receivePictureRecoveryTerminal(remote-no-usable-idr)`: 0
- H264 观察持续为 non-IDR continuation，decode 输出停止，host 继续显示旧帧

修复目标是消除 silent stuck：远端持续不给 usable IDR 时，receive 层必须输出明确 terminal diagnostic；远端给出任意 usable keyframe / clean anchor 时，恢复链可以直接闭合，无需依赖旧 episode 绑定。

## WebRTC 源码借鉴点

参考源码：

- [`RtpVideoStreamReceiver2::RtcpFeedbackBuffer`](https://webrtc.googlesource.com/src/+/refs/heads/main/video/rtp_video_stream_receiver2.cc)：RTCP feedback 用 pending flag + batching 统一发送，keyframe request 优先于 NACK。
- [`RtpVideoStreamReceiver2::OnAssembledFrame`](https://webrtc.googlesource.com/src/+/refs/heads/main/video/rtp_video_stream_receiver2.cc)：首帧前收到 delta 立即 request keyframe，后续交给 frame dependency / reference finder。
- [`VideoReceiveStream2`](https://webrtc.googlesource.com/src/+/refs/heads/main/video/video_receive_stream2.cc)：decode result / decodable timeout 回写 `keyframe_required_`，decode OK 清除，decode fail / timeout 设置并触发 keyframe request。
- [`VideoStreamBufferController`](https://webrtc.googlesource.com/src/+/refs/heads/main/video/video_stream_buffer_controller.cc)：`keyframe_required_` 时优先寻找并释放完整 keyframe，跳过普通 release cadence。
- [`PacketBuffer`](https://webrtc.googlesource.com/src/+/refs/heads/main/modules/video_coding/packet_buffer.cc)：packet continuity、missing packet、H264 SPS/PPS/IDR 共同决定 frame 是否可释放。
- [`NackRequester`](https://webrtc.googlesource.com/src/+/refs/heads/main/modules/video_coding/nack_requester.cc)：NACK list、RTT retry、list overflow -> keyframe request 都在 receive/network 侧闭合。

落地启发：

- [x] receive 层拥有 media recovery：packet/frame dependency、NACK、PLI/FIR、keyframe_required、terminal 都在 receive 内闭合。
- [x] decode 只回写 decode result：`ok` / `invalid-data` / `waiting-keyframe` / `no-output-streak`，由 receive ledger 设置或清除 `keyframe_required`。
- [x] display / owner 只拥有 presentation：decode 后 release、host submit、display stable、old-frame retained 都作为显示事实和 diagnostics。
- [x] session 只拥有 connectivity：reconnect、transport reset、backend reset、decoder-owned reset 作为 session 层动作。
- [x] episode 是 trace grouping：恢复准入、terminal、clean-anchor closure 全部读取 receive ledger facts。

## 全局不变量

- [x] 图片级 NACK / PLI / FIR 的唯一决策口是 `ReceiveFeedbackArbiter`。
- [x] 旧 `request_receiver_local_keyframe` 入口只提供 `source` / `reason` / `force` / `soft` 输入。
- [x] `ReferenceChainState` 由 receive-local ledger 事实推进；stats 只做迁移期 fallback 与 mismatch projection。
- [x] `keyframe_required` 是 receive 层核心恢复闩锁：由 packet gap、H264 inspection、NACK disposition、decoder result 设置，由 usable keyframe / clean anchor / decoder synced 清除。
- [x] `episode` 只服务 trace effectiveness、旧字段兼容与人工排障。
- [x] `MediaSupplyPhase` 只服务 UI / diagnostics / trace summary。
- [x] `displayed_idr_*` 只服务 host-visible 验收与用户实际看见什么。
- [x] session 只处理 connectivity / reconnect / backend reset，不生成 picture-level PLI/FIR。
- [x] terminal policy 归 receive 层，session 只消费 terminal 作为 connectivity / reconnect hint。

## 改造顺序

### Step 1：建立 receive recovery ledger 主结构

- [x] 在 `ReceiverTraceLedger` 或新 `ReceiveRecoveryLedger` 中落 `keyframe_required`、`response_state`、`nack_state`、`decoder_result`、`clean_anchor_state`、`terminal_state`。
- [x] 保留旧 stats / episode 字段写入，新增 ledger projection 字段，先做双写。
- [x] `ReferenceChainState` 从 ledger 投影；stats fallback 仅记录 `referenceStatsFallbackTotal`。

切换门：

- [x] 单测证明 first delta / NACK exhausted / decoder invalid 都能设置 `keyframe_required`。
- [x] 单测证明 usable IDR / decoder synced / clean anchor committed 都能清除 `keyframe_required`。

### Step 2：ReceiveFeedbackArbiter 改读 keyframe_required

- [x] `ReceiveFeedbackArbiterInput` 增加 `keyframe_required`、`keyframe_required_cause`、`response_state`、`ledger_generation`。
- [x] `SparseIdrRhythm` active 改读 `keyframe_required + response_state + RTT pacing`。
- [x] `request_receiver_local_keyframe` 保持 thin wrapper，只写 source/reason，再进入 arbiter。

切换门：

- [x] keyframe / NACK action 都能在 `receiveFeedbackDecision` 看到 ledger cause。
- [x] `NeedKeyframe` 只作为 projection，arbiter 主判断读取 `keyframe_required`。

### Step 3：Decode result 回写 receive ledger

- [x] insert gate accepted usable keyframe -> ledger response `usable-idr`。
- [x] decoder OK / reference synced -> clear `keyframe_required`。
- [x] decoder invalid / waiting-keyframe / no-output streak -> set `keyframe_required` 或 terminal candidate。
- [x] clean anchor submitted / committed / rejected -> ledger anchor state。

切换门：

- [x] decoded keyframe 无 clean anchor 时输出 `no-clean-anchor-after-decode`。
- [x] clean anchor committed 可关闭 unresolved terminal state。

### Step 4：Session / Coordinator 退出图片级恢复

- [x] `RecoveryCoordinator::check_idr_completed` 改读 receive ledger projection（`completion.rs` 统一谓词）。
- [x] `latest_transport_await_response_observed_at_ms` 改读 receive ledger response facts（H264 + `bound_recovery_epoch`）。
- [x] `RecoveryAction::RequestPli` / `RequestFir` 在 session path 保持 delegated / suppressed。
- [x] receive terminal 只变成 reconnect/connectivity candidate hint。

切换门：

- [x] `sessionPictureRecoveryViolations=0`（单测 / trace gate）。
- [x] `recovery_session_keyframe_in_flight=false` 在 picture recovery 场景保持成立。

### Step 5：VideoSchedulingOwner 退出 transport-await 推导

- [x] owner 删除或降级 `transportAwait*` hard rebuild evidence，改读 receive `mediaRecoveryProjection`。
- [x] owner 只负责 `decode -> pacer -> host present` 的 presentation release 与 display stable。
- [x] old-frame retained 只证明 display visible，不关闭 media recovery。
- [x] `has_current_clean_anchor_release_evidence` 只认 `receive_picture_recovery_complete_from_fields`（不再 clean-anchor 单独短路）。

切换门：

- [ ] `displaySupplyStarved` 必须带上游 blocker：waiting usable IDR / decoded no clean anchor / clean anchor no host submit / host retained old frame。（diagnostics 细字段待补）
- [x] host present 旧帧保活不会让 receive recovery 关闭。

### Step 6：Trace / episode projection 清理

- [x] trace report 以 ledger generation / RTP / decode / anchor / display 计算链路闭环。
- [x] episode grouping 只保留 effectiveness 统计。
- [x] report 拆开 `arbiterMismatchTotal`、`sparseMustIdrMismatchTotal`、`referenceStatsFallbackTotal`。
- [x] 新增 `epochIsolationViolations` / `sameLedgerGenerationClosure` gate。

切换门：

- [x] failing trace 回放：`1779961935840` 仍 FAIL `silentStuck`（基线）；`1780024427780` 输出 38× terminal（[`trace-validation`](../reports/trace-validation/)）
- [ ] healthy trace 可闭合 `sent -> response -> decoded -> cleanAnchor -> displayStable`。（本地代理 trace 有 anchor，无 DisplayStable；待当前构建新采）

## Recovery Epoch 闭环（2026-05-29 追加）

### 1. epoch 回收

- [x] `apply_clear_receive_recovery_projection` 清空 receive 投影 + `latest_h264_inspection_observation` + decode-sync 字段。
- [x] `ReceiveRecoveryLedger::sync_to_stats` reset 后显式写回 `None`/`0`（含 `receive_keyframe_last_sent_at_ms`）。
- [x] ingress `queue_transport_observation` / `recv_frame_inner` 立即 `sync_recovery_ledger_to_stats`。

### 2. 统一完成判定

- [x] 新增 `recovery/contract/completion.rs`：`receive_picture_recovery_complete*`。
- [x] coordinator / owner 共用同一谓词；删除「receiver 离开 waiting 即完成」fallback。

### 3. response observed 收窄

- [x] `latest_transport_await_response_observed_at_ms` 要求 H264 `bound_recovery_epoch == transport_recovery_epoch`。
- [x] `sent_at_ms` 永不参与 response observed。

### 4. display 单入口

- [x] `record_displayed_idr_fact` 为 stats display-stable 唯一写入口。
- [x] `maybe_ack_clean_anchor_commit_from_runtime_stats` 只回写 ledger。
- [x] `apply_host_display_facts_from_stats` 仅当前 epoch clean anchor 才写 display-stable。

### 5. feedback / insert 单轨

- [x] `plan_receive_feedback` / `execute_receive_feedback_keyframe` 为唯一发送口。
- [x] `build_insert_context` 的 `action_stage` 改读 `ledger.derive_packet_recovery_action_stage`。
- [x] `InsertContext::from_ledger_inputs` 作为 ledger-first 构造入口。

### 6. 闭环级验收

- [x] epoch 切换后旧 display-stable / usable-idr 不延续（单测）。
- [x] 同轮 sent → response → decode sync → anchor → display 闭合（单测）。
- [x] 长期 no-idr → terminal reason（ledger 单测 + trace gate）。
- [x] trace `epochIsolationViolations` / `sameLedgerGenerationClosure` gate。

### 审查修复（第五轮）

- [x] epoch 切换清空 `recovery_decoder_reference_synced_at_ms` / `latest_video_decode_ok_*`。
- [x] `decoder_reference_synced_for_recovery_epoch`：sync 不早于当前 episode opened_at。
- [x] owner release 移除 clean-anchor 单独短路。

## P0：Receive Ledger 与 keyframe_required 主模型

### 修改项

- [x] 在 `ReceiverTraceLedger` 中新增 receive-local recovery state：
  - `keyframe_required: bool`
  - `keyframe_required_cause`
  - `last_keyframe_request_sent_at_ms`
  - `unresolved_keyframe_request_count`
  - `response_state`
  - `last_usable_keyframe_rtp`
  - `last_clean_anchor_rtp`
  - `last_decoder_reference_synced_at_ms`
- [x] `keyframe_required` 设置条件收敛为 WebRTC-like 事实：
  - 首帧收到 delta / non-IDR continuation
  - hard gap NACK exhausted / stale / gap too large
  - H264 bootstrap 缺 IDR / SPS / PPS
  - decoder waiting-keyframe / invalid data / no-output streak
  - sent 后长期只有 continuation / unusable IDR / no decode
- [x] `keyframe_required` 清除条件只接受 receive/decode/anchor 事实：
  - usable IDR / recovery keyframe accepted by insert gate
  - decoder output reference synced
  - clean anchor committed
  - recovery reset / stream reset 开新世代
- [x] `ReferenceChainState` 从 ledger 投影：
  - `Unknown`: 尚未建立首个 usable anchor
  - `Repairing`: gap 仍有 NACK 修补价值
  - `NeedKeyframe`: `keyframe_required=true`
  - `Continuous`: decoder/reference synced 或 clean anchor committed
- [x] PLI/FIR sent 只更新 ledger 的 request facts；恢复成功由后续 packet/decode/anchor facts 关闭。
- [x] unsolicited usable keyframe 可以直接清除 `keyframe_required` 并推进 clean anchor 候选。

### 完成定义

- [ ] 新 trace 中 `keyframeRequestOutcome(sent)` 后 ledger 出现 `keyframe_required=true` 或明确 deferred cause。（live trace 待回放）
- [x] `keyframe_required=true` 时 non-IDR continuation 不会进入 decoder 主准入。
- [x] 任意 usable keyframe / clean anchor 可关闭恢复，不要求 episodeId 存在。
- [x] `ReferenceChainState` 可从 `repairing` 推进到 `need-keyframe` / `continuous` / terminal。

### 回归测试

- [x] first delta frame sets `keyframe_required` and requests PLI through arbiter。
- [x] non-IDR continuation after sent keeps `keyframe_required` and advances response state。
- [x] usable IDR without episode clears `keyframe_required` and enters clean-anchor candidate。
- [x] decoder invalid/no-output streak sets `keyframe_required` and terminal candidate。
- [x] feedback unavailable records deferred/terminal detail without claiming sent。

## P1：Receive Ledger 事实覆盖补齐

### 修改项

- [x] 扩展 `ReceiverTraceLedger` 为 receive-local picture ledger，记录以下事实：
  - RTP gap: observed / repair-in-flight / recovered / exhausted / stale
  - H264 inspection: `is_idr`、`bootstrap_ready`、`bootstrap_reject_reason`、`continuation_verdict`
  - keyframe response: no packet / non-IDR only / IDR unusable / usable IDR
  - decoder result: decode ok / ffmpeg invalid data / no-output streak / waiting-keyframe
  - clean anchor: submitted / committed / rejected
  - display: host-visible / stable settled
- [x] `reference_chain_observation` 优先从 ledger 推导状态。
  - `Unknown`: 首个 usable anchor 尚未建立
  - `Repairing`: hard gap 存在且 NACK 仍有修补价值
  - `NeedKeyframe`: NACK exhausted、hard bootstrap evidence、decoder waiting/no-output、sent 后长期只有 non-IDR
  - `Continuous`: decoder reference synced 或当前 recovery epoch clean anchor 已 committed
- [x] stats fallback 只在 ledger 缺事实时使用，并输出 `referenceChainStatsFallback=true`。
- [x] H264 `outOfRecoveryContextContinuation` 在 active recovery / sent 后应推进 ledger 的 response 事实。
- [x] decoder `ffmpegInvalidData` / no-output streak 应推进 `NeedKeyframe` 或 terminal 候选。

### 完成定义

- [ ] 新 trace 在连续 non-IDR / no-output 场景中从 `repairing` 推进到 `need-keyframe`。（live trace 待回放）
- [x] `referenceChainStateChanged` 带出 cause：`keyframe-sent-non-idr-only` / `decoder-no-output-streak` / `nack-exhausted` / `usable-idr-accepted`。
- [x] `ReferenceChainState::Continuous` 只在 receive/decode/anchor 事实成立时出现。
- [x] `NeedKeyframe` 下 non-IDR `insertGateDecision=emit` 计数为 0（trace gate `needKeyframeNonIdrFeedViolations`）。

### 回归测试

- [x] non-IDR continuation after sent promotes `NeedKeyframe` after threshold。
- [x] decoder invalid/no-output streak promotes `NeedKeyframe`。
- [x] NACK recovered returns `Repairing -> Continuous` only after reference/decode fact。
- [x] clean anchor committed clears unresolved keyframe count and returns `Continuous`。

## P1：ReceiveFeedbackArbiter 单口径收敛

### 修改项

- [x] `request_receiver_local_keyframe` 收成 thin wrapper，只调用 `plan_receive_feedback` 与 `execute_receive_feedback_keyframe`。
- [x] first-frame acquisition、insert-gate hold repair、NACK escalation、soft hint、maintenance retry 全部使用同一 arbiter input。
- [x] `ReceiveFeedbackDecision` 增加 `keyframe_required`、`terminal_candidate`、`response_state`、`ledger_generation` 字段；`episode_id` 仅作为 optional diagnostic field。
- [x] `ReceiveFeedbackDecision` 与实际 executor outcome 做同拍记录。
- [x] NACK 与 keyframe 优先级固定：
  - `NeedKeyframe` 优先 PLI/FIR
  - repairable gap 优先 NACK
  - gap too large / exhausted 进入 PLI/FIR
  - feedback target unavailable 输出 deferred terminal detail

### 完成定义

- [ ] trace 中 `receiveFeedbackDecisionEvents >= keyframeRequestOutcomeEvents`，允许纯 NACK decision 额外增加。（live trace 待回放）
- [x] `arbiterMismatchTotal=0` 与 `sparseMustIdrMismatchTotal` 分开统计。
- [x] 所有 keyframe request source 都能在 `receiveFeedbackDecision.source` 中出现。
- [x] session picture recovery violation 维持 0。

### 回归测试

- [x] insert-gate hold repair goes through arbiter。
- [x] first-frame acquisition goes through arbiter。
- [x] NACK escalation goes through arbiter and updates keyframe request ledger on sent。
- [x] `NeedKeyframe` blocks `SendNack` decision。

## P1：Remote No Usable IDR Terminal Policy

### 修改项

- [x] terminal 判定基于 receive picture ledger，同时消费 `ReferenceChainState::NeedKeyframe` 投影。
- [x] terminal 输入包含：
  - unresolved sent count
  - elapsed RTT count
  - response state: no packet / non-IDR only / IDR unusable / decode failed / no clean anchor
  - last keyframe sent age
  - current reference state
- [x] terminal reason 明确分层：
  - `remote-no-response`
  - `remote-continuation-only`
  - `remote-no-usable-idr`
  - `remote-idr-unusable`
  - `decoder-rejected-idr`
  - `no-clean-anchor-after-decode`
- [x] terminal 输出 `receivePictureRecoveryTerminal` 后，session 只消费为 connectivity / reconnect candidate hint。
- [x] usable IDR / clean anchor / display stable 会清空 unresolved terminal state。

### 完成定义

- [x] 新 trace 的 continuation-only 场景输出 `receivePictureRecoveryTerminal`，reason 为 `remote-continuation-only` 或 `remote-no-usable-idr`。（`1780024427780`：terminal=38，`remote-continuation-only`）
- [x] terminal 出现后 trace 中 `sentCountUnresolved`、`lastKeyframeSentAgeMs`、`referenceState`、`responseState` 可解释。
- [x] terminal 触发 session connectivity / reconnect hint，picture-level feedback 继续归 receive arbiter。
- [x] terminal 可被后续 usable IDR / clean anchor 正确关闭。

### 回归测试

- [x] sent >= 5 with only non-IDR responses emits terminal。
- [x] elapsed >= 3 RTT with no usable response emits terminal。
- [x] usable IDR resets terminal counter。
- [x] terminal delegates reconnect decision to session without picture recovery action。

## P1：Supply Starved 闭环补强

### 修改项

- [x] 新增 supply-starved 闭环：当 `presentAgeMs` 新鲜但 `submitAgeMs` / `decodeAgeMs` 长期增长时，必须把它解释为“旧帧保活 + 新媒体供给断裂”。
- [x] usable IDR / decoded keyframe / config-change keyframe 必须写入 receive response ledger；`episodeId` 只做存在时的观测关联。
- [x] decoded keyframe 后必须进入 clean-anchor candidate；超过 clean-anchor patience 仍无 commit 时输出 `no-clean-anchor-after-decode`。
- [ ] clean anchor committed 后必须驱动 pacer / render / host mailbox 新提交；超过 display-stable patience 仍只有旧帧时输出 `no-display-stable-after-anchor`。
- [x] host mailbox `retainedDisplayed` 只能证明用户仍看见旧帧，不能关闭 media recovery。
- [ ] `displaySupplyStarved` 必须携带上游归因：
  - `waiting-usable-idr`
  - `decoded-no-clean-anchor`
  - `clean-anchor-no-host-submit`
  - `host-retained-old-frame`
  - `decode-actor-stopped`
  - `sample-builder-overflow`
- [ ] `sample_builder` overflow / decode actor panic 必须进入独立 runtime diagnostic，避免被归入远端 IDR 或 display supply。

### 完成定义

- [ ] `displaySupplyStarved` 状态下可以从 trace 直接读出卡在 `usable-idr`、`decoded`、`clean-anchor`、`host-submit`、`display-stable` 的哪一段。
- [x] decoded keyframe config-change 后，trace 要么闭合到 clean anchor/display stable，要么输出 terminal reason。
- [x] `hostMailboxState.presentAgeMs` 新鲜但 `submitAgeMs` 很老时，diagnostics 明确标记 `old-frame-retained`。
- [ ] 用户退出后的 decode actor panic 不影响前面的 root cause 判定；若 panic 发生在退出前，它成为 P0 runtime bug。

### 回归测试

- [x] decoded keyframe without clean anchor emits `no-clean-anchor-after-decode`。
- [ ] clean anchor without host submit emits `clean-anchor-no-host-submit`。
- [x] old retained frame keeps presentation healthy but keeps media recovery open。
- [ ] sample builder overflow records runtime diagnostic and does not masquerade as remote-no-usable-idr。

## P2：MediaSupplyPhase 与 displayed_idr_* 降级为 Projection

### 修改项

- [x] `SparseIdrRhythm` 的 active 判定改为只读 `ReferenceChainState` / receive ledger picture state。
- [x] `MediaSupplyPhase::MustIdr` 只输出 mismatch / diagnostics，不作为核心 pacing 事实。
- [x] `has_current_clean_anchor_from_stats` 与 owner release 分离：
  - clean anchor 读取 decode/anchor committed fact
  - displayed IDR 读取 host-visible fact
- [x] `displayed_idr_serving_*` 只影响 UI/diagnostics 和 host-visible completion，不影响 Insert/Decode/arbiter 主准入。
- [x] `VideoSchedulingOwner` 的 anchor issue release 使用 clean-anchor/display-stable 分层事实（receive complete 谓词）。

### 完成定义

- [x] `MediaSupplyPhase` 改变不会直接改变 `ReceiveFeedbackDecision.action`。
- [x] `displayed_idr_*` 改变不会让 `ReferenceChainState` 从 `NeedKeyframe` 回到 `Continuous`。
- [x] host present 旧帧保活不会掩盖 decode/anchor 停滞。
- [x] diagnostics 仍保留用户看见旧帧的解释。

### 回归测试

- [x] media supply must-idr mismatch records diagnostic only。
- [x] displayed IDR host hint does not commit clean anchor。
- [x] owner release requires clean anchor or display stable fact（`clean_anchor_without_receive_ledger_projection_does_not_release`）。
- [x] old frame host present keeps display-visible trace but does not close media recovery。

## P2：Trace Projection 与验收脚本修正

### 修改项

- [x] `trace_receive_feedback_report.py` 拆分字段：
  - `arbiterMismatchTotal`
  - `sparseMustIdrMismatchTotal`
  - `referenceStatsFallbackTotal`
- [x] `keyframeChain` 按 receive recovery generation / response RTP 绑定；存在 `episodeId` 时额外输出 effectiveness grouping。
- [x] `receiveFeedbackDecision` 输出：
  - `episodeId`（optional）
  - `responseState`
  - `terminalCandidate`
  - `lastKeyframeSentAgeMs`
  - `h264Verdict`
  - `decoderVerdict`
  - `referenceStateCause`
- [x] `referenceChainStateChanged` 输出：
  - `source=ledger|stats-fallback`
  - `lastKeyframeSentAgeMs`
  - `responseState`
  - `decoderVerdict`
- [x] `receivePictureRecoveryTerminal` 输出：
  - `episodeId`（optional）
  - `reason`
  - `responseState`
  - `sentCountUnresolved`
  - `elapsedRttCount`
  - `nextOwnerHint`
- [x] 追加 `epochIsolationViolations` / `sameLedgerGenerationClosureFailures` gate。

### 完成定义

- [x] 旧 failing trace replay 产生明确 FAIL reason（`silentStuck`）；continuation 样本 terminal reason 可读
- [ ] 健康 trace 产生 PASS：`sent -> response -> decoded -> cleanAnchor -> displayStable` 全链路非 0（待当前构建新采）
- [x] continuation-only trace 允许 chain rate 为 0，但必须输出 terminal diagnostic（`silentStuck` gate）；`1780024427780` 无 silentStuck
- [x] report 字段命名与 stats 字段一一对应。

### 回归测试

- [x] trace projection emits receive terminal once per unresolved recovery generation。
- [x] trace report does not mix sparse mismatch into arbiter mismatch。
- [x] keyframe chain rates are ledger-generation-bound；episode grouping 只做辅助统计。
- [x] low response rate gate distinguishes terminal-handled and silent-stuck。

## P3：Episode 旧投影清理

### 修改项

- [x] `latest_keyframe_request_episode`、`recent_keyframe_request_episodes` 保留为 diagnostics snapshot，核心恢复状态读取 receive ledger。
- [x] `keyframe_effectiveness.episodes` 允许为 0；report 以 `ledgerGeneration` / RTP / decode / clean-anchor 链路计算恢复有效性。
- [x] 旧 `transportAwaitRecoveryAnchor` episode 字段迁移到 compatibility projection：
  - `waiting-response` 读取 `last_keyframe_request_sent_at_ms`
  - `continuation-only` 读取 `response_state=non-idr-only`
  - `stalled` 读取 terminal candidate / terminal reason
- [x] `RecoveryCoordinator::check_idr_completed` 使用 receive ledger 的 usable keyframe / clean anchor / decoder synced facts。
- [x] trace 中输出 `episodeProjectionState` / `displaySupplyStarvedBlocker`（`trace_projection`；旧 JSONL 无字段，新采可见）

### 完成定义

- [x] `episode=0` 的日志仍可完成 terminal 或 clean-anchor 闭环判定。
- [x] 旧 episode projection mismatch 只进入 diagnostics，不改变 Insert/Decode/arbiter/session 动作。
- [x] coordinator 不再依赖当前 episode 才能清除 keyframe in-flight / await-response 观测。

### 回归测试

- [x] usable IDR with no episode clears keyframe_required。
- [x] terminal with no episode emits receivePictureRecoveryTerminal。
- [x] legacy-only episode cannot close receive recovery without ledger fact。
- [x] episode projection mismatch increments diagnostics only。

## P3：Session / Coordinator 边界复核

### 修改项

- [x] `RecoveryCoordinator` 对 receive terminal 只产出 reconnect/connectivity candidate。
- [x] `RecoveryAction::RequestPli` / `RequestFir` 在 session path 保持 delegated / suppressed。
- [x] `latest_transport_await_response_observed_at_ms` 迁移为读取 receive ledger response facts；旧 episode 字段只做兼容 projection。
- [x] decoder reset 只在 backend/reconfigure/local maintenance 或 decoder-owned failure 出现。

### 完成定义

- [x] `recovery_session_keyframe_in_flight=false` 在 picture recovery 场景保持成立。
- [x] `sessionPictureRecoveryViolations=0`。
- [x] `decoderResetViolations=0`。
- [x] receive terminal 可以触发 reconnect candidate；session keyframe request path 保持 suppressed / delegated。

## 验收矩阵

### 必跑单测

- [x] `cargo test -p xbxengine --lib receive_feedback`
- [x] `cargo test -p xbxengine --lib ingress_loop`
- [x] `cargo test -p xbxengine --lib runtime_stats_sink`
- [x] `cargo test -p xbxengine --lib transport::rtc::recovery`
- [x] `cargo test -p xbxengine --lib transport::rtc::policy::video_scheduling_owner`
- [x] `cargo test -p xbxrc --lib trace_projection -- --test-threads=1`

### 必跑格式与构建

- [x] `cargo fmt`
- [x] `cargo test -p xbxengine --lib`（1032 passed, 2026-05-29）
- [x] `cargo test -p xbxrc --lib trace_projection -- --test-threads=1`

### 必跑 trace

- [x] `python3 .../trace_receive_feedback_report.py runtime-logs/runtime-trace-1779961935840-1.jsonl` — FAIL `silentStuck`（基线）
- [x] continuation `runtime-trace-1780024427780-1.jsonl` — `terminalRemoteContinuationOnly=37`，无 `silentStuck`
- [ ] 新采 healthy trace（当前构建）— 预期 `receiveFeedbackGate=PASS`，rates/chain 非零
- 归档：[`docs/reports/trace-validation/`](../reports/trace-validation/)

## 最终完成定义

- [x] receive 侧 keyframe sent 全部进入 receive ledger request facts；episode 统计可为空但不可影响恢复。
- [x] `ReferenceChainState` 能从 repairing 推进到 need-keyframe / continuous / terminal。
- [x] 远端长期不给 usable IDR 时输出 terminal diagnostic（代码 + 单测；live trace 待回放确认）。
- [x] 远端给 usable IDR 时闭合到 clean anchor 与 display stable（单测 + epoch 隔离）。
- [x] `MediaSupplyPhase` / `displayed_idr_*` 完成 projection-only 收口（owner release 读 receive complete）。
- [x] trace report 可以区分：
  - arbiter mismatch
  - sparse/must-idr projection mismatch
  - no response
  - continuation-only response
  - decoder rejected
  - no clean anchor
  - display stable success
- [x] 新 epoch 开始后旧 usable-idr / display-stable / decode-sync 不能污染当前轮闭合。
- [x] continuation 新日志有 terminal，无“sent 多、零 terminal”沉默组合；replay 基线仍 FAIL 以作对照
- [ ] 同构建 healthy 新采后全 gate PASS（待采）
