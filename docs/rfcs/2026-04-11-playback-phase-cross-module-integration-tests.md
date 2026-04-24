# Playback 阶段跨模块集成测试设计 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: agent
- Last Updated: 2026-04-11

**Validation:** `cargo test -p xbxengine playback_phase`（36 条，`session/playback_phase_integration/*`）。

## Background

- 当前播放期回归并非单点故障，而是跨层状态切换不一致导致的组合问题：
  - `streaming startup ready` 已成立，但 viewport/transport/media 首帧链路存在长迟滞窗口。
  - 进入播放后存在持续 `decode:drop:outputQueueOverflow` 与 `present:dropLate`，并周期性回落到 `supply-starved/recovery-eligible`。
  - transport 仍保持 `Connected` 且持续有 ingress 的情况下，恢复策略仍可能自旋在 `waitForBurst`/`keyframe in-flight` 抑制态。
- 既有测试以模块单测和局部回归为主，缺少播放期“跨模块、跨阶段、可回放证据驱动”的集成测试矩阵。

## Goal

- 基于现存 trace 设计播放期跨模块集成测试用例（不写实现）。
- 固化播放期的阶段模型、异常分类、跨模块断言口径，减少后续修复只补局部回归的风险。
- 让每条用例都可映射到具体 trace 证据窗口（`seq/tsMs/event`），用于**溯源与评审**；运行时的测试输入必须来自**独立 fixture**，见下节约定。

## Trace 与测试输入的关系（硬性约定）

- **Trace 只负责「挖场景、定口径、留证据锚点」**：从原始 `runtime-trace-*.jsonl` 中识别组合态、事件顺序与误判模式，再抽象成用例的「驱动输入」与「禁止项」。
- **集成测试不得直接依赖 trace 文件**：实现与 CI 不得挂载、读取或断言整份 `runtime-logs/*.jsonl`；不得把「在某份 jsonl 上的 seq 回放」作为稳定测试手段。仓库内应使用**检入的裁剪事件序列 / 合成事件序列**（或等价的最小 fixture），与具体一次抓取的文件名、路径解耦。
- **表格中的 `A:seq` / `B:seq` 为溯源锚点，不是测试绑定**：仅说明该用例设计来源于哪段真实证据；fixture 中可采用**相对顺序与字段语义**重现，无需复现原始 `seq` 数值。
- **与「测试资产格式建议」一致**：输入资产以 case 为单位的 `events[]` 等结构为准；若需对照 trace，可在 fixture 元数据中保留可选字段 `provenance: { traceId, anchorSeqs[] }`，不参与断言。
- **含「次数/高频」的边界用例**：表格里若出现「N 次」「洪峰」等，仅表示在原始 trace 中的观察强度；**fixture 用短序列循环或参数 `repeat_count` / `threshold` 表达**，不得依赖整段 jsonl 计数。

## Scope

- In scope:
  - 基于以下 trace 的播放期证据抽样与用例设计（文件可能仅存在于本地或制品库，**不作为测试运行时依赖**）：
    - `runtime-logs/runtime-trace-1775888724926-1.jsonl`（Trace-A）
    - `runtime-logs/runtime-trace-1775896730947-1.jsonl`（Trace-B）
  - 覆盖 `streaming -> xbxengine -> native_video -> recovery/session policy` 的跨模块集成测试设计。
  - 测试资产组织、用例矩阵、断言口径、验收清单。
- Out of scope:
  - 不修改运行时代码（仅新增/扩展**测试**与文档跟踪）。
  - 不新增埋点字段或 trace schema。
  - 不替代既有单元测试，仅定义新增集成测试层。

## Trace 证据摘要

- Trace-A（`1775888724926-1`）：
  - `seq=7517`：`streaming/startupPhase=ready:succeeded` 已成立。
  - `seq=14055`：`xbxengine-host/nativeViewportAttached(surfaceId=wgpu:stream-page-video)`。
  - `seq=17142`：`frameDropped(detail=outputQueueOverflow, stage=decode)` 与 `seq=17151 first_present` 同窗口重叠，说明首帧建立和 decode 队列压力并发出现。
  - `seq=21372`、`27351`、`34808`：持续 `frameDeadlineMissed(reason=dropLate, stage=present)`。
  - `seq=47969`：进入 `sessionPhase=recovering + streamLifecyclePhase=recovery-eligible + videoOwnerState=rebuilding-supply`，输入信号为 `transportAwaitRecoveryAnchor`。

