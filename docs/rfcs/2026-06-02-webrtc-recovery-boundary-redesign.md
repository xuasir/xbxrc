# RFC：WebRTC 式恢复边界设计级修正

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 实现与 fresh runtime trace 验收已完成
- Current State: accepted
- Owner: agent
- Last Updated: 2026-06-03

## Background

最近三份 trace 暴露出同一条结构性失败链：

- `runtime-trace-1780301689889-1.jsonl`：host 接受 18 个 keyframe，receive feedback 仍长期投影 `responseState=non-idr-only`，`remote-continuation-only` terminal 32 次，`cleanAnchorCommitted=0`，`DisplayStable=0`。
- `runtime-trace-1780304000721-1.jsonl`：fresh anchor 被 `mailboxOverwrite` 覆盖，后续恢复仍停在 `displaySupplyStarved / must-idr`。
- `runtime-trace-1780306008179-1.jsonl`：仅有 startup priming，缺少媒体恢复链，作为本轮根因分析的低权重样本。
- `runtime-trace-1780371569875-1.jsonl`：上一轮 media gate 已经打通，`pictureRecoveryTransition seq=214` 进入 `Decoded -> CleanAnchorCommitted`，`cause=decoded-usable-idr`，但 host 后续一直 retained redraw 同一帧，`DisplayStable=0`。同时 `referenceChainStateChanged seq=224` 出现 `decoderReferenceSynced=true` 与 `keyframeRequired=true/cause=first-delta` 并存，后续 non-IDR inspection 持续 `committed_sps_present=false / committed_pps_present=false`，说明 receive H264 tracker 的参数集提交仍被放在下游 display/ingress/decode 路径之后。
- `runtime-trace-1780376769157-1.jsonl`：media gate 继续闭合，`seq=538 Decoded -> CleanAnchorCommitted`，但后续 `receive_keyframe_required=true` 回到 `keyframe-sent-non-idr-only`，`receive_keyframe_response_state=non-idr-only`，并长期只有 `hostMailboxEnqueueCountTotal=1` / `lastDisplayedFrameSeq=4`。进一步回归代码发现四处旧债仍能让 continuation 回到恢复态：首轮 epoch 0 clean-anchor ack 会被 `last_consumed_clean_anchor_epoch=0` 跳过；clean anchor 提交后旧 `decoder_result=WaitingKeyframe/NoOutput/InvalidData` 仍可让 packet action stage 输出 `RequestIdr`；旧 hard gap / anchor rejected 仍可在 trace ledger 中压过 clean anchor 投影；`maybe_request_first_frame_acquisition_keyframe()` 会把启动 probe 写成 `note_first_delta()`，导致“请求关键帧”本身变成 `keyframe_required/cause=first-delta` 控制事实。
- 最新代码回归继续定位到三处设计级漏点：`current_clean_anchor_observed_at_ms()` 只承认 `displayed-idr`，导致 trace 中 `decoded-usable-idr` clean anchor 无法被 transport-await / idle guard 识别为当前锚点；`ReferenceChainState` 会因 host `submit_age_ms` 供给饥饿直接变成 `NeedKeyframe`，让 display starvation 反向 hold pre-decode continuation；decode actor 与 session owner 从 runtime stats 重建上下文时仍会把 stale `latest_video_receiver_observation=waiting-keyframe` 当成控制事实，绕过 receive ledger 的 clean anchor / decoder sync。
- 本轮代码回归继续发现 `ReferenceChainState` 与 sparse-IDR/request retry 仍有 stats-derived fallback：`InsertContext::from_runtime()` 能在生产 fallback 中重派生 reference chain，`sparse_idr_rhythm_from_recovery_ledger()` 内部仍用 stats 派生 `PacketRecoveryActionStage`，session fresh-output suppression 与 keyframe retry interval 仍读取 legacy sparse pressure。最新实现已把这些入口收紧为 receive-ledger-first：stats 只补 diagnostic facts，生产 Insert/feedback/retry/session 节奏只读 receive ledger 投影。
- 2026-06-03 追加审计发现 clean-anchor source 仍有一处分裂：`current_clean_anchor_observed_at_ms_from_stats()` 已承认 `decoded-usable-idr / clean-anchor-committed / displayed-idr`，但 `has_current_clean_anchor_from_stats()` 仍只承认 fresh/displayed display facts；decode FSM 的 waiting-keyframe continuation bypass 也要求 `displayed_idr_serving_from_stats()`。最新实现已统一为 current media anchor，decoded clean anchor 可直接释放 transport-await hard evidence、startup steady hold 与 decode continuation bypass。
- `runtime-trace-1780456567200-1.jsonl` 证明新二进制已经能进入 `sessionPhase=steady` / `videoHealth=healthy`，NACK 恢复有效率为 1.0，`sparseMustIdrMismatchTotal=0`，但仍失败在 stale packet-repair debt：`receiveFeedbackGate=FAIL`，`keyframeChain sent=23 / decoded=3 / cleanAnchorCommitted=2 / DisplayStable=0`，midsegment `submit_age_ms P95=4601ms`、`recovering streak=55.1s`、`host-retained-old-frame=346`。关键冲突是 NACK 轮次后 `keyframeRequired=true/cause=nack-exhausted` 能与 `ReferenceChainState=continuous` 共存，导致 InsertGate 继续 hold continuation。
- `runtime-trace-1780467603921-1.jsonl` 证明 stale `nack-exhausted + continuous` 已显著改善，能进入 steady/healthy，host mailbox 推进到 1058，`host-retained-old-frame` 从 346 降到 2，但仍在 packet action/reference 边界自锁：早期出现 `referenceState=continuous/keyframeRequired=false` 与 `packetRecoveryActionStage=request_idr` 的 `mustIdrHold`；后段 disposable/unknown transport gap 被 NACK exhausted 升级为 `receive-ledger-hard-gap-nack-exhausted`，`seq=1068/1069` 后 InsertGate 转 `request_idr + mustIdrHold`，host 继续刷新旧帧且 `submitAgeMs` 从 3.7s 增长到 5.2s。根因是 packet repair debt 与 reference-chain break 仍未分层，runtime timeline diagnostic gap 被当成当前 reference 控制事实。
- `runtime-trace-1780483285846-1.jsonl` 证明本轮 fresh 二进制已通过组合验收：`acceptanceGate=PASS`、`traceFreshness.freshnessGate=PASS`、`receiveFeedbackGate=PASS`、`receiveFeedbackGateFailures=[]`，keyframe chain 为 `sent=5 / responseObserved=2 / decoded=2 / cleanAnchorCommitted=2 / DisplayStable=1`，midsegment `globalLatencyGate=PASS / mediaSupplyGate=PASS / steadySupplyGate=SKIPPED`。应用以 `pnpm tauri dev --config '{"build":{"devUrl":"http://localhost:1420/#/xhome/stream/F4000F4B1B7F88D1"}}'` 运行约 420 秒，终端未见 panic，进程由 timeout 结束。

