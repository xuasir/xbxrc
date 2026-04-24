# Home Runtime ICE Candidate Selection And Timeout Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-03-23-home-runtime-ice-candidate-selection-and-timeout.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-23-home-runtime-ice-candidate-selection-and-timeout.md)
- 已修正 home 串流 runtime 阶段的本地 ICE candidate 选择与 exchange timeout 下限。

## Delivered

- 过滤 ULA / link-local IPv6 advertised candidate。
- 调整 advertised IP 选择，避免默认路由 IPv6 抢过更优的私网 IPv4。
- 将 ICE exchange timeout 提高到带 5s 下限的有界窗口。

## Changes

- `/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/connection/io_runtime.rs` 不再把 `fdfe:*` 这类 ULA 地址当作可广播候选。
- `/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/runtime/lifecycle.rs` 不再直接拿 `first_frame_grace_ms` 作为 ICE exchange timeout。
- 增加 3 个定向单测，覆盖 IPv6 过滤、IPv4 优先级、ICE timeout floor。

## Validation

- `cargo fmt -p xbxengine`
- `cargo test -p xbxengine advertised_ip_priority_rejects_unique_local_ipv6 -- --nocapture`
- `cargo test -p xbxengine choose_preferred_advertised_ip_prefers_private_ipv4_over_global_ipv6 -- --nocapture`
- `cargo test -p xbxengine ice_exchange_timeout_uses_bounded_floor -- --nocapture`
- `cargo check -p xbxrc`

## Risks

- 如果远端要求额外 candidate 派生或 relay 路径，这次修正可能只能提高成功率，不能覆盖全部失败。
- 现有 trace 仍缺少更结构化的本地/远端 candidate 摘要。

## Follow-up

- 用新版本复现，确认本地提交的 candidate 是否已不再是 `fdfe:*` 单一路径。
- 若仍失败，下一步转查 candidate 派生补偿或 TURN/Teredo 对齐。