- Trace-B（`1775896730947-1`）：
  - `seq=4487`：`streaming/startupPhase=ready:succeeded`。
  - `seq=8498`：`nativeViewportAttached(surfaceId=wgpu:stream-page-video)`。
  - `seq=11527`：首个 `decode:drop:outputQueueOverflow`，并与 `decoderRecoveryStateChanged(bootstrap-keyframe-accepted)`同窗。
  - `seq=18211`、`23996`、`26225`：播放期 `dropLate` 持续出现。
  - `seq=204380`：`transport=Connected + ingress 持续` 条件下，仍处于 `displaySupplyStarved + recovery-eligible + waitForBurst(suppressed)`。

## 播放期阶段模型（用于集成测试分层）

- 播放期不等于单一 `steady`，需覆盖以下组合态：
  - `steady/stable-serving/healthy`
  - `degraded/degraded-serving/healthy`
  - `recovery-eligible/rebuilding-supply|supply-starved/displaySupplyStarved`
- 用例需验证“阶段切换正确性”，而不是仅验证某个阶段内的局部动作。

## 影响模块

- `src/streaming/runtime/*` 与启动阶段里程碑链路（ready、runtime started）。
- `src-tauri/src/mods/xbxengine/*` 与 trace projection/runtime_state 同步口径。
- `src-tauri/src/mods/native_video/*` 的 host timing/first present/cadence 信号。
- `crates/xbxengine/core/src/transport/rtc/stream/video_source/*` 的 gap/repair/NACK 来源。
- `crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner*` 的 owner 状态切换。
- `crates/xbxengine/core/src/transport/rtc/recovery/*` 与 `session/policy*` 的恢复升级与抑制逻辑。

## Plan

1. 固化播放期阶段枚举与跨模块状态映射表。
2. 按 trace 证据设计首批播放期集成测试矩阵（含正反向断言）。
3. 定义统一集成测试资产模型（输入事件序列、期望状态序列、禁止动作序列）。
4. 产出实现顺序建议与最小验收门槛，供后续编码轮次执行。

## 用例矩阵（首批，跨模块集成）

