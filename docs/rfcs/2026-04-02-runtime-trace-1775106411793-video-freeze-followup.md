# runtime-trace-1775106411793 视频卡死 follow-up RFC

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 新 trace `runtime-trace-1775106411793.jsonl` 显示视频在 `remoteTrackAttached` 后曾成功拉起，但随后进入 `waitKeyframe` / `displaySupplyCritical` / `noPendingFrame` 循环。
- 现有恢复收口已经降低了过早 reconnect / failed-terminal 的误判，但新的回归表明：owner 侧把“当前明确在等 keyframe”的场景压成了纯 `SupplyStarved`，导致恢复链继续停留在 `cooldownSuppressed`。

## Goal

- 让 owner / recovery 链路能区分：
  - 正常的 clean anchor 回稳
  - 当前明确进入 `waitKeyframe` 或 `frame-inspection-rejected-await-anchor` 的恢复噪声
- 在 `critical noPending` 且当前 timeline 明确在等 keyframe 时，避免继续压成普通 supply starved，应该让恢复链重新朝 `RebuildingSupply` / keyframe recovery 推进。
- 保持 clean anchor + fresh supply 的正常回稳路径不回退。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`
  - 相关单测
  - 必要的 tracker / report 更新
- Out of scope:
  - 重新设计整个 RTC 恢复主链
  - 改动连接建立、TWCC warmup 或 BWE 语义

## Plan

1. 调整 owner 对 clean anchor 与当前 recovery noise 的判定边界。
2. 补充/修正回归测试，覆盖 critical supply + waitKeyframe 的反例。
3. 验证 trace 行为是否从 `SupplyStarved/displaySupplyCritical` 收敛回 `RebuildingSupply/transportAwaitRecoveryKeyframe`。
4. 更新 `docs/project-task.md` 与最终说明。

## Validation

- [x] `cargo test -p xbxengine critical_wait_keyframe_noise_prefers_rebuilding_over_supply_starved_even_with_clean_anchor -- --nocapture`
- [x] `cargo test -p xbxengine connected_lingering_no_pending_with_clean_anchor_can_return_to_stable_serving -- --nocapture`
- [x] `cargo check -p xbxengine`
- [x] `cargo test -p xbxengine transport_await_ -- --nocapture`

## Risks

- `clean anchor` 现在只清外显计时、不解武装内部 hard-fallback 起点，短暂健康帧不会把坏窗从头打散，但同一恢复 epoch 内后续坏窗会更快进入 reconnect 候选。
- 如果后续还要进一步压抖动链路，需要再给“持续健康”补一层更明确的 disarm 条件，而不是退回到直接清零计时。

## Progress

- [x] Step 1: owner 判定边界已调整，`critical + waitKeyframe` 会回到 `RebuildingSupply`
- [x] Step 2: owner 回归测试已补齐并通过
- [x] Step 3: coordinator hard-fallback 语义已收口，`transport_await_` 全量验证通过

## Execution Notes

- Date: 2026-04-02 | Status: completed
- Update: 已在 owner 状态机里把 `critical noPending + waitKeyframe` 从纯 `SupplyStarved` 拉回 `RebuildingSupply`，并在 coordinator 里把 hard-fallback 语义改成“清外显计时但保留内部起点”，避免短暂健康帧把坏窗彻底打散。
- Decision: 对 explicit waitKeyframe 噪声优先回到 anchor recovery 主链；对 `clean anchor` 只做可见状态收口，不把已在运行的 hard-fallback 计时完全解武装。
- Risk/Blocker: 无已知阻塞；如后续链路仍明显抖动，再考虑给“持续健康”增加更严格的 disarm 条件。
