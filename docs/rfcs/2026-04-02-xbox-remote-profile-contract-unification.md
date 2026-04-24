# Xbox 远端画像合同统一 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 当前 `xbxrc` 已经在多处按 `Home / Cloud / Relay` 分档，但画像定义分散在 `xbox-streaming runtime compiler` 与 `xbxengine ScenarioPolicyResolver` 两侧，尚未形成共享合同。
- 这种分散让“画像驱动”只能停留在概念层：同一个 `homeLanGaming / cloudGaming / relayGaming` 名义并没有单一来源，后续继续扩展 startup / 高 RTT / 解码/显示受限画像时容易再次分叉。

## Goal

- 把最小可落地的 Xbox 远端画像基线先统一为共享合同，当前只覆盖 `HomeLanGaming / CloudGaming / RelayGaming`。
- 让 `runtime compiler` 与 `xbxengine recovery/bwe policy` 从同一份画像定义与解析规则出发，保持现有行为不回退。

## Scope

- In scope:
  - `crates/xbxengine/protocol/src/*`
  - `crates/xbox-streaming/src/policy/runtime/compiler.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/policy.rs`
  - `crates/xbxengine/core/src/api/backend.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/remote_profile_runtime.rs`
  - `crates/xbxengine/core/src/transport/rtc/session/policy.rs`
  - `crates/xbxengine/core/src/diagnostics/stats.rs`
- Out of scope:
  - startup / high RTT / decoder constrained / display constrained 等动态子画像
  - recovery 行为、BWE 阈值、平台低延迟支撑的策略重调
  - trace/UI 面板新增画像字段

## Plan

1. 在共享层新增统一画像类型与解析规则，明确 `session_target_type` 为主语义、`transport_path` 仅细分 Home。
2. 让 `runtime compiler` 改为消费共享画像基线，而不是只靠本地 `Target` 分叉。
3. 让 `ScenarioPolicyResolver` 改为复用共享画像解析规则，消除本地重复定义。
4. 在 runtime stats 收口远端画像事实写回（`baseline_remote_profile` / `dynamic_remote_subprofile` / `effective_remote_profile_label`），并让 diagnostics 优先消费 runtime facts，保留安全 fallback 兼容旧测试快照。

## Validation

- [x] `cargo test -p xbxengine-protocol -- --nocapture`
- [x] `cargo test -p xbox-streaming --lib runtime::compiler -- --nocapture`
- [x] `cargo check -p xbxengine -p xbox-streaming -p xbxengine-protocol`
- [x] `cargo test -p xbxengine transport::rtc::session::policy::tests::owner_contract_is_persisted_to_runtime_stats -- --nocapture`
- [x] `cargo test -p xbxengine diagnostics::stats::tests::stats_prioritize_runtime_remote_profile_facts_when_present -- --nocapture`
- [x] `cargo check -p xbxengine`

## Risks

- 编译期 `runtime compiler` 仍拿不到最终 `transport_path`，因此 Home 只能先默认映射到 `HomeLanGaming` 基线；这属于当前事实边界，不应被误解成“编译期已经知道 relay”。
- 如果只统一类型名、不统一解析规则来源，后续仍可能在 engine 侧再次出现局部 helper 漂移。

## Progress

- [x] Step 1: 已确认画像统一的第一步应只覆盖 `HomeLanGaming / CloudGaming / RelayGaming`
- [x] Step 2: 已在 `xbxengine-protocol` 落地共享画像类型 `XbxEngineRemoteProfileKindDto` 与统一解析规则
- [x] Step 3: 已接入 `runtime compiler` 与 `ScenarioPolicyResolver`，两侧改为复用同一份基线画像语义
- [x] Step 4: 已在 session policy 周期入口单点写回 runtime profile facts，并让 diagnostics 默认读取 runtime stats 画像字段

## Execution Notes

- Date: 2026-04-02 | Status: completed
- Update: 本轮完成“画像合同统一”第一阶段：`runtime compiler` 与 `xbxengine recovery/bwe policy` 已从共享层复用同一基线画像解析规则。
- Decision: 编译期 `Home` 默认落到 `HomeLanGaming` 基线；实际 relay 仍由运行期 `transport_path` 在 engine 侧细化。
- Update: 新增验证覆盖 `runtime compiler` 对共享画像基线映射的单测，确保编译期场景分档不再依赖本地重复定义。
- Risk/Blocker: 动态子画像（startup/high-rtt/decoder/display constrained）尚未纳入共享合同，这部分需要后续单独 RFC，避免把合同统一与策略重调耦合在同一轮里。
- Date: 2026-04-02 | Status: completed
- Update: 本轮完成“观测事实收口”：`runtime stats` 正式持有 `baseline_remote_profile`、`dynamic_remote_subprofile`、`effective_remote_profile_label`，并在 `evaluate_scheduling_owner()` 的周期写回点单点更新，避免多处分散写入。
- Decision: `diagnostics/stats.rs` 读取优先级调整为“先读 runtime stats，缺失时回退 classify helper”，回退仅用于兼容旧测试/旧快照，不改变运行期策略。
- Date: 2026-04-02 | Status: completed
- Update: 本轮完成“策略入口优先消费 runtime facts”最小闭环：`recovery/runtime_state` 主线（`resolve_recovery_profile`、`current_profile_name` 及其直接依赖）与 `BWE policy` 主线（`resolve_transport_policy_profile_kind`、`classify_scenario_bitrate_band`、`resolve_target_remb_kbps`）已优先读取 `baseline_remote_profile`，缺失时回退 `session_target_type + transport_path`。
- Decision: 统一复用 `remote_profile_runtime::resolve_runtime_baseline_profile_kind` 做 runtime baseline 解析，避免各策略入口重复解析字符串；本轮不改阈值、不改恢复动作、不改动态子画像判定逻辑。
- Validation: `cargo test -p xbxengine profile_kind_prefers_runtime_baseline_remote_profile -- --nocapture`、`cargo test -p xbxengine owner_contract_is_persisted_to_runtime_stats -- --nocapture`、`cargo check -p xbxengine`。
