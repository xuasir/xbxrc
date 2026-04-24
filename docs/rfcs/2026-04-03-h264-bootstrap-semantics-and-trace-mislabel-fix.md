# H264 Bootstrap Semantics and Trace Mislabel Fix RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 当前 `inspectionRejectMissingSps` 在 trace 中会被当成“播放链路拒绝”，但实机分析表明，很多 delta slice 本来就不带 SPS/PPS，只要 committed 参数集已存在就应该继续承接。
- 现有 `h264InspectionRejected` 事件把“bootstrap 不完整的观测”与“实际 admission 拒绝”混在一起，容易误导回归分析。
- 需要把 H264 inspection 的 bootstrap 语义、delta slice 承接语义和 trace 事件命名统一起来。

## Goal

- 修正 `MissingSps` 的语义边界，让它只表示当前 AU 的 bootstrap 缺口，不再暗示 delta slice 解析失败。
- 增强 delta slice 在 committed 参数集存在时的承接/解析可观测性，确保它们可以被正常接受并进入 decode/render 主链路。
- 将 `h264InspectionRejected` 改成不误导的 trace 语义，区分“观测到 bootstrap 缺口”和“实际被 admission 拒绝”。

## Scope

- In scope:
  - `crates/xbxengine/core/src/media/video/h264/inspection.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs`
  - `crates/xbxengine/core/src/media/video/decode/video_decode.rs`
  - `crates/xbxengine/core/src/api/backend.rs`
  - `crates/xbxengine/core/src/runtime_stats_sink.rs`
  - `crates/xbxengine/protocol/src/runtime.rs`
  - `src/shared/rpc/xbxengine.ts`
  - `crates/xbxengine/core/src/diagnostics/stats.rs`
  - `src-tauri/src/mods/xbxengine/trace_projection.rs`
  - 相关单测与 trace 回归测试
- Out of scope:
  - 更换 H264 解析依赖
  - 改动 WebRTC transport 主链路或 keyframe 请求策略
  - 额外引入第二套视频管线

## Plan

1. 先把 H264 inspection / admission 语义拆清楚：保留 bootstrap 缺口判断，但增加能够区分“实际 rejected”与“仅 bootstrap incomplete”的观测字段。
2. 再修正 delta slice 承接路径：在 committed SPS/PPS 已存在时，确保普通 delta slice 不被误判成播放失败，并补齐关键单测。
3. 最后收口 trace / DTO / 前端投影：把 `h264InspectionRejected` 改成不误导的事件名或状态语义，更新回归用例与项目追踪。

## Validation

- [x] `cargo fmt --all`
- [x] `cargo check -p xbxengine`
- [x] `cargo test -p xbxengine media::video::h264::inspection -- --nocapture`
- [x] `cargo test -p xbxengine media::video::ingress::scheduler -- --nocapture`
- [x] `cargo test -p xbxengine media::video::decode::video_decode -- --nocapture`
- [x] `cargo test -p xbxrc trace_projection -- --nocapture --test-threads=1`
- [x] `pnpm exec vue-tsc --noEmit --pretty false`

## Risks

- Trace 事件命名调整可能影响现有分析脚本和前端诊断面板，需要同步更新。
- 如果 delta slice 相关逻辑和 bootstrap 语义没有彻底分离，容易出现“修了 trace 名称但语义仍旧混淆”的假完成。
- H264 逻辑改动跨多个 Rust 模块，需避免在 committed 参数集与 decode session 重建之间引入回归。

## Progress

- [x] Step 1: 拆分 inspection/admission 语义并定义新观测字段
- [x] Step 2: 修正 delta slice 承接与解析路径
- [x] Step 3: 修正 trace 事件命名与投影

## Execution Notes

- Date: 2026-04-03 | Status: completed
- Update: 已确认 `MissingSps` 在多数场景下只是 bootstrap 缺口，不应被 trace 命名成“实际 rejected”。
- Decision: 采用“观测 bootstrap 缺口”与“实际 admission 拒绝”分离的设计。
- Risk/Blocker: 需要同步更新 trace 投影与回归测试，否则新语义无法在实机 trace 中稳定回放。
