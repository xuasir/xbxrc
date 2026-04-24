# Cloud 供给压力 Hysteresis Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-03-29-cloud-supply-pressure-hysteresis.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-29-cloud-supply-pressure-hysteresis.md)
- 本次已完成 Cloud 恢复收口的 owner 侧 hysteresis 与 candidate ledger fallback，避免 clean anchor 后的短暂 gap reentry 继续把系统打回恢复态。

## Delivered

- `VideoSchedulingOwner` 现在会把当前 epoch 的 `SubmittedCleanAnchor` anchor candidate 视作 clean-anchor fact 的 fallback。
- `VideoSchedulingOwner` 对 `gap-reorder-pending` / `gap-resolved` 这类 clean anchor 后的短暂 reentry 增加了一次 hysteresis，不再因为单个新 gap 立即打碎恢复收口。
- `RtcSessionPolicy` 已把 `latest_anchor_candidate_ledger` 透传给 owner 输入，保证 runtime stats 里可用的 anchor candidate 能参与回稳判定。

## Changes

- [`crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs)
  - 新增 `latest_anchor_candidate_ledger` 输入。
  - 新增 clean-anchor 事实 fallback 与 clean-anchor reentry hysteresis。
  - 新增回归测试覆盖 clean anchor 后的 gap reentry。
- [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs)
  - 将 runtime stats 中的 `latest_anchor_candidate_ledger` 传入 owner 输入。
- [`docs/rfcs/2026-03-29-cloud-supply-pressure-hysteresis.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-29-cloud-supply-pressure-hysteresis.md)
  - 更新为最终完成态，并记录 clean-anchor hysteresis 的最终决策。
- [`docs/project-task.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/project-task.md)
  - 将对应任务标记为 Done。

## Validation

- `cargo test -p xbxengine transport::rtc::policy::video_scheduling_owner -- --nocapture`
- `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
- `cargo fmt`
- `git diff --check`

## Risks

- clean anchor 后的 reentry hysteresis 过宽，可能会把真正的 reference/keyframe 断裂放软过头。
- 这个收口只覆盖 owner 回稳与短暂 reentry，不等于彻底消除所有复杂波动下的视频坏帧。

## Follow-up

- 继续回放最新 Cloud trace，确认 `stable-serving` 能在 clean anchor 后站住。
- 若后续仍出现同一 gap 的重复 re-break，再考虑把 hysteresis 前移到 timeline 或 nack 层做更细的门禁。
