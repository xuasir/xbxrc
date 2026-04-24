# Video Source Backpressure Priority And Recovery Softening RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 新 trace `runtime-trace-1775342719133.jsonl` 显示，链路已进入 `steady/healthy` 后，首个异常不再是 `adapterIdleTimeout`，而是 `video source sink ingress dropped err=no available capacity`。
- 当前 `video_source/sink.rs` 把 primary / repair passthrough / RTX reinject 全部归一化后塞进同一个有界 `mpsc` 队列，满队列时统一 `try_send` 失败即丢弃。
- 本地背压丢弃会在下游被解释成普通 packet gap，再进一步放大为 `repair-in-flight -> awaitingRecoveryKeyframe -> transportAwaitRecoveryKeyframe -> reconnect`，导致健康链路在开始游玩后再次陷入恢复风暴。

## Goal

- 为 `video_source` ingress 引入更鲁棒的分级背压语义，优先保护 repair / RTX / 关键恢复相关流量，避免统一队列把健康链路直接打断。
- 为 source / timeline 增加本地背压感知，避免把本地过载导致的低价值缺口直接升级成强恢复语义。
- 保持真实 repair / 关键帧恢复失败仍然可以及时推进恢复。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/sink.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/mod.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/*`
  - `crates/xbxengine/core/src/transport/rtc/session/policy.rs`
  - `crates/xbxengine/core/src/api/runtime/*`
  - 必要的 runtime stats / diagnostics 补充
  - 定向回归测试
  - RFC / Report / `docs/project-task.md` 跟踪
- Out of scope:
  - 大范围重写整体 recovery coordinator / session policy
  - 非 `video_source` 主链的 UI 或平台层改动
  - 与本轮问题无直接关联的 BWE / ICE 策略调整

## Plan

1. 在 sink 层引入 ingress 分级背压与显式本地 drop reason，优先保护 repair / RTX / 关键恢复流量。
2. 在 source / timeline 层增加本地背压感知，收紧 `repair-in-flight -> awaitingRecoveryKeyframe` 的升级条件。
3. 补齐 sink/source/timeline 定向回归测试，并完成编译与文档闭环。

## Validation

- [x] `cargo test -p xbxengine transport::rtc::stream::video_source::sink -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::stream::video_source::source -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
- [x] `cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`
- [x] `cargo test -p xbxengine api::runtime::tests -- --nocapture`
- [x] `cargo check -p xbxengine`

## Risks

- 分级背压策略如果过于复杂，可能引入新的时序分叉或额外的队列饥饿问题。
- 如果本地背压感知口径过宽，可能误抑制真实网络损坏导致的恢复升级。

## Progress

- [x] Step 1: 已完成 trace 倒查，确认首个异常来自 sink 背压丢弃，而非 `adapterIdleTimeout`。
- [x] Step 2: 已在 `sink` 落地 priority / best-effort 分级背压、有界本地 backlog 与显式 local backpressure drop 观测；`source/timeline` 侧沿用既有 `localBackpressureDeltaGap` 与 `hard_recovery_gap_risk` 软化语义，并通过定向回归确认持续生效。
- [x] Step 3: 已在 `nack/timeline` 增加 `displayStarvedLowValueAdmission`，把显示链长时间 starved 背景下的小 delta gap 降级为软缺口，不再制造新的恢复债务。
- [x] Step 4: 已把 `displaySupplyCritical` 从 `AdapterIdleTimeout` 域中剥离，改为独立本地域 reason，并在 `session/coordinator/nack/runtime lifecycle` 四层同时加 gate，禁止本地供给问题被洗成 transport reconnect。
- [x] Step 5: 已把 `reason_domain` 作为强类型字段从 `planner/scheduling -> transport session -> pending runtime action -> runtime lifecycle` 贯通，并补齐 local/displaySupply 拒绝、peer/liveness/transport 放行、staging 保留 domain 的定向测试与文档闭环。

## Execution Notes

- Date: 2026-04-05 | Status: completed
- Update: 本轮最终交付聚焦在 `video_source/sink.rs`。统一 `try_send` 失败即丢改为分级背压：repair passthrough / RTX reinject / 恢复优先 H264 primary 进入 priority backlog，普通 delta 仅保留最新 best-effort，一个旧包被更新包替换时会显式记录 `video_frame_drop`。
- Decision: 不再对 `source/timeline` 额外叠补丁。现有实现已经具备 `localBackpressureDeltaGap`、`cloudHighRttLowValueAdmission` 与 `hard_recovery_gap_risk` 软化路径，本轮通过回归测试确认这些语义继续生效，避免设计漂移。
- Risk/Blocker: 分级背压已经避免 repair / RTX 与普通 delta 一起被无差别挤掉，但真实运行态仍需用下一份 runtime trace 确认 steady 建连后开始游玩时，不再先出现 `video source sink ingress dropped err=no available capacity` 再立即升级成恢复风暴。
- Date: 2026-04-05 | Status: completed
- Update: 追加分析 [`runtime-trace-1775345271853.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775345271853.jsonl) 后确认，`sink` 背压主因已消失，但新的末段故障链变成“显示链已长期 `displaySupplyStarved`，小型 delta gap 仍被写成 `gapRepairInFlight -> awaitingRecoveryKeyframe -> reconnect`”。因此补充在 `nack` admission 侧新增 `displayStarvedLowValueAdmission`，把 `critical no-pending + stale present` 背景下的低价值 delta gap 直接降级为软缺口，不再制造新的恢复债务。
- Decision: 这次不把修复点放在 `source`。因为 trace 证明升级发生在 sample 组帧前，最稳健的收口点是 `nack` admission：在进入 `gapRepairInFlight` 前就把显示链已饿死时的小 gap 识别为低价值本地掉帧；`timeline` 只负责把新 reason 视为软缺口，保持 hard risk 语义清晰。
- Risk/Blocker: 当前 `displayStarvedLowValueAdmission` 依赖 host no-pending pressure 与 present staleness 联合判定，后续仍需用下一份真实 trace 复核阈值是否过宽或过窄，避免误伤真正需要 repair 的 delta gap。
- Date: 2026-04-05 | Status: completed
- Update: 第三阶段把 recovery/reconnect 语义做了域分离。`displaySupplyCritical` 在 [`session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 不再映射到 `AdapterIdleTimeout`，而是走已存在的 `DisplaySupplyCritical` 本地域 reason；[`recovery/nack_outcome.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/nack_outcome.rs) 不再把这类本地供给故障在重要帧过期时改写成 `TransportAwaitRecoveryKeyframe`；[`recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 进一步把它归入 `Local` 域；[`api/runtime/lifecycle.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/runtime/lifecycle.rs) 与 [`api/runtime/mod.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/runtime/mod.rs) 增加 pending reconnect candidate 的最终 domain gate，拒绝本地 reason 落成重连。
- Decision: 这轮不再继续靠 suppress patch 微调阈值，而是明确“本地供给/显示域”和“connectivity/transport 域”边界。这样就算后续上游误产出了 pending reconnect candidate，runtime lifecycle 末端也会再挡一次，避免本地缺口重新回流到 reconnect。
- Date: 2026-04-05 | Status: completed
- Update: 收尾阶段已补齐“强类型 `reason_domain` 透传”测试与文档配套：[`planner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/planner.rs)、[`scheduling.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/scheduling.rs)、[`transport_session.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs) 与 [`api/runtime/mod.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/runtime/mod.test.rs) 现在共同验证 `displaySupplyCritical/localBackpressure` 会被 runtime gate 拒绝，`peer-closed`、`transportExpiredDeadline`、`livenessNoProgressTimeout` 会因 `ConnectivityTransport` 域被放行，且 staged pending reconnect action 不会丢失 `reason_domain`。
- Decision: `reason_domain` 已成为 reconnect 候选的主口径，runtime 不再依赖 reason 字符串白名单做最终消费门控；后续新增 reconnect reason 时，优先保证 domain 赋值正确，再决定 label 文案。
- Risk/Blocker: 仍需在后续真实 trace 中验证更多 connectivity 子类 reason 都已正确标注为 `ConnectivityTransport`；如果未来出现跨域复合 reason，仍需要明确“谁负责裁决最终 domain”的合同边界。
