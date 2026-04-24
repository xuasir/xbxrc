# H264 Effective Bootstrap Preset Closure Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-04-h264-effective-bootstrap-preset-closure.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-04-h264-effective-bootstrap-preset-closure.md)
- 已完成 H264 bootstrap / preset 语义收口：让已提交参数集可参与 effective bootstrap，修正 source admission 与 trace 侧的歧义来源，避免开始操作后因参数集上下文不一致而反复掉入 wait-keyframe。

## Delivered

- 在 [`crates/xbxengine/core/src/media/video/h264/inspection.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/h264/inspection.rs) 将当前 AU 的局部参数集与 committed 参数集合成为 effective decoder config。
- 在 [`crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs) 把 inspection admission 收口为统一的 bootstrap/continuation 判定。
- 补齐 H264 inspection 与 video source 的回归测试，覆盖 committed 参数集 bootstrap、局部参数集刷新复用、以及 admission 拒绝无 bootstrap/continuation 的帧。

## Changes

- `bootstrap_ready` 不再等价于“当前 AU 必须自带完整 SPS/PPS”，而是表示“当前 AU 在 committed 参数集上下文下足以作为有效 IDR bootstrap”。
- `parameter_sets` 现在会在缺少单边 inband 参数集时复用 committed counterpart，避免把局部 preset 刷新误判为 bootstrap 缺失。
- `resolve_inspection_admission()` 不再仅依赖 `slice_headers_valid`，从而阻止“语法可解析但既不能 bootstrap 也不能 continuation”的帧继续流入后续阶段。

## Validation

- `cargo fmt --all`
- `cargo test -p xbxengine committed_parameter_sets_allow_idr_bootstrap_without_inband_sets -- --nocapture`
- `cargo test -p xbxengine partial_inband_parameter_refresh_reuses_committed_counterpart -- --nocapture`
- `cargo test -p xbxengine inspection_admission_rejects_frames_without_bootstrap_or_continuation -- --nocapture`
- `cargo test -p xbxengine media::video::h264::inspection -- --nocapture`
- `cargo test -p xbxengine transport::rtc::stream::video_source -- --nocapture`
- `cargo check -p xbxengine`

## Risks

- 这次修复收口了 effective bootstrap 语义，但没有额外引入“SPS/PPS 引用 id 与语义内容不一致”的深校验，极端编码器行为仍需后续 trace 验证。
- `keyframeRequestEpisode` 在同一响应 keyframe 后持续更新 `responseFrameSeq` 的统计口径问题仍在，这不是本次修复范围。

## Follow-up

- 用下一份真实运行 trace 确认 `bootstrapMissingSps` 是否从“恢复主因”降为仅在真正无 committed 参数集时才出现。
- 若后续仍出现“操作触发 source/decoder 上下文重新分叉”，继续追 `keyframeRequestEpisode` 与可能的 source lifecycle 重建链路。
