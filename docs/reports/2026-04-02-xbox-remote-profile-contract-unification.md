# Xbox 远端画像合同统一 Report

> 说明：本 Report 仅用于复杂任务完全完成后的最终总结，不用于记录执行中的阶段性进度；中间过程应持续回写到对应 RFC。

## Summary

- Related RFC: [`docs/rfcs/2026-04-02-xbox-remote-profile-contract-unification.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-02-xbox-remote-profile-contract-unification.md)
- 本任务已完成两段收口：先把 `HomeLanGaming / CloudGaming / RelayGaming` 基线画像从分散定义统一为共享合同，再把运行期画像事实统一沉淀到 `runtime stats` 并作为 diagnostics 主读取来源。

## Delivered

- 在 `xbxengine-protocol` 新增共享画像类型 `XbxEngineRemoteProfileKindDto`。
- 落地统一解析规则：`session_target_type` 为主语义，`transport_path` 仅细分 Home 为 LAN/Relay。
- 接线完成：`xbox-streaming runtime compiler` 与 `xbxengine ScenarioPolicyResolver` 改为复用共享画像合同。
- `runtime stats` 现正式持有远端画像事实：`baseline_remote_profile`、`dynamic_remote_subprofile`、`effective_remote_profile_label`。
- 画像事实写回点统一到 `session policy` 周期入口（`evaluate_scheduling_owner()`），避免多处分散更新。
- `diagnostics/stats.rs` 改为优先读取 runtime facts；仅在字段缺失时安全回退到 classify helper，兼容旧测试/旧快照。

## Changes

- 新增 [`crates/xbxengine/protocol/src/remote_profile.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/protocol/src/remote_profile.rs) 并导出到 [`crates/xbxengine/protocol/src/lib.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/protocol/src/lib.rs)。
- `xbox-streaming` 增加对 `xbxengine-protocol` 依赖，`runtime compiler` 改为通过共享画像解析 `Target` 基线，再驱动 video pipeline 分档，见 [`crates/xbox-streaming/Cargo.toml`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/Cargo.toml) 与 [`crates/xbox-streaming/src/policy/runtime/compiler.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-streaming/src/policy/runtime/compiler.rs)。
- `xbxengine` 侧 `ScenarioPolicyResolver::resolve_kind()` 改为直接调用共享解析规则，不再维护重复 `is_relay_path` helper，见 [`crates/xbxengine/core/src/transport/rtc/recovery/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/policy.rs)。
- `runtime stats` 结构新增三字段，见 [`crates/xbxengine/core/src/api/backend.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/backend.rs)。
- 新增 runtime facts 写回 helper，见 [`crates/xbxengine/core/src/transport/rtc/recovery/remote_profile_runtime.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/remote_profile_runtime.rs)。
- 在 `evaluate_scheduling_owner()` 周期入口单点写回，见 [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs)。
- diagnostics 改为优先读 runtime facts 并保留 fallback，见 [`crates/xbxengine/core/src/diagnostics/stats.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/diagnostics/stats.rs)。

## Validation

- `cargo test -p xbxengine-protocol -- --nocapture`
- `cargo test -p xbox-streaming --lib runtime::compiler -- --nocapture`
- `cargo check -p xbxengine -p xbox-streaming -p xbxengine-protocol`
- `cargo test -p xbxengine transport::rtc::session::policy::tests::owner_contract_is_persisted_to_runtime_stats -- --nocapture`
- `cargo test -p xbxengine diagnostics::stats::tests::stats_prioritize_runtime_remote_profile_facts_when_present -- --nocapture`
- `cargo check -p xbxengine`

## Risks

- 编译期仍拿不到最终 `transport_path`，因此 `runtime compiler` 侧 Home 只能映射到 `HomeLanGaming` 基线；运行期 relay 细分仍在 engine 侧完成。
- 动态子画像（startup/high-rtt/decoder/display constrained）尚未进入共享合同，需后续单独任务推进。
- `diagnostics` 当前保留 classify fallback 是为了兼容旧测试/旧快照；若后续要强化合同一致性，可在全链路稳定后移除 fallback 并补齐严格断言。

## Follow-up

- 在不改变本轮行为的前提下，把 runtime stats / trace 增加显式 `remote_profile_kind` 字段，减少调试时对多字段反推画像。
- 设计第二阶段画像合同（动态子画像）前，先定义“输入信号来源清单 + 回归门禁”，避免合同扩展与策略重调耦合。
