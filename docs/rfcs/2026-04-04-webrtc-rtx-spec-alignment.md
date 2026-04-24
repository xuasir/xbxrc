# WebRTC RTX Spec Alignment RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 上一轮已把 repair/RTX 从“隐式改写成 primary 包”收口为“显式 provenance 进入 source”，提升了可观测性和误投递防护。
- 但当前 [`video_source/sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/sink.rs) 的 RTX 解包仍只恢复了 OSN 和 apt 对应的 primary PT，没有恢复 FID 关联下的 primary SSRC。
- 对 WebRTC / RFC 4588 语义来说，RTX 包在 receiver 侧解包后应恢复为“原始媒体包”的主流身份；如果缺少 `apt` / `ssrc-group:FID` 等关键协商信息，不应该靠猜测把 repair 包继续混入主视频流水线。

## Goal

- 让当前 repair/RTX 路径在 SSRC、payload type、OSN 恢复上与 WebRTC RTX 规范对齐。
- 把 answer SDP 中的 `apt` 与 `ssrc-group:FID` 关系显式接入 `payload_route_map`，让 sink 在解 RTX 时能够恢复 primary SSRC。
- 在缺少关键协商信息时采取保守丢弃，而不是继续猜测 repair -> primary 归属。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/stream/packet_router.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/sink.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs`
  - 相关 tests / diagnostics glue
- Out of scope:
  - 新增独立 repair worker / quarantine actor
  - 改写整个媒体路由服务
  - 处理 RED / ULPFEC / FlexFEC 的完整解码链

## Plan

1. 扩展 `RtcPayloadRouteMap`，解析 answer SDP 中的 RTX `apt` 与 `ssrc-group:FID` 主辅 SSRC 关系。
2. 调整 sink 的 RTX 归一化逻辑，让 de-RTX 后恢复 primary PT + primary SSRC；缺 `apt` / FID 时保守拒绝 reinject。
3. 收紧 source 侧 provenance / 统计分支，确保 repair SSRC 只作为 provenance 保留，不再污染解包后的主媒体身份。
4. 补齐 packet_router / sink / source 回归，覆盖 FID 映射、缺失协商信息拒绝 reinject、阶段更新一致性。

## Validation

- [x] `cargo test -p xbxengine transport::rtc::stream::packet_router -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::stream::video_source::sink -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::stream::video_source::source -- --nocapture`
- [x] `cargo check -p xbxengine`

## Risks

- 如果 answer SDP 并不总是带 `ssrc-group:FID`，过于严格的规范收口可能导致 repair 包更早被丢弃，需要结合真实 trace 观察兼容性。
- `RepairPrimaryPassThrough` 目前承担了一部分历史兼容语义；在规范收紧后，这条路径可能需要进一步降级为仅观测、不再参与主链。

## Progress

- [x] Step 1: 明确当前实现与 WebRTC RTX 规范的差距
- [x] Step 2: 完成 apt/FID 驱动的 RTX 身份恢复
- [x] Step 3: 完成定向测试与编译验证
- [x] Step 4: 回填 RFC / Report / 任务跟踪

## Execution Notes

- Date: 2026-04-04 | Status: planned
- Update: 用户要求继续推进 repair/RTX 改造，并明确要求遵循 WebRTC 对 RTX 处理的规范；本轮将把重点从“显式 provenance”进一步推进到“恢复原始媒体身份 + 去猜测化”。
- Decision: 延续现有 `payload_route_map + sink/source` 主路径，不引入并行大系统；以 `apt + ssrc-group:FID` 为最小必需协商信息。
- Date: 2026-04-04 | Status: completed
- Update: 已在 [`packet_router.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/packet_router.rs) 为 answer SDP 补齐 `a=ssrc-group:FID` 解析，并把 repair SSRC -> primary SSRC 关系收进 `RtcPayloadRouteMap`；[`sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/sink.rs) 的 de-RTX 现改为同时恢复 OSN、apt 对应的 primary PT 与 FID 对应的 primary SSRC。
- Decision: RTX reinject 现在把 `apt + FID` 视为最低必需协商信息，不再保留“唯一主 PT fallback”之类的猜测式恢复；缺 `apt`、缺 FID、缺 payload map 时统一保守丢弃。
- Update: [`source.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs) 侧改为直接按归一化后的主媒体 SSRC 推进 reinject observation，repair SSRC 仅保留在 provenance 中，不再参与 `primary_ssrc` 推断；已补 packet_router / sink / source 定向回归并完成 `cargo test -p xbxengine transport::rtc::stream::packet_router -- --nocapture`、`cargo test -p xbxengine transport::rtc::stream::video_source::sink -- --nocapture`、`cargo test -p xbxengine transport::rtc::stream::video_source::source -- --nocapture`、`cargo check -p xbxengine`。