| Case ID | 场景目标 | 证据锚点 | 驱动输入（跨模块） | 关键断言（跨模块） |
| --- | --- | --- | --- | --- |
| PLY-INT-01 | `startup ready` 后 viewport 迟滞期间，不提前判定播放成功 | A:7517->14055, B:4487->8498 | `streaming.startupPhase=ready` + `nativeViewportAttached` 延迟到达 | `streaming` 侧里程碑成立不应直接驱动 `stable-serving`；需等待 `native_video first_present` 与 `xbxengine` 首帧证据收敛 |
| PLY-INT-02 | 首帧建立与 decode overflow 并发时，允许局部丢帧但不得误升级 reconnect | A:17142/17151, B:11527 | `frameDropped(outputQueueOverflow)` + `first_present` + `transport=Connected` | `video_source/decode/native_video` 可出现局部 drop；`recovery/session` 不得直接触发 reconnect 候选 |
| PLY-INT-03 | steady 播放期连续 `dropLate` 下，维持播放并记录降级，不进入恢复风暴 | A:21372/27351/34808, B:18211/23996/26225 | `present dropLate` 连续脉冲 + ingress 持续 | owner 可降级到 `degraded-serving`，但 `coordinator` 不应仅凭 present stale 连续触发高强度恢复动作 |
| PLY-INT-04 | `transportAwaitRecoveryAnchor` 进入 `rebuilding-supply` 后，keyframe in-flight 只允许有界抑制 | A:47969, B:204380 | `videoTimeline.awaitingRecoveryKeyframe` + `keyframeRequestEpisode=sent` + `coalesced:keyframeInFlight` | `recoveryDecisionLedger` 抑制态需可退出；不能在 `Connected+ingress` 下无限停留 `waitForBurst/coalesced` |
| PLY-INT-05 | supply-starved 窗口中“网络活着但播放链不健康”必须被明确分类 | B:204380 | `transport=Connected` + `inbound bytes/fps > 0` + `decoder/present age 高` + `renderer stalled` | `sessionPhase` 可为 `recovering`，但必须显式归因为 `displaySupplyStarved`，不得误标为 connectivity failure |
| PLY-INT-06 | 恢复退出条件成立后必须回到稳定态，不残留陈旧恢复状态 | **B:205083**（`recovery-settled`，与 PLY-INT-12 同源）及随后有限步内 owner/session 稳态回归；**A:47969** 为 recovering 入口侧，A 侧「出口→稳态」首尾 seq 由实现轮次在 Trace-A（47969 之后）检索补记 | `clean anchor + decode progress + present progress`（fixture 合成，不 replay jsonl） | owner 从 `rebuilding-supply/supply-starved` 退出到 `stable-serving`；session 不继续维持 `recovery-eligible` |
| PLY-INT-07 | 高 RTT 播放期中 local ingest 异常与 transport 断连需严格分域 | **Trace-A/B** 中 `remoteProfile`/`subprofile` 含 **`cloudHighRtt`**，或与 **`cloudHighRttLowValueAdmission`** 同类诊断并行出现之窗口（实现轮次在 jsonl 内按字段检索补 seq）；无单点 seq 时以 profile + 事件模式为溯源 | `target_profile=cloudHighRtt` + `packetGap`/`nack`/`repair` 与 `transport=Connected` 并存（fixture 合成） | local 恢复动作优先，reconnect 只能由明确 connectivity 证据触发 |
| PLY-INT-08 | 播放期 hard fallback 到 reconnect 的升级门槛必须可验证 | **B:204380** 一带语义窗口；整段统计仅溯源，**fixture 用参数化短序列**表达 budget 耗尽与 keyframe Await 持续失败 | `decoder reset budget` 耗尽 + `await keyframe` 持续失败 | 允许升级到 reconnect candidate，但要满足预算/证据门禁；不允许无证据越级 |

## 用例矩阵（第二批扩展，覆盖恢复自旋与收敛边界）

| Case ID | 场景目标 | 证据锚点 | 驱动输入（跨模块） | 关键断言（跨模块） |
| --- | --- | --- | --- | --- |
| PLY-INT-09 | `keyframe in-flight coalesced` 长时间重复时必须可解锁 | B:33648/33661/33679/33696 | `transportAwaitRecoveryAnchor` 连续输入 + `gateResult=coalesced:keyframeInFlight` 重复 | `coordinator/session` 不能无限阻塞；必须出现可验证的解锁条件或升级动作 |
| PLY-INT-10 | `transportDeferred` episode 不得伪装成恢复成功 | B:33649/33697, A:49252/49285 | `keyframeRequestEpisode.status=deferred` + `statusDetail=sameFamilyCoalesced:transportStageSuppressed` | owner 不能误回 `stable-serving`；恢复链保持“未完成”直到出现 fresh 成功证据 |
| PLY-INT-11 | deferred + invalid bootstrap 终态下应重开 keyframe 请求而非原地循环 | B:33716/33717/33718 | `terminalDeferredInvalidBootstrap` + `bootstrapRejectReason=NonIdrVcl` | `recoveryDecisionLedger` 应从纯 coalesced 转为显式 `requestKeyframe` 或更强动作，避免死循环 |
| PLY-INT-12 | `recovery-settled` 事件后应在有限步内退出恢复面 | B:205083（recovery-settled） | `decoderRecoveryStateChanged=recovery-settled` + decode/present 有进展 | `sessionPhase/streamLifecyclePhase/videoOwnerState` 在有限窗口回归稳态，不残留 `recovery-blocked` |
| PLY-INT-13 | 连续 decoder reset 请求必须受预算与证据共同约束 | B:206773/208230/210504 | `decoderRecoveryStateChanged=decoderResetRequested` 连发 + `transport=Connected` | reset 不得无界重放；超过阈值后应转 reconnect 候选或进入显式抑制态并可退出 |
| PLY-INT-14 | `remoteTrackAttached + ingress 增长` 不等于可播成功 | A:47975, B:204385 | `runtimeEventRaw.MediaVideoTrackStatusChanged(remoteTrackAttached)` + videoBytes 持续增长 | 仍需结合 `native_video` 与 `present/decode freshness` 判断，不得直接推进为 `healthy+stable-serving` |
| PLY-INT-15 | `TWCC/RTCP 稳定` 与 `displaySupplyStarved` 并存时不得误判网络故障 | B:204380（twcc stable + supply-starved） | `twcc stable` + `lossRatio≈0` + `displaySupplyStarved` | 诊断域应保持 display/local lane，不应直接归因为 connectivity lane |
| PLY-INT-16 | `first_present` 后短窗内允许局部波动，但要在窗口外收敛 | A:17151 + 17142 + 21372 | `first_present` 紧邻 `outputQueueOverflow/dropLate` | 允许短窗 degraded；超过窗口需回稳或进入受控恢复，不可持续振荡无判决 |
| PLY-INT-17 | `startup->playback` 跨层里程碑必须单调推进，不允许回跳 | A:7517->14055->17151, B:4487->8498->11527 | startup ready、viewport attach、first present、decode/present 进展 | 里程碑状态机单调：不应在已满足后回退到早期启动语义 |
| PLY-INT-18 | `supply-starved` 与 `rebuilding-supply` 的切换要可解释且可复盘 | A:47969, B:204378/204394 | videoOwner 在 `rebuilding-supply/supply-starved` 间切换 + `transportAwait` 输入持续 | 每次切换都要有对应证据（anchor/gap/decode/present）；禁止无证据抖动切换 |