当前问题已经超过局部阈值和补丁范围。旧架构在 receive / decode / pacer / host / session policy 之间存在跨层调度闭环：

```text
Receive keyframe required
  -> InsertGate must-idr hold
  -> Decoder 等 IDR / 产出 keyframe
  -> Host/display 必须提交 displayed-idr / cleanAnchor
  -> Supply-break / mailbox latest overwrite / latest inspection 覆盖使 cleanAnchor 缺失
  -> Receive ledger 继续 need-keyframe / non-idr-only
  -> InsertGate 继续 hold continuation
```

标准 WebRTC 的参考边界是 receive 侧闭合媒体参考链，display/render 是后续消费者。`RtpVideoStreamReceiver2` 将 H264 SPS/PPS 处理、packet buffer、reference finder、NACK / keyframe feedback 放在接收侧主链；H264 SPS/PPS tracker 在缺少必要参数集时请求 keyframe；NACK requester 在 NACK list 过大时请求 keyframe；完整 frame 经 reference finder 后交给 decoder callback。参考：

- <https://webrtc.googlesource.com/src/+/refs/heads/main/video/rtp_video_stream_receiver2.cc>
- <https://webrtc.googlesource.com/src/+/refs/heads/main/modules/video_coding/h264_sps_pps_tracker.cc>
- <https://webrtc.googlesource.com/src/+/refs/heads/main/modules/video_coding/nack_requester.cc>

本仓库仍保留低延迟 latest-only / pacer / native host present 主线，但恢复事实必须回到 receive/decode 层，display 只负责显示稳定性。

## Goal

建立一条 WebRTC 式、边界清晰、无跨层自锁的恢复主线：

1. receive/decode 层闭合 media recovery：`keyframe_required -> request -> usable_idr -> decoded -> decoder_reference_synced -> media_recovered`。
2. pacer/render/host 层闭合 presentation recovery：`media_recovered -> presented -> display_stable`。
3. session policy 只处理 transport / backend / reconnect，不参与图片级 PLI/FIR 和 IDR 可用性判断。
4. diagnostics / trace / UI 只做 projection，不再反向驱动 recovery 决策。
5. 删除旧跨层状态互锁：`displayed-idr`、`cleanAnchorCommitted`、`DisplayStable`、`PresentationSupplyPhase`、`hostMailboxRetainedDisplayed` 退出 pre-decode 控制面。

## Scope

In scope:

- `crates/xbxengine/core/src/transport/rtc/receive/*`
- `crates/xbxengine/core/src/transport/rtc/recovery/contract/*`
- `crates/xbxengine/core/src/transport/rtc/session/*`
- `crates/xbxengine/core/src/media/video/decode/*`
- `crates/xbxengine/core/src/media/video/render/*`
- `crates/xbxengine/core/src/diagnostics/sink/runtime_stats_sink/*`
- `src-tauri/src/mods/xbxengine/trace_projection.rs`
- runtime trace analysis scripts and validation gates

Out of scope:

- 替换 `rtc` / Tauri / Rust 主线。
- 引入浏览器 WebRTC 作为桌面 Rust 侧替代。
- 继续增加 reconnect / decoder reset 兜底来掩盖 receive 闭环失败。
- 保留旧状态机作为运行时双轨。

## Design Principles

### 1. Media recovery 由 receive/decode 闭合

媒体恢复完成的事实源固定为：

```text
H264 usable IDR
  + decode output accepted
  + decoder reference synced
  + current recovery generation matched
= media_recovered
```

`media_recovered` 可以由解码器事实直接提交。host visible fact 继续用于 `display_stable`，不作为 `usable_idr` 或 `decoder_reference_synced` 的前置条件。

