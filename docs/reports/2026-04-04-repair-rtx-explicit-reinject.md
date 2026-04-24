# Repair RTX Explicit Reinject Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-04-repair-rtx-explicit-reinject.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-04-repair-rtx-explicit-reinject.md)
- 本轮已完成 repair/RTX 显式 provenance + source 阶段化 reinject 收口，让 repair 包不再以“看起来像 primary”的隐式改写形态进入主视频消费链。

## Delivered

- 为 `RtcVideoRtpPacket` 引入显式 ingress provenance，区分 `Primary`、`RepairPrimaryPassThrough`、`RtxReinject`。
- 收紧 sink 侧 repair/RTX 识别逻辑，去掉缺少 payload map 时基于 `pt=97` 的乐观猜测。
- 让 source 按包来源直接推进 reinject stage，并补齐对应 sink/source/diagnostics 回归。

## Changes

- [`packet_types.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/packet_types.rs) 新增 `RtcVideoRepairMetadata`、`RtcVideoIngressKind`，`RtcVideoRtpPacket` 增加 `ingress_kind` 字段；[`test_fixtures.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/test_fixtures.rs) 默认构造 `Primary`。
- [`sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/sink.rs) 将 repair-primary 直通与 RTX 解包都改为显式 provenance 投递；`is_rtx_payload` 不再在无 map 时猜测 RTX；`unpack_rtx_packet` 只有在能解析出 primary payload type 时才 reinject；成功入队后立即记录 `queued` observation。
- [`video_source/mod.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/mod.rs) 注入共享 `RuntimeStatsSink`；[`source.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs) 改为由 provenance 驱动 `adapterRead / sampleBuilderPush / adapterResolved / adapterResolveMiss` 阶段推进，并避免 `RtxReinject` 污染当前 primary SSRC。

## Validation

- `cargo test -p xbxengine transport::rtc::stream::video_source::sink -- --nocapture`
- `cargo test -p xbxengine transport::rtc::stream::video_source::source -- --nocapture`
- `cargo test -p xbxengine diagnostics::stats -- --nocapture`
- `cargo check -p xbxengine`

## Risks

- 当前方案仍是沿现有 `sink -> source` 主路径做最小闭环增强，没有引入独立 repair quarantine 或专门的 reinject worker；如果后续 trace 仍显示 repair 洪峰与主视频排队相互干扰，还需要继续细化隔离策略。
- sink 侧现在改成“识别更保守”，这会降低误投递，但如果远端协商或 payload map 本身异常，repair 包会更早被丢弃；后续仍需结合真实 trace 评估是否需要补更强的协商异常观测。

## Follow-up

- 用新的 runtime trace 复核 `queued -> adapterRead -> sampleBuilderPush -> adapterResolved` 的 stage 链是否与实际 repair 命中 gap 的时间关系一致。
- 如果后续仍发现 repair/RTX 与 primary 排队竞争导致组帧抖动，再评估是否需要把显式 provenance 继续演进为带限流/隔离策略的 reinject 队列。