## 边界 Case 矩阵（第三批，极端组合与反直觉路径）

| Case ID | 边界主题 | 证据锚点 | 边界输入组合 | 必须守住的断言 |
| --- | --- | --- | --- | --- |
| PLY-EDGE-01 | `transportDeferred` 洪峰不应被误判成功 | A:49252/49285, B:33649/33697 | `keyframeRequestEpisode.status=deferred` 高频 + `responseVerdict=transportDeferred` | 任何“恢复成功”判定都必须等待 fresh on-time/decode 成功证据，不能用 deferred 代替 |
| PLY-EDGE-02 | `NonIdrVcl` 持续拒收下避免假进展 | A:bootstrapRejectReason=NonIdrVcl(2426+), B:6070+ | inspection 持续 `NonIdrVcl` + `deltaContinuationReady=true` | recovery 不能被 `delta continuation` 伪装为可播稳定；需保持恢复态或升级 |
| PLY-EDGE-03 | `coalesced:keyframeInFlight` 极长串的退出性 | B:33648/33661/33679/33696 | input 持续 `transportAwaitRecoveryAnchor` + gate 持续 coalesced | 必须存在“解锁条件 -> 新动作/新阶段”路径；禁止无穷同态循环 |
| PLY-EDGE-04 | `suppressed:waitForBurst` 高频下的上界 | Trace-B 曾观测 **4485** 次 `waitForBurst`（仅溯源）；fixture：`repeat_count`/`threshold` + 短序列 | 连续 `waitForBurst` + ingress 持续 + output 低迷 | 超过阈值后必须触发可验证的阶段推进（probe/reset/reconnect candidate），不允许永远等待 |
| PLY-EDGE-05 | `suppressed:cooldownSuppressed` 不能吞掉真实故障 | B:溯源 **1065**（reconfigure）+ **266**（transportAwait）（实现轮次向 trace 核对为 seq 或段内计数）；fixture 合成「cooldown 抑制 + 问题链持续恶化」 | cooldown 抑制 + 问题链持续恶化 | cooldown 只能限频，不得改变最终故障归因与升级可达性 |
| PLY-EDGE-06 | `coalesced:decoderResetInFlight` 与 reset 预算联动 | Trace-B 曾观测 **214** 次 in-flight 吸收（仅溯源）；fixture：`repeat_count` + budget 参数 | reset in-flight 吸收 + reset 请求重复触发 | 重复 reset 必须受 budget 控制；budget 触顶后进入下一层动作而非 reset 自旋 |
| PLY-EDGE-07 | 音频 TWCC 干扰不应污染视频恢复判定 | B:11654/11778, A:17339 | `twcc feedback ignored for non-video stream` + video recovery 并行 | 反馈通道归因必须按媒体类型隔离，避免 audio 干扰 video recovery gate |
| PLY-EDGE-08 | `twcc=local-feedback stable` 与 `displaySupplyStarved` 并存 | B:204380 | 网络指标稳定 + renderer/decoder stalled | 明确归因为 display/local 约束，禁止走 connectivity reconnect 快路径 |
| PLY-EDGE-09 | `latestSlotOverwrite` 高频替换的边界 | B:11646/11682, A:17296/17347 | render replace 连续 + present 仍推进 | 允许 latest overwrite 保护时效，但不得导致 owner 错判为健康稳态 |
| PLY-EDGE-10 | keyframe gap 修复的“假修复”边界 | A:17206/17208/17210 | `packetGapDetected(keyframe)` 后 `nackRecovered` | `gap recovered` 不等于会话恢复完成；需继续验证 decode/present 连续性 |
| PLY-EDGE-11 | `expired-unsent` 请求终态边界 | Trace-A/B 曾分别观测 3/2 次（仅溯源）；fixture：至少 **1** 次 `expired-unsent` + 无 `sentAt` 即可判语义 | keyframe episode `expired-unsent` + 无 sentAt | 过期未发送必须进入明确补救分支，不能被吞并成无信号 |
| PLY-EDGE-12 | `missed/deadlineExpired` 的恢复出口 | Trace-B 曾观测 `deadlineExpired` **3** 次（仅溯源）；fixture：短序列复现 `missed`/`deadlineExpired` + 历史 clean anchor | response missed + deadlineExpired + clean anchor 历史存在 | 不得停在 `recovery-blocked`；要么重开 keyframe，要么升级动作 |
| PLY-EDGE-13 | `startup -> degraded -> steady` 快速摆动 | B:11651(degraded) -> 11775(steady) | first present 后短窗 degraded，随后 steady | 状态机可摆动但必须单调收敛；禁止反复回跳 startup 语义 |
| PLY-EDGE-14 | `transport connected` 下的慢性低 fps 边界 | B:204380（inbound 高、presentFps≈1） | inbound bytes 高 + decode/present age 巨大 + renderer stalled | 必须识别为“链路活着但不可持续输出”，并驱动恢复，不可误判为网络断 |
| PLY-EDGE-15 | `recovery-settled` 与后续 reset 冲突边界 | B:205083 -> 206773/208230/210504 | 刚 settled 后短时间再次 reset requested | 需要验证 settled 的稳定窗口；若快速回退，必须有新证据触发而非旧状态回放 |
| PLY-EDGE-16 | `reconfigure` 与 `transportAwait` 双信号竞争 | B:reconfigure cooldown 抑制 + transportAwait 抑制 | 两条输入信号交错出现 | 信号仲裁必须可解释：主导问题链唯一，避免 action 在两条链来回抢占 |
| PLY-EDGE-17 | `sourceEvent=gap-resolved` 与 `awaiting-recovery` 并存边界 | A/B 多窗口 | 同周期出现 repaired 与 awaiting-recovery | timeline 必须给出确定主状态；禁止“局部修复成功”掩盖全局未恢复 |
| PLY-EDGE-18 | `nativeViewportAttached surfaceId=null -> wgpu:*` 双跳边界 | A:8446->8498, B:13997->14055 | 先 attach 无 surface，再 attach 有效 surface | 仅有效 surface 才能驱动后续播放成功判定；null attach 不能提前放行 |

