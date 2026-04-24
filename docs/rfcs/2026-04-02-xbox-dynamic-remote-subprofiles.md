# Xbox Dynamic Remote Subprofiles Stage 1 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 远端基线画像（HomeLan/Cloud/Relay）已统一，但运行态仍缺少可复用、可观测的子画像合同。
- 现有 runtime stats / diagnostics / trace / TS DTO 还不能直接表达“基线画像 + 动态子画像 + 有效画像标签”。

## Goal

- 建立共享动态子画像合同（第一阶段最小集合）。
- 在 engine 侧基于既有 runtime 信号完成统一分类 helper，不新增底层采集。
- 将分类结果接入 runtime stats/diagnostics/trace/TS DTO，供外部直接消费。
- 本阶段不改变 recovery/BWE/NACK 的动作策略。

## Scope

- In scope:
  - `crates/xbxengine/protocol/src/remote_profile.rs`：扩展动态子画像合同与 `effective label` helper。
  - `crates/xbxengine/core/src/transport/rtc/recovery/remote_profile_runtime.rs`：新增运行态分类 helper。
  - `crates/xbxengine/core/src/diagnostics/stats.rs`：将 baseline/dynamic/effective 接入 `XbxEngineStatsDto`。
  - `crates/xbxengine/protocol/src/runtime.rs`、`src-tauri/src/mods/xbxengine/trace_projection.rs`、`src/shared/rpc/xbxengine.ts`、`src/streaming/runtime/xbxengine-runtime.ts`、`src/streaming/types.ts`、`src/player/domain/media.ts`：透出新字段。
- Out of scope:
  - recovery coordinator、BWE policy、NACK admission 的行为策略调整。
  - 新增底层采集信号或替换现有事实源。

## Plan

1. 扩展共享画像合同，新增动态子画像与 effective label helper。
2. 新增 engine 统一分类 helper，消费现有 runtime 信号。
3. 打通 diagnostics/trace/TS DTO 透传链路并补充验证。

## Validation

- [x] `cargo test -p xbxengine-protocol -- --nocapture`
- [x] `cargo test -p xbxengine diagnostics::stats -- --nocapture`
- [x] `cargo test -p xbxengine remote_profile_runtime -- --nocapture`
- [x] `cargo test -p xbxrc trace_projection -- --nocapture --test-threads=1`
- [x] `pnpm -s exec tsc --noEmit`

## Risks

- 第一阶段的动态子画像阈值（如 `cloudHighRtt`、freshness window）是守旧初版，后续可能需基于真实 trace 做参数收敛。
- 当前仅做观测透传，尚未驱动策略动作，收益主要体现在可观测性与后续策略对齐前置准备。

## Progress

- [x] Step 1: 共享合同落地（`Steady/CloudStartup/CloudHighRtt/DecoderConstrained/DisplayConstrained`）。
- [x] Step 2: engine 统一分类 helper 已落地并带单测。
- [x] Step 3: runtime stats/diagnostics/trace/TS DTO 已全链路透出并通过验证。

## Execution Notes

- Date: 2026-04-02 | Status: completed
- Update: 完成动态子画像第一阶段，新增共享合同、runtime 分类 helper、diagnostics/trace/TS DTO 透传字段。
- Decision: 本轮严格只做“合同 + 分类 + 观测透传”，不改变 recovery/BWE/NACK 动作策略。
- Risk/Blocker: 无阻塞；残余风险主要是阈值参数需后续基于真实 trace 继续收敛。
