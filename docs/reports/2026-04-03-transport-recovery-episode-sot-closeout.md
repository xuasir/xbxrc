# transport recovery episode SOT closeout Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-03-27-timeline-value-runtime-upgrade.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-27-timeline-value-runtime-upgrade.md)
- 本任务已完成 `transport recovery episode` 收口里最后一段 anchor candidate ledger 语义，确认 anchor 只在当前 recovery episode 内有效。

## Delivered

- 收口 `clean anchor` 判定为 current-episode only，不再接受 `epoch` 差值 grace。
- 保持同 episode 的 `anchor candidate ledger` 仍可作为恢复完成证据。
- 补齐 owner / coordinator 回归测试，覆盖同 episode 有效、跨 episode 失效两类行为。

## Changes

- [`crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs) 的 clean-anchor 判定去掉 epoch grace，仅认 current recovery epoch。
- [`crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 同步改为 current-episode anchor / candidate ledger 语义，并新增 candidate-ledger 回归。
- [`docs/project-task.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/project-task.md) 更新为完成态，指向本次 report。

## Validation

- `cargo fmt --all`
- `cargo test -p xbxengine transport::rtc::policy::video_scheduling_owner -- --nocapture`
- `cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`
- `cargo check -p xbxengine`

## Risks

- 该收口比旧实现更严格，若未来 trace 证明同 episode 之外仍需要有限复用窗口，需要重新设计“episode 证据生命周期”，而不是重新引入 epoch delta grace。
- 目前只收敛了 owner / coordinator 这一层的 anchor 事实语义，后续 trace 验收仍要确认 Cloud/Home 的恢复收口没有被误伤。

## Follow-up

- 继续沿 `docs/rfcs/2026-03-27-timeline-value-runtime-upgrade.md` 做 M5/M6 的真实 trace 验收。
- 若后续发现 stale anchor 仍有诊断价值，再考虑增加“仅投影、不参与裁决”的历史视图字段。
