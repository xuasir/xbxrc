# runtime-trace-1775101572131 两个问题修复 Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-02-runtime-trace-1775101572131-two-issue-fix.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-02-runtime-trace-1775101572131-two-issue-fix.md)
- 已完成 `runtime-trace-1775101572131.jsonl` 暴露的两个问题修复：cloud 首窗过早 `failed-terminal`，以及 Connected 后恢复链与 `noPendingFrame` 压力窗口耦合过紧。

## Delivered

- 收口 cloud 首帧前的 no-progress / reconnect 终态判定，避免 `Provisioned` 附近被过早判死。
- 收口 Connected 后的 `transportAwaitRecoveryAnchor` 恢复阶段升级与硬 fallback 证据链，避免恢复链悬挂过久。
- 补齐 session policy 与 recovery coordinator 的回归测试，并完成 `cargo check -p xbxengine`。

## Changes

- 在 [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 中补齐 cloud warmup / pre-first-frame 的 reconnect 节流与 terminal soft-hold，增加 cloud 专用 fallback window 与 attempt limit，并引入 Connected 渲染 stall 的独立 liveness 判定。
- 在 [`crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 中引入 transport-await 的分段升级窗口、decoder reset 证据门槛，以及更严格的 hard fallback 收口。
- 保留现有 RTC 分层与命令执行路径，没有新增旁路恢复链；相关测试已覆盖 cloud New / Connecting / warmup / transport-await / hard fallback 场景。

## Validation

- `cargo test -p xbxengine cloud_ -- --nocapture`
- `cargo test -p xbxengine transport_await_ -- --nocapture`
- `cargo check -p xbxengine`

## Risks

- 真实云端链路仍需继续观察是否会出现新的首窗无进展边界。
- Connected 后的恢复链仍依赖 `presentAge` / `noPendingPressure` 等信号质量，若上游观测缺失，判定会偏保守。

## Follow-up

- 后续若再遇到类似 trace，优先复核 `session/policy.rs` 中的 cloud fallback / attempt limit 语义是否仍与实际链路匹配。
- 若 Connected 后仍出现 `noPendingFrame` 长窗，再用新 trace 评估是否需要补充更细的 supply 收口，但保持现有恢复分层不变。
