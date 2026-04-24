# Render/Present Semantics Closure RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 2026-04-04 已完成 `latest_video_host_present_time_ms` 单一事实源收口，修复了恢复闭环里直接参与决策的 present freshness 双轨问题。
- 继续审查后发现，恢复输入、诊断/UI 投影、runtime event 合同与 native presenter 侧仍残留 `rendered / submit / present` 混用，存在再次长出第二套事实源的结构风险。

## Goal

- 彻底区分三类语义：`renderer rendered`、`host submit`、`host present`。
- 让恢复/owner/diagnostics/trace/runtime event 只在需要的地方消费对应层级事实，不再通过命名模糊字段混装回退。

## Scope

- In scope:
  - `crates/xbxengine/core/src/api/runtime/*`
  - `crates/xbxengine/core/src/session/recovery.rs`
  - `crates/xbxengine/core/src/diagnostics/stats.rs`
  - `crates/xbxengine/core/src/transport/rtc/**/*`
  - `crates/xbxengine/protocol/src/runtime.rs`
  - `src-tauri/src/mods/native_video/*`
  - `src-tauri/src/mods/xbxengine/{events,runtime_state,trace_projection}.rs`
- Out of scope:
  - 新增一套额外渲染管线
  - 重做 trace UI 或前端诊断面板的大规模产品改版

## Plan

1. 收紧恢复/诊断侧时间事实：去掉 `latest_frame_rendered_at_ms` 的混装回退，明确 rendered 与 host present 的职责边界。
2. 收紧 native presenter 与统计合同：把 submit/present 计数、事件与状态名改成不歧义的语义，并修正基于累计 submit 的可见输出推断。
3. 补齐测试与追踪：为恢复判据、diagnostics、trace 投影和 runtime event 添加回归覆盖。

## Validation

- [ ] `cargo fmt --all`
- [ ] 运行与 recovery / diagnostics / policy / trace 相关的定点测试
- [ ] `cargo check -p xbxengine -p xbxrc`

## Risks

- 现有前端或离线分析可能依赖旧的 runtime event / trace 字段名，需要同步调整或兼容。
- 恢复策略从“宽松回退”改为“单一事实源”后，若 host telemetry 有缺口，可能暴露新的未覆盖场景。

## Progress

- [x] Step 1: 完成问题归类与修复边界确认
- [x] Step 2: 完成 core/recovery/diagnostics/protocol 语义收口
- [x] Step 3: 完成 native_video/trace/runtime event 语义收口与验证

## Execution Notes

- Date: 2026-04-04 | Status: in-progress
- Update: 建立 RFC，确认本轮任务目标是彻底收口 rendered / submit / present 语义，不保留继续误导策略与观测的双义字段。
- Decision: 继续沿“单一事实源 + 分层命名”的方向推进；必要时直接重命名 runtime/protocol 字段，而不是再叠加解释注释。
- Risk/Blocker: 当前仓库存在大量并行改动，实施时必须限制写集并避免回退他人修改。
- Date: 2026-04-04 | Status: completed
- Update: 已完成恢复输入、diagnostics visible output、native presenter enqueue 命名、trace hostPresentState 与 xbxengine runtime `frameReady` 事件链收口。
- Decision: 纯内部 legacy `submit` 字段暂不在本轮全仓强制改名；外部合同、trace 命名与策略判据已全部切到无歧义语义。
- Risk/Blocker: 无新增 blocker；仓库现存 warnings 未在本任务处理。
