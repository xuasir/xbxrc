# H264 Preset/Profile Semantics Closure RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 新 trace [`runtime-trace-1775297730377.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775297730377.jsonl) 暴露了启动期 `bootstrapMissingSps` 恢复窗口，提示当前 Rust-owned 启动 bootstrap 对首个 clean keyframe 质量高度敏感。
- 代码排查发现 H264 preset/profile 语义在不同链路上已经分叉：
  - 浏览器直出路径按 `64 > 4d > 42e > 420` 排序。
  - Rust-owned SDP 排序却按 `4d > 42e > 420 > 64`。
  - `H264High` 在 negotiation compiler 中还被编译成 `4d`，与 UI 文案和浏览器路径不一致。
- 这种分叉会让“高/Main 档”在运行时实际协商到不同 family，形成 preset 歧义，也放大启动期问题的排查成本。

## Goal

- 为 H264 preset 建立单一、可验证的 family 语义。
- 让 Rust-owned 与浏览器直出在 H264 family 排序上保持一致。
- 补齐回归测试，防止后续再次把 `H264High` 回退成 `4d`。

## Scope

- In scope:
  - `crates/xbox-streaming/src/policy/negotiation/compiler.rs`
  - `crates/xbox-streaming/src/policy/compiler.rs`
  - `crates/xbxengine/core/src/transport/rtc/sdp/policy.rs`
  - 必要的测试与任务跟踪文档
- Out of scope:
  - H264 bootstrap 状态机重写
  - VideoToolbox 解码链的更大规模重构
  - UI 新增 codec preset 或设置迁移

## Plan

1. 收口 preset 到 family 的 canonical 映射，明确 `H264High -> 64`、`H264Main -> 4d`。
2. 统一 Rust-owned SDP H264 排序逻辑，与浏览器路径保持相同优先级。
3. 补齐 Rust 侧回归测试并更新任务跟踪。

## Validation

- [ ] `cargo test -p xbox-streaming policy::compiler -- --nocapture`
- [ ] `cargo test -p xbxengine transport::rtc::sdp::policy -- --nocapture`
- [ ] `cargo check -p xbox-streaming -p xbxengine`

## Risks

- 某些历史会话若依赖“High 实际走 Main family”的旧行为，协商结果会发生变化。
- 该修复收口了 preset 歧义，但不单独保证所有启动期 `bootstrapMissingSps` 都会消失，仍需结合新 trace 继续验证。

## Progress

- [x] Step 1: 已确认 preset/profile 语义在 browser direct 与 Rust-owned 之间分叉。
- [x] Step 2: 已实现 canonical 映射与排序修复，并同步 browser/Rust 两条链。
- [x] Step 3: 已完成验证并更新跟踪。

## Execution Notes

- Date: 2026-04-04 | Status: in-progress
- Update: 建立本 RFC，收口本轮工作范围到 H264 preset/profile 语义统一。
- Decision: 本轮不扩散到更大的 bootstrap 状态机，只先修确定性的 preset 歧义与排序分叉。
- Risk/Blocker: 若真实问题还包含服务端首帧质量波动，需要在本轮完成后继续用新 trace 验证。

- Date: 2026-04-04 | Status: completed
- Update: 已将 `H264High` 的 compiler 语义收口为 `64` family；Rust-owned SDP 排序改为“命中偏好优先，family 等级其次”；browser 侧 `SdpManipulator` 同步改为同一排序规则，并补齐 Rust 回归测试。
- Decision: `H264High` 与 `H264Main` 只保留单一 canonical family 语义，不再用同一个 `4d` family 复用两档 preset。
- Risk/Blocker: 本轮未直接复现/重放新 trace，启动期 `bootstrapMissingSps` 是否完全消失仍需下一份 runtime trace 验证。
