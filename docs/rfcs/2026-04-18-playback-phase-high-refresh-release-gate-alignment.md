# 播放期高刷新 Host Release Gate 对齐 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: Codex
- Last Updated: 2026-04-18

## Background

- 最新播放期 trace `runtime-trace-1776518192526-1.jsonl` 显示：恢复尾段 `decode_fps` 维持在约 30fps，但 `present_fps` 先掉到约 7fps，随后 decode 端 `outputQueueOverflow` 从上一份日志的 67 次抬升到 214 次。
- 当前 pacer 已引入“真实 host 刷新率 + 视频流帧率”的动态节拍拆分，但 `display_tick_epoch` 的同 tick gate 仍偏保守；在高刷新 host 场景里，这会把 latest-slot 新鲜供给拖慢，导致 decode 队列在恢复尾段持续被挤爆。

## Goal

- 让 pacer 在高刷新 host / 低视频帧率 / present 明显落后的组合下，不再被同 tick gate 过度限速。
- 保持 priming 首帧保护和正常节拍场景的原有 gate 语义不变。
- 通过定向单测锁定这次回归，避免后续动态调度再次把恢复尾段拖回 decode overflow。

## Scope

- In scope:
  - `crates/xbxengine/core/src/media/video/pacer/actor.rs`
  - `crates/xbxengine/core/src/media/video/pacer/actor.test.rs`
  - `docs/project-task.md`
- Out of scope:
  - decode 队列大小或 stale slack 调整
  - native_video presenter 主流程改造
  - owner/recovery 状态机语义重写

## Plan

1. 补失败回归测试，覆盖高刷新 host 下的同 tick release gate 放宽场景。
2. 调整 pacer host release gate，只在“高刷新 + present 落后 + 非 priming”时允许按 host 刷新窗复用同 tick。
3. 跑定向测试并回写任务跟踪。

## Validation

- [x] `cargo test -p xbxengine host_release_gate_allows_same_tick_reuse_when_high_refresh_host_lags_present_feedback -- --nocapture`
- [x] `cargo test -p xbxengine media::video::pacer -- --nocapture`
- [x] `cargo check -p xbxengine`

## Risks

- 若放宽条件过宽，可能增加 latest-slot overwrite，反而加重无效 submit。
- 若只修 pacer gate 而不锁测试，后续动态调度继续演化时仍可能回归。

## Progress

- [x] Step 1: 已从新 trace 确认问题集中在 pacer host release gate 与高刷新 host 组合。
- [x] Step 2: 补失败回归测试并实现修复。
- [x] Step 3: 完成验证与文档回写。

## Execution Notes

- Date: 2026-04-18 | Status: in-progress
- Update: 新问题定位到 pacer host release gate 在高刷新 host 下对 same-tick reuse 过度保守，导致恢复尾段 latest-slot 供给不足并把压力回灌到 decode output queue。
- Decision: 本轮只收敛 pacer gate 条件，不同步扩大到 decode queue 或 owner 阈值调整，先把最直接的供给瓶颈修正到位。
- Date: 2026-04-18 | Status: completed
- Update: `HostPacingContext` 补齐 `host_refresh_interval_ms`，same-tick gate 现仅在 `release_interval_ms` 明显大于 host 刷新窗且 `present_fps` 严重落后 `display_fps` 时允许按 host 刷新窗提前复用；`Priming` 与 `Starved` 现有语义保持不变。
- Update: 新增回归测试 `host_release_gate_allows_same_tick_reuse_when_high_refresh_host_lags_present_feedback`，锁定“144Hz host + 30fps 视频 + present feedback 落后”的场景。
