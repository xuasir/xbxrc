# Home ICE Teredo Compensation Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-03-23-home-ice-teredo-compensation.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-23-home-ice-teredo-compensation.md)
- 已补齐 `XStreamingDesktop` 参考实现里最关键的 Teredo IPv4 candidate compensation。

## Delivered

- 在 `IcePolicy::normalize()` 中增加 Teredo IPv6 到 IPv4 host candidate 的派生。
- 保留现有候选清洗、排序、补 `end-of-candidates` 的主链。
- 增加定向单测，覆盖 Teredo candidate 的 IPv4 派生。

## Changes

- `/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/src/session/signaling/ice.rs` 现在会从 `2001:0::/32` candidate 解析 `client4` 和 Teredo 映射 UDP port。
- 针对每个 Teredo candidate，额外输出 `client4:9002` 与 `client4:teredoPort` 两个 `typ host` candidates。
- 现有的 IPv6 偏好和类型优先级排序仍保持不变。

## Validation

- `cargo fmt -p xbox-streaming`
- `cargo test -p xbox-streaming normalize_adds_teredo_derived_ipv4_host_candidates -- --nocapture`
- `cargo check -p xbxrc`

## Risks

- 若真实瓶颈在本地 candidate 面或 relay 路径，这次改动不能单独保证建链成功。
- 当前没有把 compensation 前后的候选摘要结构化写入 trace。

## Follow-up

- 用新 trace 验证 `pollIceResult` 后归一化结果里是否已出现 Teredo 派生的 IPv4 host candidates。
- 若仍失败，继续对比 `XStreamingDesktop` 的候选重写排序与 foundation/priority 规则。