### 2. Display recovery 是独立后置 gate

display 层只回答：

- 用户是否看到新帧。
- latest-only mailbox 是否持续可服务。
- present cadence 是否稳定。
- retained displayed 是否只是重绘旧帧。

display 层可以影响 UI 诊断和本地 presentation policy，不能发起 PLI/FIR，不能阻塞 receive ledger 承认 usable IDR。

### 3. Session policy 退出 picture recovery

Session policy 只保留：

- ICE / SRTP / track / RTCP target / liveness。
- backend failure / reconfigure / decoder reset local maintenance。
- receive terminal 后的 reconnect candidate。

Session policy 对 picture recovery 的输出固定为 `DelegatedToReceive` 或 reconnect candidate。

### 4. Projection 无控制权

以下字段降级为 projection / diagnostics：

- `PresentationSupplyPhase` / 外部兼容字段 `media_supply_phase`
- `displayedIdrHostHint`
- `displayedIdrServing`
- `hostMailboxRetainedDisplayed`
- `PlaybackRecovered`
- `DisplayStable`
- `latest_observation_summary`

核心控制面只读 receive ledger、decoder fact、packet/NACK/H264 fact、transport capability。

### 5. Decoded anchor clears old packet/reference debt

对齐 WebRTC `FrameDecoded()` 后清 packet/reference buffers 的行为：当前 recovery epoch 的 decoded clean anchor 一旦被 receive ledger 消费，旧 packet gap、NACK pending/exhausted、anchor candidate reject、unrecoverable frame projection、stale decoder waiting/no-output fact 都退出 picture recovery 控制面。后续新 RTP gap 仍按 receive-local NACK/repair 重新建账。

### 6. Clean anchor source is media-first

当前 recovery epoch 的 clean anchor 来源必须同时承认 receive/decode 与 display 两类事实：

- `decoded-usable-idr`
- `clean-anchor-committed`
- `displayed-idr`

`displayed-idr` 只能作为 display/post-media projection 的一种来源，不能成为 current clean anchor 的唯一来源。transport-await、idle guard、owner release 与 decode context 重建都读取同一 clean-anchor 定义。

### 7. Decode continuation bypass reads media anchor

解码侧 waiting-keyframe continuation bypass 读取 current clean anchor + SPS/PPS commit + supply/timed-fallback serviceability。displayed-IDR 继续作为 presentation projection，不能成为解码续播窄路径的唯一入口。

## Target Architecture

```text
RTP / RTX / RTCP
    |
    v
ReceiveLedger
  - packet gaps
  - H264 SPS/PPS/IDR
  - NACK disposition
  - keyframe request generation
  - usable IDR response
    |
    v
InsertGate
  - Emit
  - HoldRepair
  - DropCorrupt
    |
    v
DecodeIngress
  - decoder submit
  - decode output
  - decoder_reference_synced
  - media_recovered
    |
    v
PostDecodeLatency
  - pacer
  - latest mailbox
  - host present
  - display_stable

Session
  - transport/backend/reconnect only
```

## Contract Changes

### 1. Replace `cleanAnchorCommitted` as media gate

New primary media gate:

```text
ReceiveMediaRecovered {
  generation,
  response_rtp,
  decoded_rtp,
  decoder_reference_synced_at_ms,
  source: usable-idr | bootstrap-idr | decoder-sync-idr
}
```

Compatibility mapping:

- `cleanAnchorCommitted` becomes projection of `ReceiveMediaRecovered`.
- `DisplayStable` remains display gate after host present.
- `FreshAnchorRecovered` becomes display-side event when host presents the recovered generation.

### 2. Make `usable-idr` a receive/decode fact

`usable-idr` requires:

- H264 inspection `is_idr=true`
- SPS/PPS available or injected
- AU accepted by InsertGate
- decoder output confirms keyframe or reference sync
- generation matches active keyframe request or accepted unsolicited bootstrap IDR

It does not require:

- current host displayed RTP equals IDR RTP
- latest inspection still points to that IDR
- supply-break inactive
- mailbox preserved the IDR frame as visible frame

### 3. Delete supply-break veto over media recovery

`recovery_supply_break_active_from_stats()` must not veto fresh anchor / media recovery commit.

Supply-break can describe presentation starvation. It cannot reject receive/decode proof that the reference chain recovered.

### 4. Stop using global latest inspection for delayed host facts

Host/presenter facts arrive after receive/decode facts and can be overtaken by later inspections. Any recovery closure that needs H264 identity must carry per-frame metadata:

- `rtp_timestamp`
- `is_keyframe`
- `presentation_role`
- `recovery_generation`
- `decoder_reference_synced`

Global `latest_h264_inspection_observation` remains trace context only.

### 5. Keep latest-only mailbox, remove recovery ownership

latest-only mailbox remains correct for low latency. Its overwrite output only says which frame reached host. It cannot erase `media_recovered`.

Fresh anchor overwrite becomes acceptable:

```text
fresh_anchor decoded + decoder_reference_synced
  -> media_recovered
  -> mailbox may display later continuation
  -> display_stable can close on continuation from same recovered generation
```

## Implementation Plan

### Phase 0: Contract audit and failing tests

- Add regression tests from `1780301689889` and `1780304000721`:
  - host accepts keyframe while receive remains `non-idr-only`
  - fresh anchor overwritten by continuation
  - supply-break active during decoded usable IDR
  - global latest H264 inspection overwritten before host present
