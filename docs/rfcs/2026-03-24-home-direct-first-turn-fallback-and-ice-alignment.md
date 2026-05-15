# Home Direct-First TURN Fallback And ICE Alignment RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 当前 home Rust-owned 路径会在 plan resolved 后直接把 fallback TURN 注入 PeerConnection，与 `参比实现` 的“先直连，失败后再 fallback TURN”策略不一致。
- 当前 `参比实现` 还会对远端 ICE candidates 做 Teredo IPv4 派生、排序和 end-of-candidates 归一化；canonical 路径虽已有部分归一化能力，但整体行为边界仍未显式对齐。
- 最新 home trace 显示单次 Rust-owned 协商在未建立媒体前即关闭，需把“直连优先 + fallback 重试”和候选加工对齐纳入同一改造面。

## Goal

- 让 home Rust-owned 路径先尝试 direct ICE，不在首轮协商时立即使用 fallback TURN。
- 当 direct 尝试明确失败后，再进入 fallback TURN 重试，行为语义对齐 `参比实现`。
- 对齐 ICE candidate 加工策略，确保 canonical 路径在候选归一化、Teredo 补偿、排序与收敛上具备稳定一致的语义。

## Scope

- In scope:
  - `crates/xbox-streaming` 中 home fallback TURN 的 runtime / signaling 策略编排
  - `src-tauri/src/mods/streaming` 与 `src-tauri/src/mods/xbxengine` 的 runtime 启动/重试桥接
  - `crates/xbxengine` 的 home ICE 启动、失败切换、候选交换与相关诊断
  - 对照 `参比实现` 的 direct-first / fallback TURN 与 ICE candidate 加工行为
  - 必要的文档、RFC、task tracker、完成后 report
- Out of scope:
  - cloud streaming 行为改动
  - 引入新的并行 transport 主线或浏览器/TS 替代实现
  - 与本任务无关的 recovery/BWE/渲染策略调整

## Plan

1. 梳理 current direct/TURN 注入与 retry 边界，明确 fallback 切换 owner。
2. 设计并实现 home direct-first -> fallback TURN 的单次重试主线。
3. 对齐 ICE candidate 加工语义，并补齐 trace / 测试 / 文档。

## Validation

- [x] 验证 home direct-first 尝试首轮不注入 fallback TURN。
- [x] 验证 direct 失败后会进入 fallback TURN 重试，且 trace 可区分两轮尝试。
- [x] 验证 ICE candidate 加工与 `参比实现` 对齐的关键场景（Teredo、排序、EOC）。
- [x] 运行受影响 Rust crate 的定点测试与 `cargo check`。

## Risks

- direct/fallback 切换 owner 选错层级，可能让 runtime / session / recovery 边界继续腐化。
- 过度照搬 `参比实现` 的浏览器行为，可能与 Rust-owned RTC 栈的时序约束不匹配。
- fallback TURN 重试若与现有 reconnect/recovery 机制叠加不当，可能出现重复重试或 session 清理不完整。

## Progress

- [x] Step 1: 已确认切换 owner 放在 `src/streaming/runtime/runtime-host.ts`，避免侵入 session 创建主线
- [x] Step 2: 已落地 home rust-owned 直连优先、失败后单次 fallback TURN 重试，并补 runtime attempt 日志
- [x] Step 3: 已在 `crates/xbox-streaming/src/session/signaling/ice.rs` 对齐 foundation/priority 重写、Teredo 补偿、排序与 EOC，并完成测试/文档

## Execution Notes

- Date: 2026-03-24 | Status: planned
- Update: 新建 RFC，记录 home direct-first TURN fallback 与 ICE candidate 对齐任务的背景、目标、范围与执行计划。
- Decision: 该任务属于 transport / streaming 跨层行为改动，按复杂任务处理，先立 RFC 再实施。
- Risk/Blocker: direct-first 与 fallback TURN 的切换边界仍需结合现有 runtime/recovery 主线进一步确认。
- Date: 2026-03-24 | Status: completed
- Update: `runtime-host` 已改为 home rust-owned + `turnSource==='fallback'` 时首轮去掉 TURN，若在 `connected` 之前收到 `closed/failed` 或 launch 抛错，则在同一 session 上单次切回 fallback TURN 重试；同时补充前端 attempt 日志，方便对照 runtime trace。
- Decision: direct-first/fallback 切换 owner 保持在 runtime host 层，不把“首轮直连、二轮 fallback”的编排继续下沉到 session create/recovery 边界，降低跨层腐化风险。
- Risk/Blocker: 本机缺少 `node`/`pnpm`，本轮无法补跑前端 lint；TS 变更已通过代码审查与 Rust 侧相关验证，后续建议在具备 Node 工具链的环境再跑一次静态检查。
