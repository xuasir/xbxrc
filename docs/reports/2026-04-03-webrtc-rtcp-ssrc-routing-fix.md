# WebRTC RTCP SSRC Routing Fix Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-03-webrtc-rtcp-ssrc-routing-fix.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-03-webrtc-rtcp-ssrc-routing-fix.md)
- 已完成 WebRTC RTCP 反馈路由与 SSRC 常识修复，让视频源侧 RTCP 反馈真正进入 connection 发送链，并把 NACK / PLI / FIR / REMB 的 sender / media SSRC 收敛到真实值

## Delivered

- 视频源侧 `send_rtcp` 已接入真实 connection RTCP 发送桥，不再使用 `DummyRtcpPort`
- NACK 发送使用当前视频流的真实 media SSRC，sender SSRC 使用连接级非 0 本地值
- PLI / FIR / REMB 的 sender SSRC 已收敛到连接级非 0 本地值

## Changes

- 在 connection service 中新增真实 RTCP 发送入口，并在重建时刷新本地 RTCP sender SSRC
- 在视频源中记录当前视频流 SSRC，并据此发送 NACK batch
- 在 connection tests 中补齐 RTCP 路由与 SSRC 语义回归测试，并修复相关测试变量绑定问题

## Validation

- `cargo fmt --all`
- `cargo test -p xbxengine transport::rtc::connection::service -- --nocapture`
- `cargo test -p xbxengine transport::rtc::stream::video_source -- --nocapture`
- `cargo check -p xbxengine`

## Risks

- WebRTC crate 后续升级时，RTCP receiver / sender API 可能发生兼容变化
- 连接级 sender SSRC 仍需保持在一次连接周期内稳定，重建后刷新是必要前提

## Follow-up

- 后续实机若再出现 RTCP 反馈无效，优先检查 `preferred_video_feedback_target()` 与当前 receiver 绑定是否一致
- 如 WebRTC crate 升级，优先复核 RTCP raw payload 反序列化和 `write_rtcp` 路径是否保持兼容
