# WebRTC RTX Spec Alignment Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-04-webrtc-rtx-spec-alignment.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-04-webrtc-rtx-spec-alignment.md)
- 本轮已把 video source 的 RTX 处理从“恢复 PT/OSN 但保留 repair 身份”收口为“按 WebRTC RTX 协商恢复原始 primary 身份，并在缺少协商信息时拒绝 reinject”。

## Delivered

- 在 `RtcPayloadRouteMap` 中补齐 `apt` 与 `ssrc-group:FID` 的联合映射，建立 repair SSRC -> primary SSRC 关系。
- 让 sink 的 de-RTX 路径恢复 primary SSRC + primary PT + OSN，不再把 repair SSRC 混入主视频包身份。
- 去掉唯一主 PT fallback，改为 `apt/FID` 缺失即保守丢弃，并补齐对应回归。

## Changes

- [`packet_router.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/packet_router.rs) 新增 `repair_video_ssrc_primary_ssrc` 与 `parse_ssrc_group_fid_line()`，answer SDP 现可解析 `a=ssrc-group:FID <primary> <repair>`。
- [`sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/sink.rs) 的 `unpack_rtx_packet()` 现要求同时拿到 `apt` 和 FID 映射，归一化后把 `meta.ssrc` 恢复为 primary SSRC；repair-route primary payload 直通也要求显式 FID 映射。
- [`source.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs) 的 reinject observation 改为直接使用归一化后的 primary SSRC，repair SSRC 仅保存在 provenance 中；`repair_rtx_packet_keeps_explicit_provenance_through_source_stage_updates` 也同步校正为 `primary_ssrc == 归一化后的媒体 SSRC`。

## Validation

- `cargo test -p xbxengine transport::rtc::stream::packet_router -- --nocapture`
- `cargo test -p xbxengine transport::rtc::stream::video_source::sink -- --nocapture`
- `cargo test -p xbxengine transport::rtc::stream::video_source::source -- --nocapture`
- `cargo check -p xbxengine`

## Risks

- 当前实现把 `apt + FID` 作为 reinject 的最小协商门槛，这比旧行为更严格；如果某些真实 answer SDP 缺失 `ssrc-group:FID`，repair/RTX 会更早被丢弃。
- `RepairPrimaryPassThrough` 仍保留历史兼容语义，但现在同样要求显式 FID；如果未来 trace 里这条分支持续没有真实价值，可以进一步降为仅观测。

## Follow-up

- 用新的真实 runtime trace 验证 answer SDP 是否稳定带 `ssrc-group:FID`，并确认 RTX 包的 `primary_ssrc` 已与主视频流保持一致。
- 如果后续 trace 仍显示 repair/RTX 洪峰会干扰 primary 排队，再在当前显式主身份恢复的基础上评估独立 reinject 限流/隔离策略。
