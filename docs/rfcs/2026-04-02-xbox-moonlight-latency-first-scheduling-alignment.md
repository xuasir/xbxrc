# Xbox 参照 Moonlight 的低延迟优先调度对齐 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- `moonlight-qt` 的低延迟能力并不是单点 pacer，而是“网络画像 + 包级恢复 + 帧级准入 + 解码/渲染队列上限 + 显示时钟 + 平台低延迟提示”协同收敛出来的结果。
- 当前 `xbxrc` 已经具备多项同方向基础能力：Cloud 高 RTT 下的 `latency-first` NACK admission、`IngressDecision::DropUnrecoverable`、pacer/present age budget、按 `HomeLan/Cloud/Relay` 划分的 recovery profile、host display telemetry 回灌等。
- 但这些能力还没有形成一套统一的“Xbox 远端画像驱动”的端到端调度合同；不同层仍存在局部阈值、局部启发式和平台提示缺口，导致低延迟语义已部分落地，但还不够系统化、可调参、可扩展。

## Goal

- 以 `Moonlight` 的低延迟优先原则为参照，在不偏离现有 `Tauri + Vue 3 + TypeScript + Rust` 与 Rust-owned RTC 主线的前提下，形成 `xbxrc` 的统一调度优化方案。
- 把“保交互、控队列、快恢复、按画像分档”明确收敛为可实施的分层设计，而不是继续通过零散 trace 定点修修补补。
- 建立 `Xbox 远端画像 -> 调度预算 -> 恢复动作 -> 观测验证` 的单一主线，为后续实现和回归提供稳定坐标。

## Scope

- In scope:
  - `crates/xbox-streaming/src/policy/runtime/compiler.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/policy.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/*`
  - `crates/xbxengine/core/src/media/video/ingress/*`
  - `crates/xbxengine/core/src/media/video/decode/*`
  - `crates/xbxengine/core/src/media/video/pacer/*`
  - `crates/xbxengine/core/src/media/video/render/*`
  - `src-tauri/src/mods/native_video/*`
  - runtime stats / diagnostics / trace projection 中与调度预算、帧丢弃、恢复升级、宿主显示时钟相关的字段
- Out of scope:
  - 引入新的客户端技术栈或第二条 native runtime
  - 重写 Xbox 协议/传输主线，或另起一套非 RTC 媒体路径
  - 直接照搬 Moonlight 的 RTP/FEC 协议实现
  - 未经 RFC 单独审批的 codec/transport 架构漂移

## Plan

1. 先正式定义 `Xbox 远端画像` 合同，至少覆盖 `HomeLan / HomeRelay / CloudStartup / CloudSteady / CloudHighRtt / DecoderConstrained / DisplayConstrained` 七类画像，并明确输入信号来源。
2. 把现有 `NACK admission / ingress late drop / backlog drop / pacer deadline / present frame_age_budget` 收敛成同一套端到端预算，避免每层各自独立“觉得自己该丢”。
3. 在 `video_source -> ingress -> decode -> pacer -> present` 之间补齐帧价值和恢复状态继承，保证 `anchor / reference / supply / low-value` 的价值分层跨层不失真。
4. 参考 Moonlight 的 queue history / max outstanding frame / display clock 语义，强化 `pacer + native_video` 的显示侧调度，使其从“单帧 deadline 判断”升级到“供给压力受控泄洪”。
5. 按画像引入平台级低延迟支撑项：线程优先级、计时精度、可信本地链路 QoS、无线媒体模式等，但全部放在能力探测和安全 gate 后，不做无差别启用。
6. 为上述策略补齐统一观测面与 rollout 开关，先以 trace/面板验证“队列时延下降且恢复更快”，再逐层默认开启。

## Validation

- [ ] 实现前完成画像合同与预算合同评审，确保 `runtime/policy/recovery/media/native_video` 边界一致
- [ ] 每一层改造都补结构化 runtime trace 字段，能直接回答“为什么丢、为什么等、为什么升级”
- [ ] 分别用 `Home LAN / Home Relay / Cloud 高 RTT / Cloud 启动慢热 / 宿主显示受限` 五类样本做回归

## Risks

- 如果只新增更多阈值而不统一预算来源，系统会继续变成“每层都在做 latency-first，但合起来不稳定”。
- Moonlight 的很多收益来自协议形态、平台假设和 SDL/FFmpeg 线程模型；本项目如果只照抄局部策略，可能得不到同等收益，甚至引入新的抖动。
- 平台级 QoS / 线程优先级如果缺少画像 gate，容易在云端/公网/低配设备上造成副作用或误伤。

## Progress

- [x] Step 1: 已完成 `moonlight-qt` 与 `xbxrc` 当前实现的逐层对照分析
- [x] Step 2: 已确认可迁移主轴为“画像驱动 + 预算统一 + 价值分层 + 显示侧供给控制”
- [ ] Step 3: 待按本 RFC 拆分具体实现任务并逐层落地

## Execution Notes

- Date: 2026-04-02 | Status: planned
- Update: 本轮完成了面向 `moonlight-qt` 的低延迟调度深度分析，并对照 `xbxrc` 现有 `rtc / video / native_video` 主线收敛出可实施方案。
- Decision: 不引入 Moonlight 那套协议实现本身，保留当前 Xbox / RTC / Rust-owned 主线，只吸收其“低延迟优先”的调度原则与系统协同方式。
- Decision: 后续落地优先顺序定为 `画像合同 -> 预算统一 -> NACK/Ingress -> Pacer/Present -> 平台低延迟支撑 -> 观测与 rollout`。
- Risk/Blocker: 当前最大风险不是“不会做”，而是已有局部优化已经很多，若没有统一合同继续增量修改，容易把调度语义越做越散。
