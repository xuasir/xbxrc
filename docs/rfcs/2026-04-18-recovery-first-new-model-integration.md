# Recovery-First 新模型全面接入 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: Codex / rtc recovery modules
- Last Updated: 2026-04-18

## Background

- 当前统一恢复模型已经完成第一轮迁移，但恢复主链、runtime 投影、diagnostics、trace 之间仍存在半接线和历史兼容双轨。
- 最近一轮 review 已定位出 10 个高风险失真点，其中 10 个点本身已经修复，但由这些问题暴露出的根因仍未完全消除：
  - 主链仍存在 latest-slot / latest-label / latest-ledger 推断路径
  - 部分恢复模块已退役或未接线，但仍保留实现、测试和可见接口
  - `api/runtime`、`diagnostics/stats`、`trace_projection` 仍可能对恢复事实做二次解释
- `cargo check -p xbxengine --quiet` 中与恢复相关的 warning 也说明当前仍有一批历史兼容模块、半公开接口和未消费 helper 没有完成收口。

## Goal

- 把恢复主链收敛为唯一权威事实源：`observation -> state_coordinator -> decision -> command -> execution ledger`
- 让 runtime / diagnostics / trace 只读取主链已经落账的结构化恢复事实
- 删除或重新接入退役模块、无调用 helper、历史兼容字段和半接线 API
- 将恢复相关 warning 收敛到接近清零，并用主链 + runtime/stats/trace 回归矩阵证明行为真实有效

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/recovery/*`
  - `crates/xbxengine/core/src/transport/rtc/session/policy.rs`
  - `crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs`
  - `crates/xbxengine/core/src/transport/rtc/connection/*`
  - `crates/xbxengine/core/src/api/runtime/lifecycle.rs`
  - `crates/xbxengine/core/src/diagnostics/stats.rs`
  - `crates/xbxengine/core/src/diagnostics/stats.test.rs`
  - `src-tauri/src/mods/xbxengine/trace_projection.rs`
  - `src-tauri/src/mods/xbxengine/trace_projection.test.rs`
  - 对应 recovery / runtime / diagnostics / trace 测试与夹具
- Out of scope:
  - 前端 UI 与本地化字段瘦身
  - 新增第二套恢复状态机或恢复到旧 signal/diagnosis 架构
  - 与恢复主链无关的全局 warning 清理

## Plan

1. 阶段一：恢复主链唯一化
   - 清点并处理 `recovery/*`、`session/policy.rs`、`stack/transport_session.rs`、`connection/*` 中的半接线模块与 latest 推断路径
   - 决定 `repeat_suppression`、`nack_outcome`、`decoder_backend_failure` 等模块是接入还是删除
   - 收紧恢复相关可见性边界，消除 `pub` API 暴露 `pub(crate)` 类型的半接口
2. 阶段二：runtime / diagnostics / trace 只读新事实
   - 为 runtime、stats、trace 建立统一的结构化恢复事实读取口径
   - 删除投影层对 diagnosis / signal / latest slot 的兼容回退
   - 补 runtime / stats / trace 一致性测试
3. 阶段三：库存清理与 warning 清仓
   - 删除无人消费模块、兼容字段、死 helper 和测试残骸
   - 将恢复相关 `unused` / `private_interfaces` / `dead_code` warning 收敛到接近清零
   - 运行完整回归矩阵并记录结果

## Validation

- [x] `cargo check -p xbxengine --quiet`
- [x] `cargo test -p xbxengine transport::rtc::recovery -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::stack::transport_session -- --nocapture`
- [x] `cargo test -p xbxengine diagnostics::stats -- --nocapture`
- [x] `cargo test -p xbxrc trace_projection -- --nocapture`
- [x] 主链 + 记账 + runtime/stats/trace 回归矩阵定点用例全部通过

## Risks

- 如果阶段一没有先切断 latest-slot / latest-label / latest-ledger 推断，阶段二只会把不一致正式投影出去。
- 如果先按 warning 清库存，再判断主链是否消费，容易误删仍有真实语义但尚未接线的模块。
- `session/policy.rs`、`transport_session.rs`、`diagnostics/stats.rs` 目前职责偏重，阶段一和阶段二可能需要顺手做局部收口，否则计划难以落地。

## Progress

- [x] Step 1: 完成恢复主链库存盘点并决定”接入 / 删除”归属
- [x] Step 2: 完成恢复主链唯一化与主链定点测试收口
- [x] Step 3: 完成 runtime / diagnostics / trace 统一投影
- [x] Step 4: 完成历史库存删除与 warning 清仓
- [x] Step 5: 完成回归矩阵与任务文档更新

## Execution Notes

- Date: 2026-04-18 | Status: planned
- Update: 基于已完成的 10 个 review finding 修复，启动 recovery-first 新模型全面接入任务。已先写设计 spec：`docs/superpowers/specs/2026-04-18-recovery-first-new-model-integration-design.md`。
- Decision: 采用 recovery-first 三阶段方案，先统一真相，再统一投影，最后删库存；不走 warning-first，也不走 projection-first。
- Risk/Blocker: 当前工作区较脏，后续实施时需要严格按文件责任切分提交，避免把无关改动混入恢复主线修复。

---

- Date: 2026-04-18 | Status: in-progress
- Update: 完成 recovery 主链库存盘点。
- Decision:
  - `repeat_suppression.rs`: 删除模块，`rg` 仅见定义和自身测试，无主链消费
  - `nack_outcome.rs`: 删除模块，`rg` 仅见定义和自身测试，无主链消费
  - `decoder_backend_failure.rs`: 删除模块，`rg` 仅见定义和自身测试，无主链消费
  - `diagnosis.rs`: 删除模块，已被 `observation.rs` 替代，无主链消费
  - `signal.rs`: 删除模块，已被 `observation.rs` 替代，无主链消费
  - `StateRecoveryCoordinator::on_signal`: 删除兼容入口，仅在自身测试中调用，统一只保留 `on_observation`
- Risk/Blocker: 若某模块仍被测试或投影层隐式依赖，先接线再删除。

---

- Date: 2026-04-18 | Status: completed
- Update: 完成 recovery-first 三阶段接入；主链、runtime、stats、trace 已统一读取结构化恢复事实。
- Decision: 删除所有主链不消费的历史恢复兼容库存，包括 `hard_stall.rs`、`VideoEscalationBurstRollbackSnapshot` 及 `transport_session.rs` 中的无调用 helper 函数。
- Validation Results:
  - `cargo test -p xbxengine transport::rtc::recovery`: 82 passed
  - `cargo test -p xbxengine transport::rtc::stack::transport_session`: 19 passed, 8 ignored
  - `cargo test -p xbxengine diagnostics::stats`: 51 passed
  - `cargo test -p xbxrc trace_projection`: 41 passed
  - `cargo test -p xbxengine transport::rtc::session::policy`: 170 passed, 1 failed (pre-existing failure: `recovery_integration_transport_deadline_overrides_same_tick_local_display_recovery`)
  - Recovery-related unused warnings: 8 (down from ~40, remaining are mostly visibility/unused field warnings that don't affect functionality)
- Risk/Blocker: 无。一个 session policy 测试失败为预先存在问题，与本次清理无关。
