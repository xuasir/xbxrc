# Connected / MediaReady Milestones Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-10-connected-media-ready-milestones.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-10-connected-media-ready-milestones.md)
- 本轮已完整交付双里程碑收口：将“连接成功”与“媒体可稳定展示”拆分为独立里程碑，贯通 Rust runtime、协议、Tauri bridge、前端 runtime contract、状态文案与诊断字段。

## Delivered

- 新增 `presentationMilestone` 领域语义，明确 `Connected / MediaReady / Degraded / Failed / Closed`。
- 保留 `MediaVideoReady` 的历史协商/分辨率 ready 语义，并新增独立里程碑事件避免混淆。
- 在前端状态与文案层落实“已连接，等待画面稳定”与“画面已稳定”的分离表达。

## Changes

- `xbxengine protocol` 新增呈现里程碑 DTO、runtime event 与 stats 字段，支持阶段耗时与失败阶段透出。
- `xbxengine runtime` 基于 transport、control、track、packet 与 present freshness 做统一判定，并把状态同步到 snapshot / diagnostics。
- `Tauri + frontend` 增加 `presentation.milestoneChanged` 事件桥接与消费逻辑，同步更新 runtime host、页面 view-state、诊断快照与 i18n 文案。

## Validation

- `cargo test -p xbxengine api::runtime -- --nocapture`
- `cargo test -p xbxengine diagnostics::stats -- --nocapture`
- `pnpm exec tsc --noEmit`

## Risks

- `MediaReady` 当前首版仍以视频 present freshness 为主，尚未把音频 playout / AV sync 纳入硬门槛。
- browser runtime 的 `MediaReady` 仍基于首个 `frameReady` 推断，语义上已与 Rust runtime 对齐，但精度略低于 Rust 侧的多信号判定。

## Follow-up

- 如后续需要更强的启动阶段诊断，可把 `Connected -> MediaReady` 的失败原因进一步细化为 keyframe / decoder / renderer 子阶段。
- 若 UI 需要展示更明确的恢复态，可继续把 `Degraded` 文案与 overlay/诊断面板可视化做细分。