## 每条用例统一断言模板

- 输入层（事件）：
  - `streaming`、`xbxengine runtime event`、`native_video hostTiming` 三类输入需齐全。
- 状态层（阶段）：
  - 至少断言 `sessionPhase`、`streamLifecyclePhase`、`videoOwnerState` 三个维度的一致性。
- 决策层（恢复）：
  - 断言 `recoveryDecisionLedger.inputSignal/gateResult/actionSelected` 与预期匹配。
- 约束层（禁止项）：
  - 明确“本场景下不允许出现的动作”，例如无证据 `reconnect`、无限 `waitForBurst`、错误域别升级。

## 测试资产格式建议（供后续实现）

- 输入资产：
  - 以“裁剪后或合成的事件序列”组织：**从 trace 提取灵感与字段取值，检入为独立 fixture**；运行时不读取原始 trace 文件。
  - 每个 case 包含：`events[]`、`initial_state`、`target_profile`、`time_budget_ms`；可选 `provenance` 仅作文档溯源。
- 期望资产：
  - `expected_state_transitions[]`
  - `expected_decision_transitions[]`
  - `forbidden_actions[]`
- 校验粒度：
  - 使用 `seq` 顺序语义，不绑定绝对 `tsMs` 数值。

## Validation

- [ ] RFC 首批用例可覆盖播放期三大组合态：`steady`、`degraded`、`recovery-eligible`
- [x] 每个 case 都在矩阵中标注至少一个 trace 溯源锚点（`seq` 或等价窗口说明）；实现时对应 fixture **不依赖**该 trace 文件存在
- [x] 每个 case 都包含至少一个“禁止动作”断言（首批落地用例均含禁止项）
- [ ] 用例矩阵覆盖 `streaming -> native_video -> xbxengine/recovery/session` 全链路（当前落地在 `RtcSessionPolicy` + runtime_stats 合同层）
- [x] 后续实现阶段至少落地 4 条 PLY-INT 用例并纳入 CI 目标（`cargo test -p xbxengine playback_phase`）

