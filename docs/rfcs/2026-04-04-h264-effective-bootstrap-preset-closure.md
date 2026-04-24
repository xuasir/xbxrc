# H264 Effective Bootstrap Preset Closure RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- `runtime-trace-1775297730377.jsonl` 在开始操作后首个明确异常是 `bootstrapMissingSps`，并非上一轮的 host present 闭环问题。
- 当前 H264 链路把“当前 AU 自带 SPS/PPS 的自举能力”和“解码器基于已提交参数集可继续工作的能力”混成同一套判定，导致 source / ingress / decode / trace 对同一帧可能给出互相矛盾的语义。

## Goal

- 统一 H264 参数集与 bootstrap 语义，消除操作触发后因 preset/parameter set 歧义引发的等待关键帧抖动。
- 让 source admission、ingress waiting-keyframe 和 decoder session 建立都基于同一套 effective decoder config 事实。

## Scope

- In scope:
  - `crates/xbxengine/core/src/media/video/h264/inspection.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs`
  - 相关回归测试与运行态文档收口
- Out of scope:
  - 新一轮 native video present / pacer 语义调整
  - 独立的 keyframe request episode 统计口径清理

## Plan

1. 重新定义 inspection 的 effective bootstrap 语义，允许已提交参数集参与 IDR bootstrap 与参数集合成。
2. 将 source admission 收口为“bootstrap_ready 或 delta_continuation_ready 且 slice header 有效”。
3. 补充定点测试、验证并更新任务跟踪。

## Validation

- [x] `cargo fmt --all`
- [x] `cargo test -p xbxengine committed_parameter_sets_allow_idr_bootstrap_without_inband_sets -- --nocapture`
- [x] `cargo test -p xbxengine partial_inband_parameter_refresh_reuses_committed_counterpart -- --nocapture`
- [x] `cargo test -p xbxengine inspection_admission_rejects_frames_without_bootstrap_or_continuation -- --nocapture`
- [x] `cargo test -p xbxengine media::video::h264::inspection -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::stream::video_source -- --nocapture`
- [x] `cargo check -p xbxengine`

## Risks

- 如果某些流的部分参数集刷新与旧参数集引用关系不兼容，简单合成可能会高估可恢复性。
- source admission 收紧后，之前被“语法有效但语义不完整”放行的帧会更早进入 wait-keyframe，需要确认不会放大误判。

## Progress

- [x] Step 1: 已定位根因到 bootstrap/effective-config 语义混用，并在 inspection 中实现 committed 参数集合成。
- [x] Step 2: 已将 source admission 收口到 bootstrap 或 continuation 二选一的统一条件。
- [x] Step 3: 已完成验证，并同步更新 report / tracker。

## Execution Notes

- Date: 2026-04-04 | Status: in-progress
- Update: 基于 `runtime-trace-1775297730377.jsonl` 复核后，确认首个异常为 `bootstrapMissingSps`；已开始收口 inspection/source 的参数集判定。
- Decision: `bootstrap_ready` 改为表达“当前 AU 在现有 committed 参数集上下文下是否足以作为有效 IDR bootstrap”，不再等价于“当前 AU 必须自带完整 SPS/PPS”。
- Risk/Blocker: 仍需用回归测试确认“部分参数集刷新 + 已提交 counterpart”不会引入误判。
- Date: 2026-04-04 | Status: completed
- Update: `inspection.rs` 已支持 committed 参数集与局部 inband 参数集合成，`source.rs` admission 已收口为 `bootstrap_ready || delta_continuation_ready()`；同步修正并补充 H264/视频源回归测试。
- Decision: 保留 `has_inband_sps/pps` 作为原始输入事实，但把 `bootstrap_ready` 收口为 effective decoder bootstrap 事实，避免 trace 将“缺少 inband 参数集”误报成“当前无法恢复/解码”。
- Risk/Blocker: 真实流若出现“只刷新部分参数集且引用关系不兼容”的罕见编码器行为，后续仍需结合运行态 trace 再确认是否需要引入更严格的引用一致性校验。