- Add explicit tests that these cases produce `ReceiveMediaRecovered`.

### Phase 1: Receive ledger owns media recovery

追加收口：

- `ReceiveRecoveryLedger::apply_decoder_facts_from_stats()` 必须先消费 `decoder_reference_synced`，再处理 waiting-keyframe / no-output / invalid-data，避免陈旧 decoder state 覆盖当前 decode sync。
- `ReceiveRecoveryLedger::note_first_delta()` 在 clean anchor committed 或 decoder reference synced 后保持 continuous，避免 first delta 重新打开 `keyframe_required`。
- `ReceiveRecoveryLedger::note_non_idr_continuation()` 在 clean anchor committed 或 decoder reference synced 后保持 continuous，避免恢复后的正常 continuation 被误记为 `keyframe-sent-non-idr-only`。
- H264 SPS/PPS tracker 属于 receive packet/H264 主链；完整 AU 被 InsertGate 接受后立即 `inspection.commit()`，后续 non-IDR continuation 能直接读取 committed SPS/PPS。
- First-frame acquisition request 只作为 receive-local feedback probe，不再调用 `note_first_delta()`，避免启动采集请求自我打开 `keyframe_required` 并关闭 startup compatibility 窗口；receive feedback arbiter 单独支持 `force_keyframe`，保证 probe 仍会真实进入 KeyframeRequester 发送 PLI/FIR。
- Keyframe executor sent 后必须立即重投影 receiver/timeline，使上层 owner facts 读取到 `waiting-keyframe` / `keyframeRequestPending=true` 的最终发送状态，而不是请求前 timeline。
- Decoded clean-anchor ack 必须同时清 receive-local blocking wait、retry timer 与 keyframe pending projection；否则 mediaRecovered 后 receiver state 仍是 `waiting-keyframe`，下一帧 continuation 会被 InsertGate 再次 hold。
- 对齐 WebRTC `FrameDecoded()` / `NackRequester`：decoder reference sync、clean anchor、NACK recovered、hard gap resolved、complete candidate 都必须清 stale NACK exhausted debt；`nack_exhausted` 只在仍有 unresolved hard gap 时参与 reference control，gap resolved 后只保留历史诊断。
- NACK exhausted 是 packet repair debt，只有 trace ledger 证明 unresolved hard/reference gap 时才能设置 `keyframe_required` 或触发 PLI/FIR 升级；disposable/unknown transport gap 继续走 NACK/repair 诊断，不进入 picture recovery 控制。

- Add `ReceiveMediaRecovered` fact to receive ledger / runtime stats.
- Update keyframe lifecycle to close on decoded usable IDR + decoder sync.
- Make `receive_keyframe_response_state=usable-idr` survive display lag and mailbox overwrite.
- Keep `keyframeChain.cleanAnchorCommitted` as projection of media recovery for trace compatibility.

### Phase 2: InsertGate reads only receive/decode facts

- Remove `displayed-idr` / supply-break / host mailbox facts from InsertGate control inputs.
- Keep `PresentationSupplyPhase` as diagnostics/UI projection and keep the external `media_supply_phase` field for DTO compatibility.
- Ensure `ReferenceChainState::Continuous` or `Repairing` follows decoder sync and NACK facts, not display facts.
- Production InsertContext must be built from ledger-projected `ReferenceChainObservation`, ledger-derived `PacketRecoveryActionStage`, and ledger `keyframe_required`; stats-derived `from_runtime` stays as test/legacy projection only.
- `ReceiverTraceLedger::reference_chain_observation()` owns the production projection: receive ledger decides state/cause, stats supplies diagnostic fields such as decoder sync, bootstrap readiness, active gap, NACK exhausted, and submit age.
- `PacketRecoveryActionStage` 进入 Insert 前必须按 reference state 归一化：decoder/reference 已同步且 reference 为 `Continuous/Repairing` 时，旧 `WaitKeyframe/RequestIdr` 降为 `NackMissed/NackPending/Steady`；gap stale 只在 unresolved reference gap 上触发 keyframe-only。

### Phase 3: Decode/pacer bridge carries recovery identity

- Carry `recovery_generation`, `response_rtp`, `is_keyframe`, `decoder_reference_synced` through decoded frame metadata.
- Pacer keeps latest-only policy, but never converts local overwrite into media recovery failure.
- Render mailbox drop telemetry preserves kept/dropped identity for trace only.

### Phase 4: Host/display gate becomes post-media

- `record_displayed_idr_fact` becomes display projection.
- `DisplayStable` closes after media recovery and fresh presentation from same generation.
- Retained displayed frame continues repainting but cannot close media recovery.

### Phase 5: Session policy purge

- Remove remaining picture recovery authority from session.
- Session fresh-output suppression and keyframe retry cadence read receive-ledger sparse pressure, not legacy presentation/supply-derived sparse projection.
- Session reacts to receive terminal:
  - `remote-no-response`
  - `remote-continuation-only`
  - `remote-idr-unusable`
- Reconnect candidate remains transport/session responsibility after receive terminal.

### Phase 6: Trace and gates

- Update `trace_receive_feedback_report.py`:
  - `mediaRecovered`
  - `displayStable`
  - `mediaRecoveredWithoutDisplayStable`
  - `displayStableWithoutMediaRecovered`
  - `mailboxOverwriteFreshAnchorAccepted`