## Risks

- 若只对整条 trace 做端到端回放，测试会对噪声日志和时间戳过敏，维护成本过高。
- 若断言只停留在单模块输出，无法捕获“阶段切换不一致”这类真实回归。
- 若不定义禁止动作集，集成测试会退化为“仅验证发生了某事”，难以防止策略越级。

## Progress

- [x] Step 1: 完成 trace 证据抽样与播放期阶段模型归纳
- [x] Step 2: 完成首批跨模块集成测试矩阵设计
- [x] Step 3（部分）: 已落地 PLY-INT-01/02/05/17 共 4 条集成测试（`playback_phase_integration.test.rs`），复用 `RecoveryIntegrationHarness`；其余矩阵项待后续迭代

## Execution Notes

- Date: 2026-04-11 | Status: in-progress
- Update: 基于两份最新 trace 完成播放期阶段归纳与跨模块集成测试设计；本轮仅沉淀 RFC，不写代码。
- Decision: 播放期测试以“阶段切换一致性 + 恢复动作门禁 + 禁止动作”作为主合同，而非单点事件断言。
- Risk/Blocker: 当前仍缺统一的 trace 裁剪资产格式与 harness，需在实现轮次补齐。
- Date: 2026-04-11 | Status: in-progress
- Update: 在首批 8 条基础上扩展到 18 条测试矩阵，新增覆盖 deferred episode 回收、coalesced 自旋解锁、reset 连发预算门禁、twcc 稳定与 display-starved 并存、里程碑单调推进等高风险组合。
- Decision: 第二批用例优先约束“恢复链可退出性”和“错误域别升级”两类回归，不先增加更多低风险稳态场景。
- Date: 2026-04-11 | Status: in-progress
- Update: 继续补充第三批 18 条边界 case，优先覆盖 `transportDeferred` 洪峰、`NonIdrVcl` 持续拒收、suppressed/coalesced 高频自旋、audio-twcc 干扰、latest overwrite 假稳定等反直觉路径。
- Decision: 新增 case 全部要求“禁止误判成功”和“必须可退出”，以边界失稳保护为主，不再扩普通 happy-path。
- Date: 2026-04-11 | Status: in-progress
- Update: 实现轮次在 `xbxengine` 内新增 `playback_phase_integration.test.rs`（4 条用例）。**实现注意**：`session/facts::build_scheduling_demand_signal` 使用墙钟 `now_ms_f64()` 计算 present/decode 年龄，fixture 时间戳必须与墙钟对齐，勿写死小常量 epoch（已在测试文件模块注释中说明）。
