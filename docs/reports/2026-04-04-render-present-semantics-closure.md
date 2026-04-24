# Render/Present Semantics Closure Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-04-render-present-semantics-closure.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-04-render-present-semantics-closure.md)
- 已完成 render / host enqueue / host present 三层语义收口，修复恢复输入、诊断投影、trace 命名与 runtime 事件中的残留歧义。

## Delivered

- 恢复逻辑中的媒体进展信号已明确区分 decode 与 host present，不再把 rendered snapshot/health cache 回退成 present freshness。
- diagnostics / trace / native presenter 已统一把宿主计数表述为 enqueue，而不是冒充 present 成功计数。
- xbxengine runtime 事件与上层 runtime host 已把 renderer 时间链路改成 `videoFrameRendered / frameReady`，不再冒充 `framePresented`。

## Changes

- `core` 侧将 `XbxEngineMediaSignal` 的 present 语义改为显式 `latest_frame_presented_at_ms`，并补回归测试验证 host present 缺失时不会被 rendered 时间掩盖。
- `diagnostics` 侧移除了“累计 submit 即当前可见输出”的判据，`has_visible_video_output` 现在只认 host present 事实。
- `native_video` / `trace` / `frontend runtime` 侧统一了 `present_enqueue_count_total`、`rendererFrameTimeMs`、`frameReady` 等命名，避免 submit/rendered/present 三套口径继续混用。

## Validation

- `cargo fmt --all`
- `cargo test -p xbxengine runtime_does_not_use_rendered_snapshot_as_present_freshness_signal -- --nocapture`
- `cargo test -p xbxengine build_stats_keeps_priming_when_only_submit_count_exists_without_host_present -- --nocapture`
- `cargo test -p xbxengine recovery_signals_request_keyframe_before_reconnect -- --nocapture`
- `cargo test -p xbxrc host_present_state_projects_no_pending_supply_signals -- --nocapture`
- `cargo check -p xbxengine -p xbxrc`
- `pnpm exec tsc -p tsconfig.json --noEmit`

## Risks

- `core` 内部仍保留少量历史字段名含 `submit`，但它们已不再作为 visible output 判据或对外主展示口径；后续若要进一步做纯内部重命名，需单独安排一次低风险清理。
- 浏览器 runtime 仍沿用 player 内部的 `stats.videoFrameProcessed` 事件名，那条链是浏览器播放器自有合同，不在本轮 Rust-owned runtime 收口范围内。

## Follow-up

- 下一份 runtime trace 到来时，优先确认 `hostPresentState.presentEnqueueCountTotal`、`presentAgeMs` 与 `frameReady` 首帧时序是否和预期一致。
- 如果后续继续收口浏览器 runtime，可把 player 内部 `stats.videoFrameProcessed` 也升级成与 Rust-owned runtime 一致的 `videoFrameRendered` 命名。
