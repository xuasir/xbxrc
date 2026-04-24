# 基于队列历史的显示侧调度改造 Report

## Summary

- 本轮完成了显示侧调度第一步收口：将 Rust `pacer` 从纯单帧 deadline/catch-up 判定，升级为“队列历史 + 宿主供给压力”共同驱动的最小 pacing 策略。
- 保持了既有 `renderer/latest-slot/native_video` 外部接口不变，仅在 `pacer` 内部引入小队列和受控丢帧水位。

## Delivered

- 在 [`crates/xbxengine/core/src/media/video/render/pacer.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/render/pacer.rs) 新增 `HostPacingPressure`、`QueueHistoryController` 与 `QueuePressureDecision`，实现“短突发宽容、持续积压收紧”的 drop target 策略。
- 在 [`crates/xbxengine/core/src/media/video/pacer/actor.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/media/video/pacer/actor.rs) 引入 `pacing_queue`（上限 3），并消费 runtime stats 中已有的 `host_display_interval_ms`、`host_frame_age_budget_ms`、`host_no_pending_pressure_level`、`host_no_pending_streak`、`video_present_overwrite_count_total/video_present_submit_count_total`。
- 新增结构化 pacer 丢帧细节：`queueCap`、`queuePressure`、`queuePressureAggressive`、`deadline`，继续沿用 `record_pipeline_frame_drop` 进入现有观测链。

## Validation

- `cargo fmt --all`
- `cargo test -p xbxengine media::video::render::pacer -- --nocapture`
- `cargo test -p xbxengine transport::rtc::policy::video_scheduling_owner -- --nocapture`
- `cargo test -p xbxengine diagnostics::stats -- --nocapture`
- `cargo check -p xbxengine`

## Residual Risks

- 当前 `pacer` 仍是单线程 actor 内部顺序 drain，并未进一步演进到“等待期间继续收包”的更细粒度事件循环，后续若继续压低显示侧延迟，仍可再拆。
- 本轮使用的宿主压力事实以 `no-pending` 与 overwrite 为主，尚未把 host present drop ratio 和更多 viewport 侧节奏历史一起接入。

## Follow-up

- 下一轮可继续补 `pacer/actor` 级别的纯 helper 单测，验证持续高压下队列会稳定回落到低水位。
- 若后续继续推进 Moonlight 对齐，可再评估 decode -> pacer 的 pull-model 与宿主 display tick 联动。
