# WebRTC RTCP SSRC Routing Fix RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 当前视频接流侧的 NACK 反馈存在协议常识问题：`TransportLayerNack` 使用了 `sender_ssrc = 0` 和 `media_ssrc = 0`。
- 视频源的 RTCP 发送端在主挂载路径里还是 `DummyRtcpPort`，意味着 NACK 反馈即使组包，也不会真正进入 WebRTC 发送链路。
- `PLI / FIR / REMB` 也使用了 `sender_ssrc = 0`，虽然不一定立刻失效，但不符合 WebRTC 互操作常识，也会让排障信号失真。

## Goal

- 让视频接流侧的 RTCP 反馈真正进入 WebRTC 连接发送链路。
- 让 NACK 使用真实的视频媒体 SSRC，而不是 0。
- 让 PLI / FIR / REMB 使用非 0 的本地 sender SSRC。
- 保持现有恢复、补包和 H264 主链逻辑不变，只修协议与路由语义。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/stack/media_pipeline.rs`
  - `crates/xbxengine/core/src/transport/rtc/connection/service.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/*`
  - `crates/xbxengine/core/src/transport/rtc/connection/tests/service.rs`
  - `docs/project-task.md`
- Out of scope:
  - recovery policy 阈值调整
  - H264 bootstrap / inspection 语义变更
  - 新增独立传输栈或替代 RTC 路由

## Plan

1. 增加一个真实的 RTCP 发送桥，让视频源侧 `send_rtcp` 落到 connection service 的 `receiver.write_rtcp()`。
2. 给视频源补一个非 0 的本地 NACK sender SSRC，并把 NACK 的 media SSRC 绑定到当前视频流。
3. 把 PLI / FIR / REMB 的 sender SSRC 收敛成连接级的非 0 本地值，并补回归测试。

## Validation

- [x] `cargo fmt --all`
- [x] `cargo test -p xbxengine transport::rtc::connection::service -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::stream::video_source -- --nocapture`
- [x] `cargo check -p xbxengine`

## Risks

- RTCP raw bytes 需要在发送桥里正确反序列化，否则会把现有反馈链切断。
- 真实的 video media SSRC 可能在重建/重连时变化，需要确保 NACK 发送端使用最新值。
- 连接级 sender SSRC 需要保持非 0 且在一次连接周期内稳定。

## Progress

- [x] Step 1: RTCP 发送桥已接入真实 connection receiver
- [x] Step 2: NACK / PLI / FIR / REMB 的 SSRC 已收敛到真实值
- [x] Step 3: 测试已补齐并通过

## Execution Notes

- Date: 2026-04-03 | Status: completed
- Update: 已完成 RTCP 发送桥、NACK 媒体 SSRC、PLI/FIR/REMB sender SSRC 的收敛，并补齐服务测试与整包验证。
- Decision: 以真实 connection receiver 作为 RTCP 发送落点，视频源通过 connection service 写入 RTCP；NACK 绑定当前视频流媒体 SSRC，连接级反馈使用非 0 本地 sender SSRC。
- Risk/Blocker: 当前实现通过整包测试验证可用，后续仍需关注 WebRTC crate 升级时 RTCP receiver / sender API 的兼容性。
