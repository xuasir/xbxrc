# Repair RTX Explicit Reinject RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 当前 [`video_source/sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/sink.rs) 已从“非 RTX repair 一律丢弃”收敛为“RTX 解包 / repair-primary 直通 / unsupported repair 丢弃”。
- 但 repair/RTX 目前仍是 sink 本地隐式改写为 `RtcVideoRtpPacket` 再直接送入 source，下游无法显式区分包的来源与接受原因。
- 这种“先改写成像 primary 的包，再由 source 事后推断是否命中 gap/nack”的模式不够鲁棒，会让 repair provenance、观测链和后续 reinject 调优持续耦合不清。

## Goal

- 为 repair/RTX 引入显式 reinject provenance，让 source 在消费时能区分 primary / RTX / repair-primary-pass-through。
- 保持现有主架构不变，只做最小闭环改造，不引入新的独立 repair worker 或并行 transport 路径。
- 让 runtime stats 的 RTX reinject stage 在“包进入 source 时”就具备完整来源信息，而不是仅靠 latest observation 反推。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/stream/packet_types.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/sink.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs`
  - 相关 tests / diagnostics glue
- Out of scope:
  - 新增独立 reinject queue actor
  - 引入全新 repair quarantine 子系统
  - 大规模重构 `packet_router` 或 media ingress service

## Plan

1. 为 `RtcVideoRtpPacket` 增加 repair/RTX provenance，定义 source 可消费的最小来源语义。
2. 调整 sink 的 repair/RTX 归一化逻辑，让 RTX 解包结果携带原始 repair 信息进入 source。
3. 在 source 消费路径中显式记录 reinject stage，而不是依赖 latest observation 的隐式串联。
4. 补齐 sink/source 定向测试，验证 repair provenance、stage 投影与现有帧组装行为兼容。

## Validation

- [x] `cargo test -p xbxengine transport::rtc::stream::video_source::sink -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::stream::video_source::source -- --nocapture`
- [x] `cargo test -p xbxengine diagnostics::stats -- --nocapture`
- [x] `cargo check -p xbxengine`

## Risks

- provenance 字段如果设计过重，会把 `sink -> source` 接口扩成另一个半套 repair 子系统。
- source 若对 provenance 分支处理不当，可能让正常 primary 包也进入额外慢路径。
- 如果继续保留过度乐观的 PT fallback，显式 provenance 只能提升可观测性，不能真正降低误投递风险。

## Progress

- [x] Step 1: 明确 repair/RTX 显式 provenance 结构
- [x] Step 2: 完成 sink-source 显式 reinject 改造
- [x] Step 3: 完成测试与编译验证
- [x] Step 4: 回填 RFC 与任务跟踪

## Execution Notes

- Date: 2026-04-04 | Status: planned
- Update: 用户要求继续把 repair/RTX 识别和投递改得更健壮、更有鲁棒性，本轮计划把 sink 的隐式改写收敛为显式 provenance + source 内可解释的 reinject 流程。
- Decision: 继续沿现有 `sink -> source` 主路径演进，不新增独立 repair worker；优先最小改造达成“来源可解释、阶段可验证”。
- Date: 2026-04-04 | Status: completed
- Update: 已在 [`packet_types.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/packet_types.rs) 为 `RtcVideoRtpPacket` 增加 `RtcVideoIngressKind` / `RtcVideoRepairMetadata`，让 repair-primary-pass-through 与 RTX reinject 都以显式 provenance 进入 source；[`sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/sink.rs) 去掉“无 payload map 时按 `pt=97` 猜 RTX”的乐观 fallback，并在成功排入 source 前记录 `queued`；[`source.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs) 改为直接按 provenance 推进 `adapterRead / sampleBuilderPush / adapterResolved / adapterResolveMiss` 等 reinject stage。
- Decision: 不新增独立 repair 队列或 quarantine 子系统，先把现有主路径做成“识别更保守、来源可追踪、阶段可验证”的最小闭环；同时避免 repair SSRC 污染 primary 媒体 SSRC。
- Update: 已补定向回归，包括 sink 侧 provenance 断言与 `repair_rtx_without_payload_map_is_not_reinjected_by_pt_guess`，以及 source 侧 `repair_rtx_packet_keeps_explicit_provenance_through_source_stage_updates`，并完成 `cargo test -p xbxengine transport::rtc::stream::video_source::sink -- --nocapture`、`cargo test -p xbxengine transport::rtc::stream::video_source::source -- --nocapture`、`cargo test -p xbxengine diagnostics::stats -- --nocapture`、`cargo check -p xbxengine`。
