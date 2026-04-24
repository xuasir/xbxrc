# Video Recovery Success Consumption Fix Report

## Delivered

- 在 `RtcVideoFrameSource` 的 clean keyframe 成功提交路径上补了 `record_transport_clean_anchor(...)`，让生产路径真正写入 `video_anchor_clean_epoch` / `video_anchor_clean_source_event`。
- 保持原有 H264 inspection、timeline、frame recovery ledger 逻辑不变，只补“成功恢复事实”写入与消费闭环。
- 新增回归测试，确认 clean keyframe 提交会关闭 transport recovery episode，并把 clean anchor 事实落到 runtime stats。

## Changes

- `crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs`
  - clean keyframe 提交时调用 `runtime_stats.record_transport_clean_anchor(...)`
  - 增加最小回归测试，覆盖 clean anchor 写入与 recovery episode 关闭
- `crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`
  - 无代码变更，但已有 owner 消费逻辑已可在 clean anchor 事实写入后退出 `rebuilding-supply`
- `docs/rfcs/2026-04-03-video-recovery-success-consumption-fix.md`
  - 更新为完成态
- `docs/project-task.md`
  - 记录任务完成结果

## Validation

- `cargo fmt --all`
- `cargo test -p xbxengine transport::rtc::stream::video_source::source -- --nocapture`
- `cargo test -p xbxengine transport::rtc::policy::video_scheduling_owner -- --nocapture`
- `cargo check -p xbxengine`
- `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`

## Residual Risks

- 这次修复已经把 clean anchor 事实接回生产路径，但仍建议后续用一次真实 trace 确认 `video_anchor_clean_epoch` 在恢复窗口里稳定出现，并且 owner 能及时回到 `stable-serving`。

## Follow-ups

- 下一次实机回归重点观察 `video_anchor_clean_epoch`、`video_owner_state`、`waitingKeyframe` 是否同步退出。