- Healthy gate requires:
  - `sent > 0` only when recovery triggered
  - `responseObserved > 0`
  - `decoded > 0`
  - `mediaRecovered > 0`
  - sustained recovering streak < 5s
- Display gate is separate and may fail independently.

## Validation

- [x] `cargo test -p xbxengine media::video::decode::actor --lib`
- [x] `cargo test -p xbxengine diagnostics::sink::runtime_stats_sink --lib`
- [x] `cargo test -p xbxengine transport::rtc::stack::runtime_port --lib`
- [x] `cargo test -p xbxengine decoded_clean_anchor_ack_consumes_epoch_without_displayed_idr --lib`（覆盖 decoded clean-anchor ack 清 receive-local waiting 与 pending projection）
- [x] `cargo test -p xbxengine clean_anchor_masks_stale_decoder_waiting_for_insert_control --lib`
- [x] `cargo test -p xbxengine clean_anchor_masks_stale_decoder_waiting_for_recovery_contract --lib`
- [x] `cargo test -p xbxengine clean_anchor_ack_consumes --lib`
- [x] `cargo test -p xbxengine recovery_ledger --lib`
- [x] `cargo test -p xbxengine forced_keyframe_request_sends_without_latching_keyframe_required --lib`
- [x] `cargo test -p xbxengine first_frame_acquisition_probe_does_not_latch_keyframe_required --lib`（覆盖 sent 后 receiver/timeline pending projection）
- [x] `cargo test -p xbxengine first_frame_probe_keeps_startup_priority_window_active --lib`
- [x] `cargo test -p xbxengine blocking_receive_recovery_closes_startup_priority_window --lib`
- [x] `cargo test -p xbxengine transport::rtc::receive --lib`
- [x] `cargo test -p xbxengine transport::rtc::recovery::contract --lib`
- [x] `cargo test -p xbxengine transport::rtc::session::policy --lib`
- [x] `cargo test -p xbxengine transport::rtc::session --lib`
- [x] `cargo test -p xbxengine --lib`
- [x] `cargo test -p xbxengine transport::rtc::receive --lib`：188 passed, 16 ignored
- [x] `cargo test -p xbxengine transport::rtc::recovery::contract --lib`：47 passed
- [x] `cargo test -p xbxengine --lib`：1055 passed, 24 ignored
- [x] `cargo test -p xbxengine media::video::decode --lib`：65 passed
- [x] `cargo test -p xbxengine transport::rtc::receive --lib`：188 passed, 16 ignored
- [x] `cargo test -p xbxengine transport::rtc::recovery::contract --lib`：49 passed
- [x] `cargo test -p xbxengine transport::rtc::recovery::startup --lib`：14 passed
- [x] `cargo test -p xbxengine --lib`：1059 passed, 24 ignored
- [x] `python3 .agents/skills/analyze-runtime-logs/scripts/trace_receive_feedback_report.py runtime-logs/runtime-trace-1780456567200-1.jsonl`：旧构建最新 trace 仍 `receiveFeedbackGate=FAIL`，失败项为 `arbiterMismatchTotal` 与 `insertSurfacePhaseActionStage`，`DisplayStable=0`。
- [x] `python3 .agents/skills/analyze-runtime-logs/scripts/trace_midsegment_report.py runtime-logs/runtime-trace-1780456567200-1.jsonl`：`GLOBAL_LATENCY_GATE=FAIL`，`submit_age_ms P95=4601ms`、recovering streak `55.1s`。
- [x] `cargo test -p xbxengine transport::rtc::receive --lib`：194 passed, 16 ignored
- [x] `cargo test -p xbxengine transport::rtc::recovery::contract --lib`：49 passed
- [x] `cargo test -p xbxengine --lib`：1065 passed, 24 ignored
- [x] `python3 .agents/skills/analyze-runtime-logs/scripts/summarize_runtime_trace.py runtime-logs/runtime-trace-1780467603921-1.jsonl`：host mailbox 推进到 1058，CleanAnchorCommitted=3，后段仍出现 `request_idr + mustIdrHold` 与 `submitAgeMs` 秒级增长。
- [x] `python3 .agents/skills/analyze-runtime-logs/scripts/trace_receive_feedback_report.py runtime-logs/runtime-trace-1780467603921-1.jsonl`：`receiveFeedbackGate=FAIL`，`arbiterMismatchTotal=2733`，`sparseMustIdrMismatchTotal=602`，`insertSurfacePhaseActionStage=1`。
- [x] `python3 .agents/skills/analyze-runtime-logs/scripts/trace_midsegment_report.py runtime-logs/runtime-trace-1780467603921-1.jsonl`：`GLOBAL_LATENCY_GATE=FAIL`，后段 `receiverWaitingKeyframe` 与 retained old frame 复现。
- [x] `cargo test -p xbxengine transport::rtc::receive --lib`：197 passed, 16 ignored
- [x] `cargo test -p xbxengine transport::rtc::recovery::contract --lib`：49 passed
- [x] `cargo test -p xbxengine --lib`：1068 passed, 24 ignored
- [x] `cargo test -p xbxengine transport::rtc::receive --lib`：197 passed, 16 ignored
- [x] `cargo test -p xbxengine transport::rtc::recovery::contract --lib`：50 passed
- [x] `cargo test -p xbxengine transport::rtc::session --lib`：77 passed
- [x] `cargo test -p xbxengine --lib`：1069 passed, 24 ignored
- [x] `python3 -B .agents/skills/analyze-runtime-logs/scripts/trace_receive_feedback_report.py --fail-on-gate --require-media-recovered --require-display-stable runtime-logs/runtime-trace-1780467603921-1.jsonl`：旧 trace 返回 `gate_code=2`，证明严格验收可机器失败 stale / incomplete trace。
- [x] `python3 -B -m unittest discover .agents/skills/analyze-runtime-logs/tests`：6 tests OK，覆盖 receive trace strict gate PASS/FAIL 退出码。
- [x] `python3 -B .agents/skills/analyze-runtime-logs/scripts/trace_webrtc_acceptance_gate.py runtime-logs/runtime-trace-1780467603921-1.jsonl`：旧 trace 返回 `gate_code=2`，证明组合 gate 可同时失败 receive 与 low-latency 未完成样本。
- [x] `python3 -B -m unittest discover .agents/skills/analyze-runtime-logs/tests`：7 tests OK，覆盖组合 gate PASS。
- [x] `python3 -B -m unittest discover .agents/skills/analyze-runtime-logs/tests`：9 tests OK，覆盖组合 gate `--latest` 与 `--max-age-seconds`。
- [x] `python3 -B .agents/skills/analyze-runtime-logs/scripts/trace_webrtc_acceptance_gate.py --latest --max-age-seconds 3600`：旧最新 trace 返回 `gate_code=2`，`traceFreshness.freshnessGate=FAIL`，防止 stale trace 被误读成 fresh acceptance。
- [x] `cargo fmt`
- [x] `git diff --check`
- [x] replay `runtime-trace-1780301689889-1.jsonl` with trace report: old binary shows `decoded=1` and `cleanAnchorCommitted=0`, matching the pre-fix failure.
- [x] replay `runtime-trace-1780304000721-1.jsonl` with trace report: old binary shows `decoded=1` and `cleanAnchorCommitted=0`, matching the pre-fix mailbox/display closure failure.
- [x] replay `runtime-trace-1780306008179-1.jsonl` with trace report: trace has no receive-feedback events, treated as low-weight startup/no-media sample.
- [x] `python3 -B .agents/skills/analyze-runtime-logs/scripts/trace_receive_feedback_report.py runtime-logs/runtime-trace-1780483285846-1.jsonl --fail-on-gate --require-media-recovered --require-display-stable`：`receiveFeedbackGate=PASS`，`sent=5 / responseObserved=2 / decoded=2 / cleanAnchorCommitted=2 / DisplayStable=1`。
- [x] `python3 -B .agents/skills/analyze-runtime-logs/scripts/trace_webrtc_acceptance_gate.py --latest --runtime-log-dir runtime-logs --max-age-seconds 900`：fresh trace `runtime-trace-1780483285846-1.jsonl` 返回 `acceptanceGate=PASS`，`traceFreshness.freshnessGate=PASS`，midsegment `globalLatencyGate=PASS / mediaSupplyGate=PASS / steadySupplyGate=SKIPPED`。
- [x] 420 秒应用运行验收：`pnpm tauri dev --config '{"build":{"devUrl":"http://localhost:1420/#/xhome/stream/F4000F4B1B7F88D1"}}'`，终端未见 panic，进程由 timeout 结束。

