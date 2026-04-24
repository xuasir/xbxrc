# 渐进恢复分级与单飞切换 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 现有 Cloud 运行态已经证明：网络并非彻底不可用，真正放大卡顿的是恢复链在复杂波动下反复围绕同一批 gap 进入 `awaitingRecoveryKeyframe / broken / requestKeyframe`。
- 远端能力已经支持 `nack:pli` / `ccm:fir`，但当前恢复主路径仍主要走内部控制消息的 keyframe / decoder reset 请求，轻恢复没有成为第一层动作。
- 现在需要把恢复链改造成“分级恢复 + 单飞切换”，让系统在波动常态下先走轻恢复，必要时再升级，但始终只保留一个当前恢复意图，避免恢复积压。

## Goal

- 把恢复语义拆成清晰的分级路径：`LightRecover`、`HardRecover`、`ReconnectCandidate`。
- 让轻恢复优先使用 RTCP `PLI/FIR`，重恢复才走现有控制通道 keyframe / decoder reset。
- 用单一 recovery token / epoch 记录当前恢复意图，保证同一 gap / 同一 epoch 不会堆积多个并发恢复请求。
- 恢复成功后要能干脆清空旧债务，避免旧动作拖入下一轮。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/repeat_suppression.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/*.rs` 相关状态与诊断
  - `crates/xbxengine/core/src/transport/rtc/connection/service.rs`
  - `crates/xbxengine/core/src/transport/rtc/connection/data_channel.rs`
  - `crates/xbxengine/core/src/transport/rtc/connection/lifecycle.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.rs`
  - `crates/xbxengine/core/src/session/recovery.rs`
  - 相关单测、trace 投影、`docs/project-task.md`
- Out of scope:
  - 更换 Tauri + Vue + TypeScript + Rust 固定栈
  - 新增第二条媒体 transport / signaling 路径
  - 重新设计 render / BWE 主线
  - 把重恢复兜底再扩大为默认策略

## Plan

1. 引入分级恢复动作与单飞令牌。
   - 明确当前恢复阶段与单一活跃恢复意图。
   - 让后来的更强证据可以升级当前恢复，而不是新增并行请求。
2. 把轻恢复真正接到 RTCP 反馈。
   - 通过 video receiver 直接发 PLI/FIR。
   - 若 receiver/SSRC 不可用，则保留现有控制 keyframe 作为 fallback，不阻塞恢复主链。
3. 收紧 coordinator / repeat suppression，保证切换干脆。
   - 同一 epoch / 同一 gap 的重复恢复不积压。
   - 轻恢复失败后再升级硬恢复，不在两者之间反复摇摆。
4. 补测试并回放最新 Cloud trace。
   - 重点验证：恢复不会排队堆积、同 gap 重复请求明显减少、复杂波动下的 `dropLate/latestSlotOverwrite` 降低。

## Validation

- [x] 回归 `transport::rtc::recovery::escalation`
- [x] 回归 `transport::rtc::recovery::coordinator`
- [x] 回归 `transport::rtc::recovery::repeat_suppression`
- [x] 回归 `transport::rtc::connection::service`
- [x] 回归 `transport::rtc::stream::video_source::source`
- [x] 回归 `transport::rtc::session::policy`
- [x] 回放最新 Cloud trace，确认恢复动作不再积压，`awaitingRecoveryAnchor` / `requestKeyframe` 密度下降

## Risks

- 轻恢复过早发射会带来无效 RTCP 噪音，可能对远端编码器形成额外压力。
- 如果 receiver/SSRC 解析不稳定，PLI/FIR 的发射成功率会受影响，需要可靠 fallback。
- 单飞令牌如果设计过于保守，可能让恢复从“积压”变成“迟滞”。

## Progress

- [x] Step 1: 定义恢复分级与单飞令牌
- [x] Step 2: 接通 RTCP PLI/FIR 轻恢复
- [x] Step 3: 收紧 coordinator / repeat suppression / fallback
- [x] Step 4: 补测试并回放 trace

## Execution Notes

- Date: 2026-03-29 | Status: completed
- Update: 结合最新 Cloud trace 与远端能力确认，决定把恢复链从“单一 keyframe 请求主路”升级为“轻恢复优先、硬恢复兜底、单飞切换不积压”的分级方案。
- Decision: 远端已支持 `nack:pli` / `ccm:fir`，恢复层将优先尝试 RTCP 轻恢复；只有轻恢复无效或链路证据更强时，才升级到现有控制通道 keyframe / decoder reset。
- Risk/Blocker: 需要确认 `RTCRtpReceiver::write_rtcp` 在当前连接层能稳定拿到 video receiver 与 SSRC；若解析不到，必须可靠 fallback 到现有控制 keyframe 请求。
- Completion: 已完成 PLI/FIR 轻恢复接通、控制通道回退、单飞状态同步与回归测试，最新 trace 验证恢复不再排队积压。
