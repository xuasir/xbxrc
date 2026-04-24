# Session Flow Canonicalization RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 当前 session 相关逻辑横跨 `src/streaming`、`src-tauri/src/mods/streaming`、`crates/xbox-streaming`、`src-tauri/src/mods/xbxengine` 四层。
- 最近多轮 home ready / session ready / bounded retry 修复后，行为已经逐步稳定，但“谁拥有哪段状态机”没有被集中写清楚，继续迭代时很容易在 UI、Tauri adapter 和 domain flow 之间重复塞策略。
- 需要一份当前代码库可直接对照的 canonical session flow 文档，防止后续继续把局部补丁堆成新的腐化点。

## Goal

- 固定当前 session flow 的 canonical 链路、阶段边界和 owner。
- 明确 home / cloud 共用主链和 home 特有分叉。
- 写清楚反腐规则：哪些层只做投影/桥接，哪些层才允许承载策略。

## Scope

- In scope:
  - `docs/stream-session-flow.md`
  - 本轮对现有 session 架构的梳理结论
  - 任务追踪与总结文档
- Out of scope:
  - 代码重构
  - runtime / WebRTC 细节重写
  - 页面交互或视觉调整

## Plan

1. 收拢 session 启动、轮询、runtime 握手和关闭链路。
2. 归纳当前最容易继续腐化的边界错位。
3. 产出 canonical 文档并补 tracking。

## Validation

- [x] 文档内容与当前代码入口一致
- [x] 覆盖 home / cloud / runtime / close 四个主场景
- [x] 追踪文档已更新

## Risks

- 文档如果只描述理想态而不对齐当前代码，会继续制造“文档说一套、实现跑一套”的漂移。
- 如果不把“禁止再往哪层塞策略”写清楚，梳理本身也无法阻止后续腐化。

## Progress

- [x] Step 1: 已收拢 `useStreamExecution`、`StreamingService`、`SessionFlowService`、`SessionScheduler` 和 `TauriXbxEngineHostBridge` 的真实入口。
- [x] Step 2: 已归纳主要腐化风险：UI 重复状态机、Tauri adapter 承担策略、字符串驱动错误语义、runtime 握手边界扩散。
- [x] Step 3: 已补 canonical 文档、Report 与任务追踪。

## Execution Notes

- Date: 2026-03-24 | Status: completed
- Update: 本轮不继续加实现逻辑，而是把当前 session flow 固化成文档和边界约束，后续修复应先回到这份文档。
- Decision: 以 `crates/xbox-streaming` 为 session 编排 owner，以 `src-tauri/src/mods/streaming/service.rs` 为 adapter/投影 owner，以 `src/streaming/useStreamExecution.ts` 为页面编排 owner。
- Risk/Blocker: 当前仓库仍存在历史遗留 TS 类型错误，本轮只做文档梳理，不扩修无关问题。
