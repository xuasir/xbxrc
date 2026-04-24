# Home Runtime ICE Candidate Selection And Timeout RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 最新 home 串流日志已不再卡在 `Provisioning`，而是进入 runtime 后停在 `Connecting / ExchangingIce`。
- 日志显示本地只向远端提交了 `fdfe:dcba:9876::1` 这类 ULA IPv6 host candidate，随后远端 candidates 虽已返回并被应用，但 ICE exchange loop 在约 1.8s 内超时退出。

## Goal

- 避免 runtime 把 ULA / link-local IPv6 当作可广播候选。
- 让 ICE exchange loop 至少保留一个合理的 trickle 窗口，不再被首帧 grace 过早截断。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/connection/io_runtime.rs`
  - `crates/xbxengine/core/src/api/runtime/lifecycle.rs`
  - 定向单测与编译验证
- Out of scope:
  - session 层启动链
  - TURN/Teredo 补偿策略
  - 媒体解码/渲染链

## Plan

1. 修正 advertised IP 选择规则。
2. 放宽 ICE exchange timeout 下限。
3. 补单测并验证编译。

## Validation

- [x] `cargo fmt -p xbxengine`
- [x] `cargo test -p xbxengine advertised_ip_priority_rejects_unique_local_ipv6 -- --nocapture`
- [x] `cargo test -p xbxengine choose_preferred_advertised_ip_prefers_private_ipv4_over_global_ipv6 -- --nocapture`
- [x] `cargo test -p xbxengine ice_exchange_timeout_uses_bounded_floor -- --nocapture`
- [x] `cargo check -p xbxrc`

## Risks

- 如果远端确实依赖额外的 Teredo/relay 候选，仅靠本地 advertised IP 和 timeout 修正可能仍不够。
- 当前还没有把最终选中的 local candidate 结构化打到更高层 trace，后续复盘仍主要依赖 runtime log。

## Progress

- [x] Step 1: 已确认 `fdfe:*` ULA candidate 是当前最可疑的本地可达性问题。
- [x] Step 2: 已改为过滤 ULA/link-local IPv6，并给 ICE exchange 增加 5s 下限。
- [x] Step 3: 已补单测并完成编译验证。

## Execution Notes

- Date: 2026-03-23 | Status: completed
- Update: `io_runtime.rs` 现在不会把 ULA/link-local IPv6 作为 advertised candidate，并且优先保留更高优先级的私网 IPv4；`lifecycle.rs` 的 ICE exchange timeout 改为 `max(first_frame_grace_ms, reconnect_stall_ms, 5000ms)`。
- Decision: 先做最小 runtime/ICE 修正，不直接引入 Teredo/relay 派生逻辑。
- Risk/Blocker: 若复现后仍卡在 `Connecting`，下一步转查 candidate 派生补偿或 TURN/relay 使用率。
