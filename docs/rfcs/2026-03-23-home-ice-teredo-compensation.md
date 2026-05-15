# Home ICE Teredo Compensation RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- `参比实现` 在拉取远端 ICE candidates 后，会对 Teredo IPv6 (`2001:0::/32`) 额外派生 IPv4 host candidates。
- 我们当前只做候选清洗和排序，没有这层补偿。
- 新日志已经从 session 层卡点转到 runtime/ICE，补偿逻辑值得单独补齐。

## Goal

- 在当前候选归一化链中补最小 Teredo/IPv4 compensation。
- 不扩散到 TURN/Teredo 大改或完整 ICE 主线重写。

## Scope

- In scope:
  - `crates/xbox-streaming/src/session/signaling/ice.rs`
  - 远端 ICE candidate 归一化与补偿
  - 定向单测与编译验证
- Out of scope:
  - session 启动链
  - runtime/recovery 主链
  - TURN/Teredo 网络栈改造

## Plan

1. 提取 `参比实现` 的补偿模式。
2. 在当前归一化链加入 Teredo IPv4 派生。
3. 补单测并验证编译。

## Validation

- [x] `cargo fmt -p xbox-streaming`
- [x] `cargo test -p xbox-streaming normalize_adds_teredo_derived_ipv4_host_candidates -- --nocapture`
- [x] `cargo check -p xbxrc`

## Risks

- 如果失败主因在本地 candidate 生成或 relay 路径，单补远端 Teredo compensation 只能提升成功率，不能覆盖全部失败。
- 当前只补最小派生，没有完全重建 foundation/priority 排序链。

## Progress

- [x] Step 1: 已确认 `参比实现` 会把 Teredo IPv6 派生成 `client4:9002` 和 `client4:teredoPort` 两个 host candidates。
- [x] Step 2: 已在 `IcePolicy::normalize()` 中加入 Teredo IPv4 补偿。
- [x] Step 3: 已补单测并完成编译验证。

## Execution Notes

- Date: 2026-03-23 | Status: completed
- Update: 在 `session/signaling/ice.rs` 中，对 `2001:0::/32` Teredo candidate 额外派生 IPv4 host candidates，并沿现有归一化链输出。
- Decision: 先补最小 compensation，不对全部 candidate 重写 foundation/priority。
- Risk/Blocker: 若后续 trace 仍卡在 `Connecting`，下一步继续下探 candidate 派生排序或 TURN/relay 使用情况。
