# Keyframe Request / Response Trace Verification Report

> 本 Report 记录复杂任务的最终交付结果，不记录中间过程；中间过程已回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-03-keyframe-request-response-trace-verification.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-03-keyframe-request-response-trace-verification.md)
- 已完成 keyframe request episode 的 trace 可回归闭环，能区分请求已发、客户端看见首个 keyframe packet、首个 keyframe decoded，以及 on-time / late / missed verdict。

## Delivered

- 在 runtime stats 中加入单一 `latest_keyframe_request_episode` 观测对象，并贯通 protocol DTO。
- 在 transport / connection / session loop / decode actor 中补齐 request、sent、packet seen、decoded、timeout 回写。
- 在 trace projection 中新增 `keyframeRequestEpisode` 状态投影与稳定事件名，便于实机 JSONL 回放与 grep。

## Changes

- `crates/xbxengine/core/src/runtime_stats_sink.rs` 新增 keyframe episode 的 requested / sent / packet seen / decoded / missed helpers。
- `crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs` 在 `RequestKeyframe` 入口登记 episode 请求。
- `crates/xbxengine/core/src/transport/rtc/connection/service.rs` 与 `crates/xbxengine/core/src/transport/rtc/connection/data_channel.rs` 在实际发送 keyframe 请求时登记 sent，并在 pump 收尾补 timeout / missed。
- `crates/xbxengine/core/src/transport/rtc/pipeline/session_loop.rs` 与 `crates/xbxengine/core/src/media/video/decode/actor.rs` 分别登记首个 keyframe packet seen 与 decoded。
- `src-tauri/src/mods/xbxengine/trace_projection.rs` 新增 `keyframeRequestEpisode` state / event 投影，并将 episode 嵌入 observability snapshot。

## Validation

- `cargo fmt --all`
- `cargo check -p xbxengine`
- `cargo test -p xbxengine keyframe_request_episode -- --nocapture`
- `cargo test -p xbxengine media::video::decode::actor -- --nocapture`
- `cargo test -p xbxengine transport::rtc::connection::service -- --nocapture`
- `cargo test -p xbxengine transport::rtc::stack::transport_session -- --nocapture`
- `cargo test -p xbxrc build_observability_snapshot_includes_latest_keyframe_episode -- --nocapture`
- `cargo test -p xbxrc record_runtime_trace_observations_projects_keyframe_episode_lifecycle -- --nocapture`

## Risks

- 当前只能证明客户端侧是否收到了 keyframe packet、是否完成 decode，不能单独证明服务端内部 encode / send 是否成功，除非服务端也补同样的 episode id 日志。
- `cargo test -p xbxrc trace_projection -- --nocapture` 里仍有若干与本次 keyframe episode 无关的既有失败项，未在本轮修复范围内。

## Follow-up

- 实机测试时优先检查 `keyframeRequestEpisode` 状态对象和 `keyframeRequestEpisode*` 事件是否完整出现。
- 如需证明服务端内部响应，再在服务端侧补对应 episode id 日志。
