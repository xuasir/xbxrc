# Keyframe Request / Response Trace Verification RFC

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 当前 recovery trace 已能看到 `requestKeyframe`、`videoTimelineObserved`、`nack` 和恢复升级，但还不能稳定回答“某次 keyframe 请求之后，客户端到底有没有收到一个可用的 keyframe 响应”。
- 现有日志只能分别证明“请求发出”和“客户端后续看见 keyframe / 继续卡住”，缺少单一 episode 的请求-响应闭环。
- 本次目标是把 keyframe 请求和客户端可见的响应统一到同一个 trace episode，方便实机测试时直接判断“服务端未响应 / 响应太晚 / 已响应但中途丢失”。

## Goal

- 在 runtime stats 中引入统一的 keyframe request episode 观测。
- 让 trace 能回看一次 episode 的请求时间、发送方式、首个 keyframe packet / decode 响应时间与是否超时。
- 不改变恢复策略本身，只补可回归证据。

## Scope

- In scope:
  - `crates/xbxengine/core/src/api/backend.rs`
  - `crates/xbxengine/protocol/src/runtime.rs`
  - `crates/xbxengine/core/src/runtime_stats_sink.rs`
  - `crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs`
  - `crates/xbxengine/core/src/transport/rtc/connection/service.rs`
  - `crates/xbxengine/core/src/transport/rtc/pipeline/session_loop.rs`
  - `crates/xbxengine/core/src/media/video/decode/video_decode.rs`
  - `src-tauri/src/mods/xbxengine/trace_projection.rs`
- Out of scope:
  - 服务端仓库外的服务器实现改造
  - 恢复策略调参
  - 前端 UI 额外展示

## Plan

1. 在 stats / protocol 中增加 `latest_keyframe_request_episode` 观测结构。
2. 在 transport command、connection send、session ingress、decode accept 四个点回填 episode 状态。
3. 在 trace projection 中新增稳定的 `keyframeRequestEpisode` 投影与响应事件。
4. 补测试并用真实 trace 验证 episode 闭环。

## Validation

- [x] `cargo fmt --all`
- [x] `cargo test -p xbxengine keyframe_request_episode -- --nocapture`
- [x] `cargo test -p xbxengine media::video::decode::actor -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::connection::service -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::stack::transport_session -- --nocapture`
- [x] `cargo test -p xbxrc build_observability_snapshot_includes_latest_keyframe_episode -- --nocapture`
- [x] `cargo test -p xbxrc record_runtime_trace_observations_projects_keyframe_episode_lifecycle -- --nocapture`
- [x] `cargo check -p xbxengine`

## Risks

- 客户端 trace 只能证明“客户端收到了 keyframe packet / decoded keyframe”，不能单独证明服务端内部是否已经编码并发送，除非服务端侧也补同样的 episode id。
- 新 episode 字段若更新过多，可能和现有 recovery snapshot 竞争可读性，所以只保留请求 / sent / packet seen / decoded / verdict 这几个核心阶段。

## Progress

- [x] Step 1: RFC 已建立
- [x] Step 2: stats / transport / decode 记录已实现
- [x] Step 3: trace projection 已实现
- [x] Step 4: 测试与实机回归已完成

## Execution Notes

- Date: 2026-04-03 | Status: complete
- Update: 已把 keyframe request episode 串到 runtime stats / transport / decode / trace projection，实机 trace 可直接回看 request -> sent -> packet seen -> decoded -> on-time/late/missed。
- Decision: 以客户端可见的 keyframe packet / decode 作为“服务端已响应”的可回归证据，不额外改动服务端仓库外的实现。
- Risk/Blocker: 若后续需要证明“服务端内部已接收请求”，仍需服务端侧补同样的 episode id 日志。