## Risks

- Trace vocabulary churn: existing scripts and UI diagnostics need a migration layer for one release.
- Tests may expose older RFC assumptions that explicitly pinned `cleanAnchorCommitted` to host visible fact.
- Separating media and display gates can initially show more display failures, because media recovery will stop hiding presentation stalls.
- Receive ledger may need per-frame metadata plumbing across modules with large call-surface impact.

## Progress

- [x] Step 0: root cause isolated from latest three traces.
- [x] Step 1: RFC drafted.
- [x] Step 2: user confirms execution.
- [x] Step 3: implement receive/decode-side media recovery closure.
- [x] Step 4: split display/host into post-media gate and preserve compatibility projection.
- [x] Step 5: second pass purge for displayed-IDR receive control, supply-break session bypass, and stats-read decoded ack coupling.
- [x] Step 6: write report and update task tracker.
- [x] Step 7: validate with a fresh runtime trace from the rebuilt binary.

## Execution Notes

- Date: 2026-06-02 | Status: code-side completed, fresh runtime trace pending
- Date: 2026-06-03 | Update: 进一步收紧 presentation / receive 边界，`PresentationSupplyPhase` 已退出 `MustIdr`，`RecoverySurfacePhase::AwaitIdr` 由 receive-local keyframe fact / decoder waiting / packet action stage 直出，`displayed-idr` 仅保留为 presentation diagnostic。验证：`cargo test -p xbxengine transport::rtc::recovery::contract --lib`、`cargo test -p xbxengine --lib`、`git diff --check`。
- Date: 2026-06-03 | Update: 基于 `runtime-trace-1780456567200-1.jsonl` 继续收口 stale NACK debt。`ReceiveRecoveryLedger::note_decoder_reference_synced()` 现在按 WebRTC `FrameDecoded()` 语义直接清 keyframe debt 与 NACK exhausted；NACK recovered、gap resolved、complete candidate 在无 unresolved hard gap 时清 `nack-exhausted`；`ReceiverTraceLedger::reference_chain_observation()` 只在 hard gap 仍存在时把 NACK exhausted 投为 reference control。验证：`cargo test -p xbxengine transport::rtc::receive --lib`、`cargo test -p xbxengine transport::rtc::recovery::contract --lib`、`cargo test -p xbxengine --lib`、`git diff --check`。
- Date: 2026-06-03 | Update: 基于 `runtime-trace-1780467603921-1.jsonl` 继续收口 packet action/reference mismatch。NACK escalation 只有 unresolved hard/reference gap 才写 `note_nack_exhausted()` 和触发 keyframe feedback；receive decode context 的 `nack_exhausted` 同样绑定 hard gap；生产 packet action stage 不再读取 runtime timeline diagnostic gap；Insert control 会把 continuous/repairing + decoder synced 下的 `RequestIdr/WaitKeyframe` 归一为 repair-first。验证：`cargo test -p xbxengine transport::rtc::receive --lib`、`cargo test -p xbxengine transport::rtc::recovery::contract --lib`、`cargo test -p xbxengine --lib`。
- Date: 2026-06-03 | Update: 清理 packet action/reference 收口后的测试辅助噪音，补上 timed-fallback contract 回归测试标记，并重新跑 receive/contract/session/full lib。验证：`cargo test -p xbxengine transport::rtc::receive --lib`（197 passed, 16 ignored）、`cargo test -p xbxengine transport::rtc::recovery::contract --lib`（50 passed）、`cargo test -p xbxengine transport::rtc::session --lib`（77 passed）、`cargo test -p xbxengine --lib`（1069 passed, 24 ignored）、`cargo fmt`、`git diff --check`。
- Date: 2026-06-03 | Update: receive-feedback trace report 增加 `--fail-on-gate`、`--require-media-recovered`、`--require-display-stable`，fresh trace 可用单条命令同时验收 receive gate、media recovery 与 display stable。旧 trace `runtime-trace-1780467603921-1.jsonl` 在严格 gate 下返回 2，默认 JSON 输出保持兼容。
- Date: 2026-06-03 | Update: receive-feedback trace report 新增黑盒测试，最小 PASS trace 验证严格 gate 返回 0，最小 arbiter mismatch trace 验证 `--fail-on-gate` 返回 2。验证：`python3 -B -m unittest discover .agents/skills/analyze-runtime-logs/tests`（6 tests OK）。
- Date: 2026-06-03 | Update: 新增 `trace_webrtc_acceptance_gate.py` 组合验收入口，将 strict receive recovery gate 与 midsegment low-latency gate 合并成 `acceptanceGate` JSON。最小 synthetic trace 覆盖全绿返回 0，旧 trace `runtime-trace-1780467603921-1.jsonl` 返回 2。验证：`python3 -B -m unittest discover .agents/skills/analyze-runtime-logs/tests`（7 tests OK）。
- Date: 2026-06-03 | Update: 组合验收入口增加 `--latest` 与 `--max-age-seconds`，可直接选择最新 runtime trace 并用 `traceFreshness` 门槛拒绝 stale evidence。验证：`python3 -B -m unittest discover .agents/skills/analyze-runtime-logs/tests`（9 tests OK）、`trace_webrtc_acceptance_gate.py --latest --max-age-seconds 3600` 对旧最新 trace 返回 2。
- Date: 2026-06-03 | Update: fresh runtime trace `runtime-trace-1780483285846-1.jsonl` 通过最终组合验收：`acceptanceGate=PASS`、`traceFreshness.freshnessGate=PASS`、`receiveFeedbackGate=PASS`、`receiveFeedbackGateFailures=[]`，keyframe chain 为 `sent=5 / responseObserved=2 / decoded=2 / cleanAnchorCommitted=2 / DisplayStable=1`，midsegment `globalLatencyGate=PASS / mediaSupplyGate=PASS / steadySupplyGate=SKIPPED`。应用运行约 420 秒未见 panic，进程由 timeout 结束。验证：strict receive gate、combined WebRTC acceptance gate、`git diff --check`、`git diff --cached --check`。
- Update: Drafted design-level correction after `no-idr` recovery analysis.
- Decision: Media recovery completion moves to receive/decode ledger; display recovery becomes post-media gate.
- Implementation:
  - `record_picture_recovery_episode_decoded()` now commits decoded usable-IDR as current-epoch clean anchor, clears receive keyframe requirement, records decoder reference sync, and keeps `cleanAnchorCommitted` as compatibility projection.
  - decode actor guards recovery commits by owner episode and recovery epoch, then records pending displayed IDR only after pacer submit succeeds.
  - receive ledger clean-anchor ack now consumes current-epoch decoded clean anchor without requiring `recovery_displayed_idr_at_ms`; display stable remains conditional on host display fact.
  - runtime port tests now assert host present preserves `decoded-usable-idr` clean-anchor ownership and only contributes post-media playback/display projection.
  - trace report counts `CleanAnchorCommitted` in decoded/anchor chain for compatibility with the new projection.
  - receive-local state derivation now ignores displayed-IDR projection; decoder `waiting-keyframe` and no-output streak facts always enter `ReceiveRecoveryLedger`.
  - H264 PS/config strict mode and repairing missing-IDR pressure now read packet/reference/decoder evidence, with displayed-IDR removed from those IDR request decisions.
  - `MediaSupplyPhase` was renamed internally to `PresentationSupplyPhase`; `receive_media_recovery_pressure_from_stats()` now reads only receive-local IDR facts, while `supply-break` remains a presentation diagnostic.
  - host submit starvation now projects `PresentationSupplyPhase::SupplyBreak` instead of `MustIdr`; pure presentation supply pressure can no longer map into `RecoverySurfacePhase::AwaitIdr`.
  - suspect-anchor evidence now reads displayed-IDR as a plain display fact and receive IDR pressure as a receive-local fact; session no longer imports the displayed-IDR relaxed-control helper.
  - startup/session phase steady-hold now blocks only on receive hard bootstrap evidence or active decoder waiting control facts; displayed-IDR relaxed-control helper no longer enters session/startup/policy control paths.
  - session policy, startup compatibility, expensive reconnect gate, and recovery coordinator now ignore supply-break projection for picture-recovery control decisions.
  - host displayed fresh-anchor qualification now records matching H264/display facts without a supply-break veto.
  - first-frame acquisition PLI/FIR probe no longer writes `note_first_delta()` into receive ledger; real `keyframe_required` now comes from packet/H264/decoder/NACK facts, while startup compatibility remains active when the probe itself is non-blocking.
  - receive feedback arbiter now maps explicit `force_keyframe` to a `forced-keyframe` PLI/FIR decision, so first-frame acquisition keeps WebRTC-like feedback behavior without fabricating a reference-chain block.
  - keyframe request sent now republishes timeline as `keyframe-request-sent`, aligning `latest_video_timeline_observation.chain.state=waiting-keyframe` with receiver pending state after executor success.
  - decoded clean-anchor ack now clears receive-local blocking wait and republishes `clean-anchor-committed`, cutting the old trace path where `CleanAnchorCommitted` was followed by more `receiverWaitingKeyframe` holds.
  - decoded clean-anchor ack now also advances stale decoder recovery projection from `waiting-keyframe` to `nominal / clean-anchor-committed`, so runtime stats no longer reopens the closed media recovery gate on the next InsertGate pass.
  - `decoder_waiting_keyframe_control_active_from_stats()` is now the single control helper for decoder waiting facts. InsertGate, packet recovery action stage, reference chain, gap mode, supply phase, IDR pressure, and recovery exit use it, so current clean anchor / decoder reference sync masks old decoder waiting while preserving diagnostic visibility.
  - `ReceiverTraceLedger::reference_chain_observation()` now projects state/cause from receive ledger and only merges stats as diagnostic facts. Legacy `derive_reference_chain_state_from_stats()` remains for contract tests and fallback projections.
  - Production `InsertContext` construction now uses `from_ledger_inputs()` with ledger reference/action/keyframe inputs. `from_runtime()` / `from_runtime_with_reference()` are compiled only for tests.
  - `sparse_idr_rhythm_from_recovery_ledger()` now receives ledger-derived `PacketRecoveryActionStage` from the caller, so feedback arbiter, NACK acceleration, and keyframe requester interval all share the same receive-local action stage.
  - Session fresh-output bypass and recovery keyframe retry interval now read `receive_ledger_sparse_pressure_active_from_stats()`, which is backed by receive ledger sync fields and decoder reference sync.
  - sparse/MustIdr mismatch reporting now compares ledger-projected reference state with presentation supply phase as diagnostics, without re-deriving reference chain from runtime stats.
  - `has_current_clean_anchor_from_stats()` now uses the same source whitelist as `current_clean_anchor_observed_at_ms_from_stats()`, so decoded clean anchor and clean-anchor committed are current anchors before host displayed-IDR appears.
  - Runtime recovery serviceability now uses `media_anchor_output_pipeline_active()`, allowing decoded clean anchor + fresh decode/present output to absorb stale soft `NonIdrVcl` / `bootstrapMissingIdr` transport-await evidence.
  - Decode FSM recovery-exit policy now keys continuation bypass on current clean anchor instead of displayed-IDR serving. A regression proves waiting-keyframe non-IDR continuation enters decoder when current clean anchor, committed SPS/PPS, and supply-break serviceability are present.
  - Session/state recovery coordinator now treats picture recovery as receive-only authority end to end: `ActionCoordinator` returns `DelegatedToReceive` without entering `FrameRecovery` or marking `idr_requested`, stale `FrameRecovery` is cleared on the next delegated decision, and `sync_connectivity_escalation_state()` refuses accidental session `RequestPli/Fir` state mutation.
  - Session RFC cost/narrative projection now treats `RequestPli/Fir` as absorb/delegated diagnostics rather than active session recovery. `latest_recovery_decision_ledger` command-result pending is limited to real session commands: decoder reset and reconnect.
  - First-frame acquisition suppression now resolves to `DelegatedToReceive` rather than `CoalescedKeyframeInFlight`, and records a receive keyframe hint. This keeps startup probing receive-local while preventing session from manufacturing a keyframe-in-flight placeholder.
  - H264 picture blocker classification and deferred recovery command unlock reasons now use `has_current_clean_anchor_from_stats()`, so decoded/committed clean anchors suppress stale continuation/bootstrap diagnostics before host displayed-IDR exists.
- Residual: old traces can only prove the previous failure shape. A fresh trace from the new binary is still required for field validation of continuation emit/decode, host mailbox progress, and `DisplayStable > 0`.
