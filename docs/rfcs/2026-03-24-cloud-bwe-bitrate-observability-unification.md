# 统一云游戏 BWE/码率观测口径并修复错误显示 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 云游戏链路中同名指标在不同层语义不一致（如 `inbound_bitrate_kbps` 与 `inbound_video_bitrate_kbps`），导致排障时口径混用。
- 当前存在“指标错误显示/误解读”问题：前端与 trace 消费链未统一按“总码率/视频码率/音频码率/BWE 目标/TWCC 观测”分层展示，影响诊断与策略验证。
- 近期已完成 BWE 主路径切换到配置驱动策略；若观测面不统一，仍会出现“策略已修复但可视化误判”的问题。

## Goal

- 建立云游戏 BWE/码率指标的单一语义口径，并在 runtime stats、trace、RPC DTO、前端展示中一致落地。
- 修复当前错误显示与误导字段，使 UI/trace 可以直接区分：
  - 总下行码率
  - 视频下行码率
  - 音频下行码率
  - BWE 目标与决策理由
  - TWCC 反馈可用性与质量
- 形成可回归验证的观测基线，支持后续网络/BWE 问题快速定位。

## Scope

- In scope:
  - Rust 端观测定义与投影统一（`runtime stats`、`trace_projection`、`diagnostics/stats`）。
  - `xbxengine`/`streaming` RPC DTO 字段语义校准与命名对齐（必要时新增明确字段，保留兼容层）。
  - Vue 前端展示与图表消费链修正，确保不再把音频码率误当总码率/视频码率。
  - 口径说明文档补充（字段解释、优先展示字段、排障读取顺序）。
- Out of scope:
  - BWE 算法策略本身的进一步调参（本任务只收敛观测与显示）。
  - 新增独立传输路径或替换现有 Tauri + Rust + Vue 架构。
  - 历史 trace 回填改写。

## Affected Modules

- Rust core:
  - `crates/xbxengine/core/src/api/backend.rs`
  - `crates/xbxengine/core/src/diagnostics/stats.rs`
  - `crates/xbxengine/core/src/runtime_stats_sink.rs`
  - `crates/xbxengine/core/src/transport/rtc/connection/transport_metrics.rs`
  - `crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs`
- Tauri projection:
  - `src-tauri/src/mods/xbxengine/trace_projection.rs`
  - `src-tauri/src/mods/streaming/types.rs`
- Shared DTO / Frontend:
  - `src/shared/rpc/xbxengine.ts`
  - `src/shared/rpc/streaming.ts`
  - `src/pages/*` 中消费统计/诊断展示的页面与组件
  - `src/streaming/*` 中执行态与监控态展示链路

## Plan

1. 观测口径审计：逐层盘点“字段定义 -> 投影 -> DTO -> UI 消费”，标出语义冲突与错误显示点。
2. 统一语义与兼容策略：确定权威字段与命名，设计最小破坏的兼容迁移（保留旧字段但标记废弃来源）。
3. 实施修复：后端投影与前端展示同步更新，补充 trace 中 BWE/TWCC 可读锚点。
4. 回归与文档：验证指标一致性、显示正确性、排障可读性，并更新口径说明。

## Validation

- [x] `cargo check -p xbxengine` 与相关定点测试通过（观测投影/DTO 映射不回归）。
- [x] trace 样本验证：`bweUpdated` / `twccFeedbackSent` / bitrate 字段在语义上互相一致。
- [x] 前端显示验证：同一时刻总码率、视频码率、音频码率与 trace/UI 投影口径对齐，无误标。
- [x] 兼容性验证：旧字段消费链不崩溃，迁移期有明确 fallback 行为。

## Risks

- 指标重命名或语义收紧可能影响现有前端/脚本的隐式依赖，需兼容窗口。
- 不同采样周期（stats tick、TWCC 反馈、UI 刷新）可能导致“短时不一致”观感，需要在展示层明确时间基线。
- 若缺少代表性 trace 样本，可能出现“语义正确但场景覆盖不足”的盲点。

## Progress

- [x] Step 1: 任务立项，完成 RFC 建档与初版范围界定。
- [x] Step 2: 完成跨层指标审计清单与冲突点标注。
- [x] Step 3: 完成后端口径统一与 DTO 对齐。
- [x] Step 4: 完成前端显示修复与回归验证。
- [x] Step 5: 完成文档补充并准备结项说明。

## Execution Notes

- Date: 2026-03-24 | Status: planned
- Update: 创建 RFC，定义了观测口径统一与错误显示修复的目标、范围、模块与验证基线。
- Decision: 本任务优先做“口径一致性与显示正确性”，不在本 RFC 内扩展 BWE 算法调参。
- Risk/Blocker: 待补齐当前 UI 全量消费点清单与典型 trace 样本对照表。
- Date: 2026-03-24 | Status: completed
- Update: 已将 `inbound_bitrate_kbps` 统一为 video+audio 聚合口径，新增显式 `video_bwe_*` / `video_twcc_*` 字段，并贯通 Rust stats、protocol DTO、trace snapshot、前端运行时映射与性能面板展示。
- Decision: 保留 `video_remb_bps` / `br` 作为兼容字段，但新增 `bitrate.totalKbps`、`bwe.targetKbps`、`bwe.actualVideoKbps`、`twcc.receiveKbps` 等显式语义字段，避免 UI/trace 继续猜测字段含义。
- Risk/Blocker: 仍存在采样周期差异导致的短时数值抖动观感，但字段语义已统一；后续若扩展图表面板，需继续沿用本 RFC 口径。
