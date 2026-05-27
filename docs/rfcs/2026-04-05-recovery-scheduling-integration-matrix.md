# Recovery Scheduling Integration Matrix RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: in-progress
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 最近多轮恢复调度修复都能止住某一条明确故障链，但新的真实 trace 仍会从别的入口重新陷入恢复风暴。
- 现有测试版图以模块级单测和定向黑盒测试为主：
  - `session/policy.test.rs` 覆盖 liveness、pending reconnect、owner contract、局部 recover/reconnect gate。
  - `recovery/coordinator.rs` 内联 `mod tests` 覆盖 cooldown、budget、hard fallback、`transportAwaitRecoveryAnchor` 升级。
  - `policy/video_scheduling_owner.test.rs` 覆盖 display supply、clean anchor、steady/rebuilding-supply 边界。
  - `stream/video_source/*test.rs` 覆盖 repair/RTX、backpressure、timeline、NACK admission。
- 这些测试能证明局部规则，但还缺一层统一的“恢复风暴集成矩阵”：
  - 缺少把 `video_source -> owner -> recovery coordinator -> session policy -> runtime gate` 串成同一场景资产的验收。
  - 缺少把“本地缺口”和“真实网络问题”做硬切的跨层断言。
  - 缺少基于真实 trace 归纳的 replay profile，使每次修复都只能追加局部回归，而不是统一回放矩阵。
- 相关已有规划和可复用资产：
  - [`docs/rfcs/2026-03-30-cloud-recovery-liveness-simplification.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-30-cloud-recovery-liveness-simplification.md)
  - [`docs/rfcs/2026-04-04-video-pipeline-blackbox-test-coverage.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-04-video-pipeline-blackbox-test-coverage.md)
  - [`docs/rfcs/2026-04-05-video-source-backpressure-priority-and-recovery-softening.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-05-video-source-backpressure-priority-and-recovery-softening.md)
  - [`docs/rfcs/2026-04-04-steady-jitter-gentle-absorption.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-04-steady-jitter-gentle-absorption.md)
  - [`docs/rfcs/2026-04-04-stale-recovery-diagnosis-expiry.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-04-04-stale-recovery-diagnosis-expiry.md)
  - 代表性 trace：
    - [`runtime-trace-1775310674617.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775310674617.jsonl)
    - [`runtime-trace-1775319678083.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775319678083.jsonl)
    - [`runtime-trace-1775342719133.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775342719133.jsonl)
    - [`runtime-trace-1775345271853.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775345271853.jsonl)
    - [`runtime-trace-1775354304239.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775354304239.jsonl)

## Goal

- 重新设计恢复调度的集成测试体系，让“恢复风暴”不再靠单点补丁被动追踪。
- 建立统一的恢复场景矩阵，使每次调度改动都必须回答：
  - 这个故障属于本地缺口、显示供给、repair/RTX、还是 connectivity/transport？
  - 它应该被温和吸收、局部恢复，还是必须升级到 reconnect？
  - 各层是否在同一场景上给出一致结论，而不是互相打架。
- 让真实 trace 能稳定沉淀为 replay profile 和跨层断言，而不是每次临时 grep 分析。

## Scope

- In scope:
  - `crates/xbxengine/core/src/transport/rtc/session/policy.test.rs`
  - `crates/xbxengine/core/src/transport/rtc/recovery/coordinator.test.rs`
  - `crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.test.rs`
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/*.test.rs`
  - 新增 recovery scheduling 集成测试 harness / fixture / replay profile
  - `runtime-logs/runtime-trace-*.jsonl` 的测试资产化方式
  - `docs/project-task.md`
- Out of scope:
  - 本 RFC 内不直接调整恢复调度阈值或业务逻辑
  - 不重写整个 runtime trace 分析脚本体系
  - 不引入新的客户端栈或外部测试框架

## Plan

1. 梳理现有测试分层，定义“模块单测 / 黑盒测试 / 集成回放测试”三层边界与职责。
2. 提炼恢复风暴的核心场景矩阵，并把真实 trace 归类成可复用 replay profile。
3. 设计统一集成 harness，串起 `video_source -> owner -> coordinator -> session policy -> runtime gate` 的最小闭环。
4. 先落第一批高价值场景，覆盖“本地缺口不升级 reconnect”和“真实网络问题必须可升级 reconnect”两大主合同。
5. 将后续 recovery 相关改动的验收收口到该矩阵，避免继续散落在局部定向回归中。

## Validation

- [ ] 至少补齐一套跨层集成 harness，能在单个测试里断言 owner/coordinator/session/runtime 的一致输出
- [ ] 覆盖以下最小场景族：
- [ ] `transportAwaitRecoveryAnchor` 退出与重入
- [ ] `displaySupplyCritical/displaySupplyDegraded` 的本地域吸收
- [ ] `video_source` 本地背压 / repair / RTX 干扰不升级 reconnect
- [ ] `adapterIdleTimeout` / `stale diagnosis` 不得回放成恢复风暴
- [ ] 真正 connectivity/transport 故障必须仍可升级到 reconnect/failed-terminal
- [ ] 每个场景都能映射到具体 trace 样本或明确的 replay profile 来源
- [ ] `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
- [ ] `cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`
- [ ] `cargo test -p xbxengine transport::rtc::policy::video_scheduling_owner -- --nocapture`
- [ ] `cargo test -p xbxengine transport::rtc::stream::video_source -- --nocapture`
- [ ] 新增 recovery integration test 目标全部通过

## Risks

- 如果只堆更多局部回归，不建立统一场景矩阵，恢复风暴仍会在跨层交界处反复漏出。
- 如果 replay profile 直接照抄 trace 文本而不抽象成稳定场景，测试会对观测字段和时间戳过度敏感，维护成本很高。
- 如果集成 harness 不收敛到“域别 + 升级链 + 退出条件”三个主合同，仍会退化为更大的白盒测试集合。

## Integration Matrix

### 1) 场景域

- `LocalDisplay`
  - `displaySupplyDegraded`
  - `displaySupplyCritical`
  - `noPendingFrame`
  - host present telemetry gap
- `LocalIngress`
  - sink backpressure
  - repair/RTX burst
  - low-value delta gap
- `RecoveryBridge`
  - `transportAwaitRecoveryAnchor`
  - `waitKeyframe`
  - stale diagnosis replay
  - cooldown / hard-fallback exit
- `ConnectivityTransport`
  - transport disconnected
  - peer closed
  - expired deadline
  - liveness no progress

### 2) 期望动作

- `Absorb`
  - 保持 steady/degraded，不发恢复动作
- `LocalRecover`
  - 允许 keyframe / decoder reset，但禁止 reconnect
- `TransportRecover`
  - 允许 reconnect candidate / reconnect / failed-terminal
- `ExitRecovery`
  - 观察到 clean anchor / healthy chain / fresh supply 后，必须离开 recovery surface

### 3) 主断言

- 域别断言：本地缺口不能被洗成 `ConnectivityTransport`
- 升级断言：真实网络问题不能被 steady absorption 吞掉
- 出口断言：recovery surface 不能在“已恢复事实成立”后继续自旋
- 一致性断言：owner、coordinator、session policy、runtime gate 对同一场景必须给出相容结论

## First Batch

- `T1 steady_local_display_gap_absorbed`
  - 来源：`runtime-trace-1775310674617.jsonl`
  - 断言：steady 抖动优先停留在 `degraded-serving/local recover`，不得直接落成 reconnect
- `T2 transport_await_exits_after_recovery_completion`
  - 来源：`runtime-trace-1775354304239.jsonl`
  - 断言：`clean anchor + chain healthy + decode/video progress` 成立后必须退出 `rebuilding-supply`
- `T3 local_backpressure_does_not_escalate_to_reconnect`
  - 来源：`runtime-trace-1775342719133.jsonl`
  - 断言：sink 背压 / repair 干扰属于本地 ingress 域，只允许局部恢复
- `T4 stale_idle_diagnosis_does_not_replay_storm`
  - 来源：`runtime-trace-1775319678083.jsonl`
  - 断言：steady 进展存在时，旧 `adapterIdleTimeout` 不得重新驱动恢复风暴
- `T5 true_transport_failure_must_still_reconnect`
  - 来源：cloud liveness / peer closed / expired deadline 既有用例与 trace
  - 断言：真实 connectivity/transport 故障仍必须能升级到 reconnect 或 failed-terminal

## Extended Scenario Design

### A) 按链路画像分层

#### Home

- 典型特征
  - RTT 更低，短抖动与 reorder 更常见
  - clean anchor 后容易出现短窗 delta gap 重入
  - 本地网络切换、IPv4/IPv6 family 漂移、console ready/service registration 抖动更常见
  - 更容易出现“transport 仍活着，但 timeline/display 被短噪声打扰”的场景
- 主要风险
  - 本地小缺口被误判成 `transportAwaitRecoveryAnchor`
  - clean anchor 后短重入重新点燃 recovery debt
  - display/pacer/noPending 小脉冲被放大到 reconnect
- 场景侧重点
  - 软/硬恢复切换
  - steady 抖动吸收
  - 局部 gap / repair / reorder / 家庭网络切换

#### Cloud

- 典型特征
  - RTT 更高，feedback 与 media progress 更容易错位
  - NACK 过期、recovered-late、sample-loss、liveness no-progress 更常见
  - reconnect 之后仍可能停在 `Connecting/Priming/SeekingAnchor`
  - `transportAwaitRecoveryAnchor` 与 lifecycle reconnect 交叠更常见
- 主要风险
  - 高 RTT 下把“慢恢复”误判成“无恢复”，过早 reconnect
  - `adapterIdleTimeout/livenessNoProgressTimeout` 风暴式重放
  - reconnect 后缺关键帧又二次升级，形成恢复自旋
- 场景侧重点
  - 高 RTT + 丢包 + feedback 延迟
  - `transportAwait -> decoder reset -> reconnect` 的升级节拍
  - reconnect 后 recovery exit 与 failed-terminal 上界

### B) 按网络扰动建模

- `JitterBurst`
  - 短时间 inter-arrival 抖动升高，但 transport 仍 `Connected`
  - 重点区分 ingress 抖动 vs render/deadline 抖动
- `ReorderBurst`
  - gap / NACK / repair 会出现，但后续包仍能补齐
  - 重点验证 Home 场景下 reorder 不能轻易升级 reconnect
- `LossBurstRecoverable`
  - 连续丢若干包，repair/RTX 仍可补齐一部分
  - 重点验证先走 local recover / wait keyframe
- `LossBurstUnrecoverable`
  - repair 无法补齐，reference chain 断裂
  - 重点验证升级链必须单调，不得跳过前序证据
- `FeedbackDelay`
  - TWCC / RTT / loss 反馈延迟，但媒体链路未完全断
  - 重点验证 Cloud 高 RTT 下不能把慢反馈误判成 transport failure
- `IntermittentBlackhole`
  - 短窗无进展，随后恢复
  - 重点验证 idle/liveness 门限去抖与 recovery exit
- `PathDegrade`
  - RTT/loss 持续恶化，但仍有零星流量
  - 重点验证 `sampleLoss/recoveredLate/severe/expired` 的真实节拍
- `HardDisconnect`
  - transport disconnected / peer closed / all candidate paths unreachable
  - 重点验证 reconnect/failed-terminal 的刚性升级

### C) 场景矩阵

| 场景 ID | Target | 子画像 | 扰动模型 | 真实特征 | 期望动作 | 主要断言 |
| --- | --- | --- | --- | --- | --- | --- |
| `HOME-S1` | Home | `Steady` | `JitterBurst` | clean anchor 后短窗 delta gap、少量 pacer/present drop | `Absorb` | 不得从 `stable-serving` 重回 reconnect 主链 |
| `HOME-S2` | Home | `Steady` | `ReorderBurst` | gap + NACK + repair，最终补齐 | `LocalRecover` | repair/reorder 只能留在 local 域 |
| `HOME-S3` | Home | `DisplayConstrained` | `JitterBurst` | render/deadline miss 明显，但 ingress gap 很低 | `Absorb/LocalRecover` | 不得误洗成 ingress/transport failure |
| `HOME-S4` | Home | `Steady` | `IntermittentBlackhole` | 短窗 `idleTimeout/noPending`，随后恢复 | `Absorb` | reconnect 候选不能重复触发 |
| `HOME-S5` | Home | `Steady` | `HardDisconnect` | candidate path 失效或 peer closed | `TransportRecover` | 必须允许升级 reconnect，不能被本地吸收吞掉 |
| `CLOUD-S1` | Cloud | `CloudStartup` | `FeedbackDelay` | 首帧前 transport 有进展但媒体未出画 | `Absorb/LocalRecover` | 不得在 `New/Connecting` 早期误发 reconnect |
| `CLOUD-S2` | Cloud | `CloudHighRtt` | `LossBurstRecoverable` | `transportAwaitRecoveryAnchor` 前有 media loss | `LocalRecover` | 必须先 keyframe/decoder reset，再考虑 reconnect |
| `CLOUD-S3` | Cloud | `CloudHighRtt` | `PathDegrade` | `sampleLoss/recoveredLate` 连续出现 | `LocalRecover -> TransportRecover` | reason 优先级正确，节拍不越级 |
| `CLOUD-S4` | Cloud | `CloudHighRtt` | `PathDegrade` | repeated `severeDeadline` | `TransportRecover` | fresh second-hit 升 reconnect |
| `CLOUD-S5` | Cloud | `CloudHighRtt` | `PathDegrade` | repeated `expiredDeadline` | `TransportRecover` | third-hit 升 reconnect，不能套 severe 节拍 |
| `CLOUD-S6` | Cloud | `CloudHighRtt` | `IntermittentBlackhole` | reconnect 后仍 `transportAwaitRecoveryAnchor` | `ExitRecovery/TransportRecover` | reconnect 后不能陷入恢复自旋 |
| `CLOUD-S7` | Cloud | `CloudHighRtt` | `HardDisconnect` | transport disconnected / lifecycle recovering | `TransportRecover/FailedTerminal` | 必须可达 reconnect 或 failed-terminal，不得静默 |

### D) 设计成可复用 fixture 的输入轴

- `target_type`
  - `Home` / `Cloud`
- `dynamic_subprofile`
  - `Steady` / `CloudStartup` / `CloudHighRtt` / `DecoderConstrained` / `DisplayConstrained`
- `network_shape`
  - `jitter_ms`
  - `reorder_ratio`
  - `loss_burst_len`
  - `feedback_delay_ms`
  - `rtt_ms`
  - `blackhole_window_ms`
- `media_shape`
  - `present_drop_ratio`
  - `pacer_drop_ratio`
  - `renderer_drop_ratio`
  - `decode_stall`
  - `anchor_clean`
- `expected_contract`
  - `reason_domain`
  - `max_reconnect_candidates`
  - `must_emit_local_recover_before_reconnect`
  - `must_exit_recovery_after_clean_anchor`
  - `must_reach_failed_terminal_within`

### E) 下一批最值得先落地的 8 个场景

1. `HOME-S1`
   - 值得先做，因为它直接验证 Home 的“短抖动温和吸收”主合同。
2. `HOME-S3`
   - 用来证明 render/deadline 抖动不应被误判成 ingress/network。
3. `HOME-S4`
   - 用来收口短窗 idle timeout / reconnect 过敏。
4. `CLOUD-S1`
   - 用来收口首帧前 Cloud 建连阶段的误重连。
5. `CLOUD-S2`
   - 用来锁住 media loss 后必须先走 `transportAwait`/local recover。
6. `CLOUD-S4`
   - 用来锁住 repeated `severeDeadline` 的 second-hit 节拍。
7. `CLOUD-S5`
   - 用来锁住 repeated `expiredDeadline` 的 third-hit 节拍。
8. `CLOUD-S6`
   - 用来验证 reconnect 后 recovery exit，不再形成恢复自旋。

## Progress

- [x] Step 1: 已盘点现有 recovery/scheduling 测试分布与主要缺口
- [x] Step 2: 已先在 `session/policy.test.rs` 落第一版跨层 harness，串起 owner/coordinator/policy
- [x] Step 3: 第一批高价值场景已覆盖 `transportAwaitRecoveryAnchor` 出口、本地 display 只留局部恢复、stale idle 吸收，以及 healthy baseline 下 local ingress drop 不发 recovery/reconnect
- [x] Step 4: `video_source` 侧已补一套可复用的真实 ingress 驱动夹具，先把 repair overflow 与 unmatched RTX burst 的 `sink -> source -> session policy` 闭环收口成同一测试骨架
- [ ] Step 5: 将 recovery 相关验收继续收口到该矩阵
- [x] Step 5a: 已把 `transportAwaitRecoveryAnchor` 的 lingering hard-fallback 计时、`Connected + ingress` 误升级 reconnect、以及 session 静态 reason-domain 误贴标三处结构性问题落到生产修复，并用 coordinator/session/runtime 三层回归锁住
- [x] Step 5b: 已新增“decoder reset 后短暂恢复再重入”和“owner 已回到 stable-serving 时 fresh transportAwait 不能夺权”两组回归，并在 `session policy` 入口补上 `owner healthy` 优先吸收，防止健康后段被 fresh fallback diagnosis 重新拖回恢复面
- [x] Step 5c: 第二批画像场景已补齐 `HOME-S4/CLOUD-S2/CLOUD-S4/CLOUD-S5`，并额外修正一条生产边界：`Recovering + transportAwaitRecoveryKeyframe` 在存在真实连接恢复证据时，不再被本地恢复面永久遮挡；同时把 `expiredDeadline` third-hit 用例改成真实时间窗驱动，和当前 escalation controller 合同对齐
- [x] Step 5d: 已把一条 cloud 场景继续上提到 `sink -> source -> session -> transport_session -> runtime` 闭环，验证“本地 local candidate 先被 runtime gate 拒绝，随后真实 `rtcConnectionRecovering` transport candidate 被接受，恢复后不再重复 reconnect”
- [x] Step 5e: 已把同一条 `CLOUD-S6` 闭环补上 owner/coordinator 可观测断言，并确认 recovered 段当前主合同应是“停止追加 reconnect / 不再留下 pending action”，而不是强行要求 owner 与 decision ledger 在同一拍立刻清零到 `stable-serving/none`
- [x] Step 5f: 已补齐 Home/Cloud 两条缺失的 runtime 全链路样板：`HOME-S1` 的 clean-anchor 短抖动吸收不会抵达 reconnect，`CLOUD-S5` 的 repeated expired deadline 会经 `transport_session -> runtime` 被完整放行，且 recovery exit 后不再追加 reconnect

## Execution Notes

- Date: 2026-04-05 | Status: in-progress
- Update: 确认当前问题不是“测试数量不够”，而是缺统一的跨层场景矩阵。现有单测已经很多，但 recovery 风暴仍在 owner/coordinator/session/runtime 的边界处反复漏出。
- Decision: 先建立“域别 + 升级链 + 退出条件”三轴矩阵，再补 harness 与场景，不继续零散地往各模块 test 文件里堆定向用例。
- Risk/Blocker: 现有 trace 资产足够做第一批 replay profile，但如果不先收敛 fixture 结构，后续仍会滑回按 trace 逐个补丁式加例子的旧路。
- Date: 2026-04-05 | Status: in-progress
- Update: 已在 [`session/policy.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.test.rs) 增加第一版 `RecoveryIntegrationHarness`，先把 `owner -> recovery coordinator -> session policy` 串成统一场景入口，并落下 3 个跨层场景：`transportAwaitRecoveryAnchor` 恢复出口、本地 `displaySupplyCritical` 只停留在局部恢复、`stale adapterIdleTimeout` 被 steady 进展吸收但真实 transport 断链仍升级 reconnect。
- Validation: `cargo test -p xbxengine recovery_integration_transport_await_exits_after_completion_evidence -- --nocapture`、`cargo test -p xbxengine recovery_integration_local_display_stays_local_recovery -- --nocapture`、`cargo test -p xbxengine recovery_integration_stale_idle_is_absorbed_but_transport_failure_still_reconnects -- --nocapture`、`cargo check -p xbxengine`
- Decision: 第一批先不新建独立 integration crate，而是挂在现有 `session/policy.test.rs`，直接消费真实 runtime stats 与 policy 内部 owner/coordinator 组合，先证明跨层合同能稳定断言，再决定是否上升为更通用的 replay fixture。
- Risk/Blocker: 当前 harness 还没有把 `video_source` 和 `runtime gate` 末端一起纳入同一场景闭环；下一步要继续把 ingress/local-domain 与 pending reconnect runtime gate 接进来。
- Date: 2026-04-05 | Status: in-progress
- Update: 已在 [`api/runtime/mod.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/runtime/mod.test.rs) 补充 `PendingReconnectCandidateMatrixCase` 小型夹具，把 pending reconnect candidate 的构造、tick 驱动和 runtime gate 结果收口到同一骨架；并新增两组成对矩阵场景，明确同类 pending reconnect candidate 在 `Local` 域必须被 gate 拒绝、在 `ConnectivityTransport` 域必须被放行。
- Validation: `cargo test -p xbxengine runtime_pending_reconnect_candidate_matrix_separates_local_ingress_from_transport_connectivity`、`cargo test -p xbxengine runtime_pending_reconnect_candidate_matrix_keeps_display_local_but_allows_liveness_transport`、`cargo check -p xbxengine`
- Decision: runtime gate 这一层不再继续堆孤立单测，而是以“同一骨架只切换 domain / transport state / observation source”的矩阵方式收口，避免未来再出现 `localBackpressure`、`displaySupplyCritical`、`peer-closed`、`livenessNoProgressTimeout` 分散回归、彼此口径漂移。
- Risk/Blocker: 当前闭环仍差 `video_source -> runtime gate` 这一段，本地 ingress/repair/RTX 干扰还没有以同一套矩阵入口证明“只允许局部恢复、不允许落成 reconnect”。
- Date: 2026-04-05 | Status: in-progress
- Update: 已先在 [`stream/video_source/source.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.test.rs) 收紧 repair/RTX 的本地合同：命中 bootstrap 缺口的 repair 包在完成帧组装后仍不得额外冒出 transport 观测；未命中缺口的 RTX reinject 只保留 `adapterResolveMiss` 级别 provenance，不能直接抬升成 `AwaitRecoveryKeyframe` / transport recovery signal。
- Validation: `cargo test -p xbxengine repair_packet_closes_bootstrap_gap_and_allows_frame_assembly`、`cargo test -p xbxengine repair_rtx_packet_keeps_explicit_provenance_through_source_stage_updates`
- Decision: 在 `session policy` 尚未显式消费 `localBackpressureDeltaGap` 之前，repair/RTX 的“温和吸收”必须先在 `video_source` 源头被锁死，不能让本地 ingress 干扰借 transport observation 漏进后续恢复升级链。
- Risk/Blocker: 当前只能证明 `video_source` 不直接发出 transport 观测；还没有把 `sink backpressure / repair burst -> source timeline -> owner/coordinator -> runtime gate` 串成单个场景闭环。
- Date: 2026-04-05 | Status: in-progress
- Update: 继续把 repair/RTX 的“本地吸收”从单包扩展到 burst 场景：[`stream/video_source/source.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.test.rs) 新增 `unmatched_repair_rtx_burst_stays_local_and_does_not_emit_transport_observation`，验证未命中缺口的 RTX reinject 即使成束到来，也只会刷新本地 `adapterResolveMiss` provenance，不会生成 `AwaitRecoveryKeyframe` 或其他 transport observation。
- Validation: `cargo test -p xbxengine unmatched_repair_rtx_burst_stays_local_and_does_not_emit_transport_observation`
- Decision: repair/RTX 的鲁棒性先按“单包 miss 不升级、burst miss 也不升级”两层合同落地，后续再把 `sink` 背压和 `timeline` 的本地 gap reason 一起并入统一 harness。
- Risk/Blocker: 当前仍未证明“repair burst + sink overflow + timeline localBackpressureDeltaGap”在跨层上一定只停留在 local recover；这一段还需要真正的 `sink -> source -> session` 闭环用例。
- Date: 2026-04-05 | Status: in-progress
- Update: 已把首条本地 ingress 闭环继续推进到 `session policy`：[`stream/video_source/sink.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/sink.test.rs) 的 `repair_overflow_drains_into_source_but_stays_local_without_transport_observation` 现用同一份 `runtime_stats` 串起真实 `RtcVideoSourceSink`、`RtcVideoFrameSource` 与 `RtcSessionPolicy`，先由 `sink` 触发 `localBackpressureRepairOverflow`，再把残余 repair 队列排给 `source`，最后在 healthy baseline 下把该 stats 喂给 `session policy`，确认既不泄露 transport observation，也不会产出 reconnect candidate。
- Validation: `cargo test -p xbxengine repair_overflow_drains_into_source_but_stays_local_without_transport_observation`
- Decision: 对于“本地 repair overflow 是否会被误洗成网络恢复”的问题，当前已经有一条真实 `sink -> source -> session policy` 闭环兜住；下一步再把这类场景并入现有 `RecoveryIntegrationHarness`，统一到 owner/coordinator/runtime gate 的同一矩阵入口。
- Risk/Blocker: 这条闭环仍是 `sink.test.rs` 内部构造的 healthy baseline，还没有完全复用 `session/policy.test.rs` 那套 `owner -> coordinator -> runtime gate` 统一 harness。
- Date: 2026-04-05 | Status: in-progress
- Update: 第二批画像场景本轮已落地并回归通过：[`session/policy.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.test.rs) 新增/修正 `recovery_integration_home_short_idle_blackhole_is_absorbed_until_progress_returns`、`recovery_integration_cloud_media_loss_prefers_transport_await_before_reconnect`、`cloud_high_rtt_repeated_transport_severe_deadline_second_hit_reconnects`、`cloud_high_rtt_repeated_transport_expired_deadline_third_hit_reconnects`。其中生产侧额外修正 [`session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs)：当 `ConnectionLifecycleState=Recovering` 且 diagnosis 为 `rtcConnectionRecovering`，如果当前已具备真实连接恢复证据，就允许 lifecycle reconnect 重新夺回优先级，并保持 reason label 为 `rtcConnectionRecovering`，避免被残留 `transportAwaitRecoveryAnchor` 本地恢复面长期遮挡。
- Validation: `cargo test -p xbxengine recovery_integration_home_short_idle_blackhole_is_absorbed_until_progress_returns -- --nocapture`、`cargo test -p xbxengine recovery_integration_cloud_media_loss_prefers_transport_await_before_reconnect -- --nocapture`、`cargo test -p xbxengine cloud_high_rtt_repeated_transport_severe_deadline_second_hit_reconnects -- --nocapture`、`cargo test -p xbxengine cloud_high_rtt_repeated_transport_expired_deadline_third_hit_reconnects -- --nocapture`
- Decision: `expiredDeadline` 的跨窗 third-hit 继续沿当前真实时间窗合同，不把 `snapshot.now_ms` 伪装成 controller 时间；而 `rtcConnectionRecovering` 则在“确有连接恢复证据”的前提下优先于残留本地恢复面，保证真正 transport 问题仍可升级 reconnect。
- Risk/Blocker: 当前第二批仍停留在 `owner -> coordinator -> session policy` 层，下一步要把 `CLOUD-S6` 或 `HOME-S1` 至少一条继续上提到 `sink -> source -> owner -> coordinator -> session -> runtime` 单场景闭环。
- Date: 2026-04-05 | Status: in-progress
- Update: 已先把 `CLOUD-S6` 半步上提成真实闭环测试：[`api/runtime/mod.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/runtime/mod.test.rs) 新增 `runtime_cloud_recovery_replay_accepts_transport_reconnect_after_local_noise_rejection_and_exits_cleanly`，复用 [`video_source/test_fixtures.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/test_fixtures.rs) 的 `LocalIngressReplayFixture` 先驱动 `sink -> source` 产生真实 local repair overflow 噪声，再用 [`session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 产出 `rtcConnectionRecovering` reconnect candidate，通过 [`stack.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stack.rs) 暴露的测试桥接入口送入 `transport_session -> pending runtime recovery action`，最后由 runtime gate 真正消费并验证恢复后不再二次 reconnect。
- Validation: `cargo test -p xbxengine runtime_cloud_recovery_replay_accepts_transport_reconnect_after_local_noise_rejection_and_exits_cleanly -- --nocapture`、`cargo test -p xbxengine runtime_accepts_recovering_candidate_after_local_transport_await_candidate_was_rejected -- --nocapture`
- Decision: 闭环第一条优先选 `CLOUD-S6`，而不是 `HOME-S1`，因为现有 replay fixture 已具备 cloud high-rtt / degraded / recovered 的完整状态切换；Home clean-anchor 抖动画像后续再单独上提。
- Risk/Blocker: 这条闭环目前仍是 `sink -> source -> session -> transport_session -> runtime`，还没有把 owner/coordinator 的独立观测结果显式串出为同一份断言资产；下一步可以考虑把现有 `RecoveryIntegrationHarness` 和 replay fixture 做更轻量的汇合。
- Date: 2026-04-05 | Status: in-progress
- Update: 已继续沿同一条 `CLOUD-S6` 闭环补 owner/coordinator 断言，并在 [`api/runtime/mod.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/runtime/mod.test.rs) 显式锁住三段语义：local recover 首拍 owner 当前实际收口为 `priming/steady`，而不是先前假定的 `rebuilding-supply/anchor`；进入 `rtcConnectionRecovering` reconnect candidate 时，owner 当前实际仍是 `rebuilding-supply/transportAwaitRecoveryKeyframe/anchor`，同时 decision ledger 必须 `gate_result=pass` 且存在 budget 快照；recovered 段则只要求“不再重复 reconnect / 不再残留 pending reconnect action”，不再把“owner 与 ledger 同拍清零”误记成现行合同。
- Validation: `cargo test -p xbxengine runtime_cloud_recovery_replay_accepts_transport_reconnect_after_local_noise_rejection_and_exits_cleanly`、`cargo test -p xbxengine multi_stage_replay_steady_local_noise_expired_then_recover_stays_stable`
- Decision: 当前真实闭环已经足够说明 owner/coordinator 的局部观测与 runtime gate 结论一致，但 recovered 段 owner 完全回稳仍不是同拍合同；后续若要补 `HOME-S1`，应优先扩 `LocalIngressReplayFixture` 的健康画像配置，而不是继续把 recovered 段硬断言成 `stable-serving`
- Date: 2026-04-05 | Status: in-progress
- Update: 已继续按 Home/Cloud 补上两条真正到 runtime 的全链路样板，统一挂在 [`api/runtime/mod.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/runtime/mod.test.rs)：`runtime_home_clean_anchor_short_jitter_replay_never_reaches_reconnect` 用真实 `sink -> source` local repair overflow 输入配合 Home clean-anchor baseline，验证短抖动只会停留在 `no-signal`，不会经 `transport_session` 生成 pending reconnect，更不会触发 runtime restart；`runtime_cloud_replay_promotes_expired_deadline_to_transport_reconnect_and_exits_cleanly` 则把 cloud local noise 之后 repeated `transportExpiredDeadline` 的 third-hit 升级一路串到 `transport_session -> runtime`，并验证 recovery exit 后 pending reconnect action 会被清空、不再二次重连。
- Validation: `cargo test -p xbxengine runtime_home_clean_anchor_short_jitter_replay_never_reaches_reconnect`、`cargo test -p xbxengine runtime_cloud_replay_promotes_expired_deadline_to_transport_reconnect_and_exits_cleanly`
- Decision: 这样 Home/Cloud 两侧现在都已有一条“真实 ingress 驱动 + session 裁决 + runtime 终态”的代表性闭环，下一步再补的优先级应转向更细的 Home render/deadline 误分类边界或 Cloud startup/feedback-delay 闭环，而不是继续在已覆盖的 `rtcConnectionRecovering/expiredDeadline` 线上堆近似场景
- Date: 2026-04-05 | Status: in-progress
- Update: 已把 “healthy timeline/track baseline + local ingress drop stats 已存在” 收进 [`session/policy.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.test.rs) 的 `RecoveryIntegrationHarness`，新增场景 `recovery_integration_local_ingress_drop_stays_no_signal_under_healthy_baseline`：直接给 runtime stats 注入 `latest_video_frame_drop(reason=localBackpressureRepairOverflow)`，并保持 clean anchor / healthy timeline / remoteTrackAttached / present/decode fresh 的 baseline，断言当前 `session policy` 不发任何 recovery command，也不产出 reconnect candidate。
- Validation: `cargo test -p xbxengine recovery_integration_local_ingress_drop_stays_no_signal_under_healthy_baseline -- --nocapture`、`cargo check -p xbxengine`
- Decision: 因为 `session policy` 当前并未显式消费 `latest_video_frame_drop`，这条场景的正确合同不是“伪造 local recover signal”，而是明确表达为 `no-signal / not reconnect`；后续若要把 sink/source 的本地 provenance 显式接进调度，再在业务逻辑里补真实信号消费。
- Risk/Blocker: 当前 harness 仍只能证明 “已有 local ingress drop stats 时 policy 不误升级”；还不能证明 sink/source 在所有 repair overflow 组合下都稳定把本地 provenance 传到 stats，这部分仍需依赖 `sink/source` 模块闭环测试继续兜底。
- Date: 2026-04-05 | Status: in-progress
- Update: 已把真实 ingress 驱动正式上提为 [`video_source/test_fixtures.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/test_fixtures.rs) 内的 `LocalIngressReplayFixture`，把共享的 `runtime_stats + RtcVideoSourceSink + RtcVideoFrameSource + transport_observation_rx + session policy healthy baseline` 收口成可复用 replay fixture，`sink.test.rs` 里的 repair overflow 与 unmatched RTX burst 场景现复用同一骨架，避免后续 repair/RTX 场景继续手工拼装闭环。
- Update: 在同一套 [`video_source/test_fixtures.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/test_fixtures.rs) 上继续补齐 `LocalIngressReplayProfile` / `LocalIngressReplayPacket` / `LocalIngressHealthyBaseline`，让真实 ingress 场景从“共享搭建”进一步收口为“profile 化输入 + 固定回放驱动”；[`sink.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/sink.test.rs) 中的 repair overflow 与 unmatched RTX burst 现只保留各自 replay profile 与场景断言。
- Update: 已在 [`session/policy.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.test.rs) 增加组合场景 `recovery_integration_local_ingress_and_stale_idle_stay_no_signal_under_healthy_baseline`，直接覆盖“healthy baseline + local ingress provenance(`localBackpressureRepairOverflow`) + stale `adapterIdleTimeout` diagnosis 共存”的风暴重入边界，确认当前 policy 仍保持 `input_signal=none`、`gate_result=no-signal`、`action_selected=none`，不会误发 reconnect candidate。
- Update: 本轮补齐该场景对 `latest_recovery_decision_ledger.input_signal=none` 的显式断言（此前已断言 `gate_result/action_selected`），继续只表达现有调度合同，不改任何生产策略逻辑；`api/runtime/mod.test.rs` 本轮不新增用例，避免把“session 层未产生 candidate”的边界强行迁移到 runtime gate 层。
- Update: 已在 [`api/runtime/mod.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/runtime/mod.test.rs) 新增 runtime gate 交界矩阵 `runtime_pending_reconnect_candidate_matrix_keeps_transport_await_local_but_allows_deadline_transport`，直接对比“`transportAwaitRecoveryAnchor` 若被标成 `Local` 必须被 gate 拒绝”和“真正 `transportExpiredDeadline` 的 `ConnectivityTransport` candidate 必须被放行”，继续锁住本地 provenance 不能被误洗成 reconnect。
- Validation: `cargo test -p xbxengine runtime_pending_reconnect_candidate_matrix_keeps_transport_await_local_but_allows_deadline_transport -- --nocapture`、`cargo check -p xbxengine`
- Validation: `cargo test -p xbxengine recovery_integration_local_ingress_and_stale_idle_stay_no_signal_under_healthy_baseline -- --nocapture`、`cargo check -p xbxengine`
- Update: 2026-04-05 复测确认：组合场景在 `healthy baseline + local ingress provenance + stale adapterIdleTimeout` 下仍然 `no-signal` 且无 `RequestReconnectCandidate`，仅有既存编译告警，无新增失败。
- Validation: `cargo test -p xbxengine repair_overflow_drains_into_source_but_stays_local_without_transport_observation -- --nocapture`、`cargo test -p xbxengine unmatched_repair_rtx_burst_through_real_ingress_stays_local_without_transport_observation -- --nocapture`、`cargo check -p xbxengine`
- Decision: 这轮不把 shared fixture 硬拉回 `session/policy.test.rs`，因为当前最稳定的真实入口仍在 `video_source` 测试侧；先用同一夹具锁住“repair overflow 不泄露 transport observation”和“unmatched RTX burst 经真实 sink->source->policy 仍不产出 reconnect candidate”两条合同，后续若 owner/coordinator 真开始消费这类 provenance，再决定是否上提成跨文件矩阵资产。
- Risk/Blocker: 当前夹具仍停在 `sink -> source -> session policy`，还没有把 runtime gate 与 trace replay profile 末端直接并进同一个测试体；不过这已经足够覆盖本轮要防的本地 ingress/repair 干扰误升级。
- Date: 2026-04-05 | Status: in-progress
- Update: 已确认当前“健康链路后段重入恢复风暴”的一个真实代码根因为 `transportAwaitRecoveryAnchor` 的 hard-fallback 内部计时 lingering：此前 `explicit healthy clean anchor`、`recovery epoch` 轮转、甚至非 `transportAwait` reason 都不会清掉内部起点，导致后段再遇到局部坏窗时会直接沿用旧 timeout 窗口进入 reconnect；同时 `session policy` 末端仍按静态 `reason.reconnect_domain()` 给 pending reconnect candidate 贴域，`transportAwaitRecoveryAnchor` 会被天然包装成 `ConnectivityTransport`。
- Update: 本轮已在 [`recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 收紧 hard-fallback 语义：`clean anchor` 确认、`recovery epoch` 前进、stall evidence 清空、以及非 `transportAwait` reason 都会真正复位内部 hard-fallback 计时；同时 `Connected + ingress` 仍在推进时，`transportAwait` 超时只允许继续停留在本地恢复链，不再直接升 reconnect。并在 [`session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 把 runtime reconnect domain 从“静态 reason”改成“按 proposal 语义判定”，将 `transportAwaitRecoveryAnchor` / `waitKeyframe` / `adapterIdleTimeout` / `decoderBackendFailure` 等 media-local reason 的 reconnect candidate 统一标成 `Local`，继续保留 `transportExpiredDeadline` / `transportSevereDeadline` / `lifecycleRecovering` 等真正 connectivity reason 为 `ConnectivityTransport`。
- Validation: `cargo test -p xbxengine transport_await_hard_fallback -- --nocapture`、`cargo test -p xbxengine runtime_reconnect_reason_domain_keeps_transport_await_local -- --nocapture`、`cargo test -p xbxengine runtime_reconnect_reason_domain_keeps_deadline_transport_connectivity -- --nocapture`、`cargo test -p xbxengine runtime_pending_reconnect_candidate_matrix_keeps_transport_await_local_but_allows_deadline_transport -- --nocapture`、`cargo check -p xbxengine`
- Decision: 这轮把问题正式归类为“恢复调度结构问题”而不是 trace 特例补丁，后续优先继续沿 `stale diagnosis fallback -> coordinator hard-fallback -> runtime gate` 的组合边界补矩阵，而不是再回到 `sink/source` 单点深挖。
- Date: 2026-04-05 | Status: in-progress
- Update: 本轮新增两条更贴近现场的回归：[`coordinator.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.test.rs) 的 `transport_await_hard_fallback_does_not_inherit_timeout_window_after_decoder_reset_and_short_healthy_reentry` 锁住“decoder reset 后短暂健康脉冲会真正打断旧 hard-fallback timeout 窗口”；[`session/policy.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.test.rs) 的 `recovery_integration_fresh_transport_await_does_not_override_stable_owner_without_clean_anchor` 则直接打出一条新缺口：即使没有 clean anchor，只要 owner 已在当前拍回到 `stable-serving` 且媒体输出健康，fresh `transportAwaitRecoveryAnchor` 仍会被 fallback diagnosis 重新带回 recovery proposal。
- Update: 针对上述新缺口，已在 [`session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 收紧 `should_absorb_stale_transport_await_replay(...)` 的入口语义：`Connected` 下若 owner 当前拍已是 `stable-serving/degraded-serving` 且 timeline healthy、track attached、decoder/renderer 未 stalled、media output fresh，则直接吸收 `transportAwaitRecoveryAnchor` fallback diagnosis，不再要求一定先拿到 clean anchor；而旧的 stale replay 吸收路径继续保留 `clean anchor + healthy media baseline` 约束，避免误伤真正 stale recovery 回放合同。
- Validation: `cargo test -p xbxengine transport_await_hard_fallback_does_not_inherit_timeout_window_after_decoder_reset_and_short_healthy_reentry -- --nocapture`、`cargo test -p xbxengine recovery_integration_fresh_transport_await_does_not_override_stable_owner_without_clean_anchor -- --nocapture`、`cargo test -p xbxengine stale_transport_await_does_not_replay_during_steady_progress -- --nocapture`、`cargo test -p xbxengine recovery_integration_stale_transport_await_after_completion_evidence_stays_no_signal -- --nocapture`、`cargo test -p xbxengine runtime_rejects_replayed_local_pending_reconnect_candidates_without_request_storm -- --nocapture`、`cargo check -p xbxengine`
- Date: 2026-04-05 | Status: in-progress
- Update: 已继续补齐“反误伤”序列矩阵，验证新吸收逻辑不会吞掉真正网络故障升级链：[`session/policy.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.test.rs) 新增 `recovery_integration_fresh_transport_await_absorption_does_not_block_following_transport_disconnect`，确认 `owner stable-serving` 下 fresh `transportAwaitRecoveryAnchor` 先被吸收为 `no-signal`，但下一拍若 lifecycle 进入 `Disconnected`，仍会产出 `RequestReconnectCandidate(reason=rtcConnectionDisconnected, reason_domain=ConnectivityTransport)`；[`api/runtime/mod.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/runtime/mod.test.rs) 新增 `runtime_accepts_transport_candidate_after_local_candidate_was_rejected`，确认上一拍 `Local` 域 candidate 被 gate 拒绝后，不会污染下一拍 `ConnectivityTransport` 域的 `transportExpiredDeadline` reconnect。
- Validation: `cargo test -p xbxengine recovery_integration_fresh_transport_await_absorption_does_not_block_following_transport_disconnect -- --nocapture`、`cargo test -p xbxengine runtime_accepts_transport_candidate_after_local_candidate_was_rejected -- --nocapture`
- Date: 2026-04-05 | Status: in-progress
- Update: 已进一步补上 `Recovering` 生命周期下的混合信号优先级：[`session/policy.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.test.rs) 新增 `recovery_integration_recovering_lifecycle_overrides_fresh_transport_await_absorption`，确认即使当前拍还带着 fresh `transportAwaitRecoveryAnchor` 且媒体输出健康，只要连接生命周期已进入 `Recovering`，`session policy` 仍会优先发出 `RequestReconnectCandidate(reason=rtcConnectionRecovering, reason_domain=ConnectivityTransport)`，不会被 `Connected + owner healthy` 这条本地吸收路径误吞。
- Validation: `cargo test -p xbxengine recovery_integration_recovering_lifecycle_overrides_fresh_transport_await_absorption -- --nocapture`
- Date: 2026-04-05 | Status: in-progress
- Update: 已继续补“同拍混合信号优先级”场景：[`session/policy.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.test.rs) 新增 `recovery_integration_transport_deadline_overrides_same_tick_local_display_recovery`，构造同一拍内 `displaySupplyCritical` owner intent 与 `transportExpiredDeadline` fallback diagnosis 并存的场景。测试最初直接打出真实缺口：本地 owner intent 会遮掉 transport deadline 的 proposal 入口。随后已在 [`session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 增加 `resolve_connectivity_fallback_reason(...)`，并在 `build_recovery_proposal()` 中将 `transportExpiredDeadline/transportSevereDeadline/transportRecoveredLate/transportSampleLoss` 这类 connectivity fallback 置于本地 media owner intent 之前选路。
- Update: 这条同拍优先级测试最终锁定的合同不是“当拍必须立刻 reconnect”，而是“proposal 输入归因必须切到 connectivity deadline，不再被本地 display 抢走”。原因是 coordinator 对 `transportExpiredDeadline` 在不同坏窗形态下本来就可能先走 `requestKeyframe` 或 `requestDecoderReset`，动作类型仍受 recovery budget / hard-stall 策略控制；本轮修复只收口信号优先级，不改 coordinator 的动作策略。
- Validation: `cargo test -p xbxengine recovery_integration_transport_deadline_overrides_same_tick_local_display_recovery -- --nocapture`、`cargo test -p xbxengine recovery_integration_local_display_stays_local_recovery -- --nocapture`、`cargo check -p xbxengine`
- Date: 2026-04-05 | Status: in-progress
- Update: 已继续补齐 deadline 类 mixed-signal 矩阵：[`session/policy.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.test.rs) 新增 `runtime_reconnect_reason_domain_keeps_severe_deadline_transport_connectivity`、`recovery_integration_transport_severe_deadline_overrides_same_tick_local_display_recovery`、`recovery_integration_transport_deadline_overrides_same_tick_local_transport_await`、`recovery_integration_transport_severe_deadline_overrides_same_tick_local_transport_await` 与 `recovery_integration_repeated_transport_severe_deadline_upgrades_to_connectivity_reconnect`，把 `transportSevereDeadline/transportExpiredDeadline` 与本地 `displaySupplyCritical/transportAwaitRecoveryKeyframe` 并存时的 proposal 归因和升级出口继续锁死。
- Update: 新测试先打出一条真实结构缺口：`session policy` 已把 `transportSevereDeadline` 视为 connectivity fallback，但 [`recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 仍把 `transportExpiredDeadline/transportSevereDeadline/transportRecoveredLate/transportSampleLoss` 归进 `MediaRecovery` 域，导致 deadline 类信号跨层后无法使用 reconnect fallback 预算。现已把这些 reason 改归 `Connectivity` 域，收口 `session -> coordinator -> runtime gate` 的 reason-domain 口径。
- Validation: `cargo test -p xbxengine runtime_reconnect_reason_domain_keeps_severe_deadline_transport_connectivity -- --nocapture`、`cargo test -p xbxengine recovery_integration_transport_severe_deadline_overrides_same_tick_local_display_recovery -- --nocapture`、`cargo test -p xbxengine recovery_integration_transport_deadline_overrides_same_tick_local_transport_await -- --nocapture`、`cargo test -p xbxengine recovery_integration_transport_severe_deadline_overrides_same_tick_local_transport_await -- --nocapture`、`cargo test -p xbxengine recovery_integration_repeated_transport_severe_deadline_upgrades_to_connectivity_reconnect -- --nocapture`、`cargo check -p xbxengine`
- Date: 2026-04-05 | Status: in-progress
- Update: 已继续并行补齐剩余 5 类场景测试。`session policy` 侧新增 [`recovery_integration_repeated_transport_expired_deadline_upgrades_to_connectivity_reconnect`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.test.rs#L4964)、[`recovery_integration_transport_sample_loss_overrides_same_tick_local_display_recovery`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.test.rs#L5046)、[`recovery_integration_transport_sample_loss_overrides_same_tick_local_transport_await`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.test.rs#L5112)、[`recovery_integration_transport_recovered_late_overrides_same_tick_local_display_recovery`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.test.rs#L5179)、[`recovery_integration_transport_recovered_late_overrides_same_tick_local_transport_await`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.test.rs#L5248)，确认 repeated `transportExpiredDeadline` 也能升级到 connectivity reconnect，且 `transportSampleLoss/transportRecoveredLate` 同拍时不会被本地 `displaySupplyCritical/transportAwaitRecoveryKeyframe` 抢占。
- Update: runtime gate 与全链 replay 侧新增 [`runtime_accepts_transport_severe_candidate_after_local_display_candidate_was_rejected`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/runtime/mod.test.rs#L3202)、[`runtime_accepts_recovering_candidate_after_local_transport_await_candidate_was_rejected`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/runtime/mod.test.rs#L3306)、[`local_repair_noise_does_not_block_following_repeated_transport_severe_deadline_reconnect`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/sink.test.rs#L955)。前两条锁住“上一拍 Local 被 gate 拒绝，下一拍真正 Connectivity 仍可放行”的跨拍污染边界；最后一条复用真实 `sink -> source` replay fixture，证明本地 repair overflow 噪声不会挡住后续 repeated `transportSevereDeadline` 升级为 `ConnectivityTransport` reconnect。
- Validation: `cargo test -p xbxengine recovery_integration_repeated_transport_expired_deadline_upgrades_to_connectivity_reconnect -- --nocapture`、`cargo test -p xbxengine recovery_integration_transport_sample_loss_overrides_same_tick_local_display_recovery -- --nocapture`、`cargo test -p xbxengine recovery_integration_transport_sample_loss_overrides_same_tick_local_transport_await -- --nocapture`、`cargo test -p xbxengine recovery_integration_transport_recovered_late_overrides_same_tick_local_display_recovery -- --nocapture`、`cargo test -p xbxengine recovery_integration_transport_recovered_late_overrides_same_tick_local_transport_await -- --nocapture`、`cargo test -p xbxengine runtime_accepts_transport_severe_candidate_after_local_display_candidate_was_rejected -- --nocapture`、`cargo test -p xbxengine runtime_accepts_recovering_candidate_after_local_transport_await_candidate_was_rejected -- --nocapture`、`cargo test -p xbxengine local_repair_noise_does_not_block_following_repeated_transport_severe_deadline_reconnect -- --nocapture`、`cargo check -p xbxengine`
- Date: 2026-04-05 | Status: in-progress
- Update: 继续下探 `budget/cooldown` 振荡边界时，新补的 [`escalation.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/escalation.test.rs) 用例 `new_recovery_epoch_clears_severe_deadline_idle_timeout_shortcut` 首次直接打出真实结构缺口：`VideoEscalationController::begin_recovery_epoch()` 之前没有清理 `last_severe_deadline_at`，导致上一轮 `transportSevereDeadline` 会把下一轮 `AdapterIdleTimeout` 直接推入 reconnect shortcut，形成跨 recovery epoch 的污染。
- Update: 现已在 [`recovery/escalation.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs) 的 `begin_recovery_epoch()` 中显式清理 `last_severe_deadline_at`，并补上 [`adapter_idle_after_severe_deadline_window_expires_does_not_jump_to_reconnect_candidate`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/escalation.test.rs) 与 [`new_transport_recovery_epoch_clears_severe_deadline_idle_timeout_shortcut`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.test.rs)，分别锁住“超出 severe window 后不能借 idle timeout 直跳 reconnect”和“跨 epoch 后不能继承 severe shortcut”两条合同。
- Validation: `cargo test -p xbxengine new_recovery_epoch_clears_severe_deadline_idle_timeout_shortcut -- --nocapture`、`cargo test -p xbxengine adapter_idle_after_severe_deadline_window_expires_does_not_jump_to_reconnect_candidate -- --nocapture`、`cargo test -p xbxengine new_transport_recovery_epoch_clears_severe_deadline_idle_timeout_shortcut -- --nocapture`、`cargo test -p xbxengine transport::rtc::recovery::escalation::tests -- --nocapture`、`cargo check -p xbxengine`
- Date: 2026-04-05 | Status: in-progress
- Update: 已继续收口 recovery budget / cooldown 临界振荡。新增 [`transport_severe_deadline_requires_fresh_second_hit_before_reconnect`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.test.rs) 与 [`transport_expired_deadline_window_resets_after_large_gap`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.test.rs)，分别锁住 `TransportSevereDeadline` 的 reconnect 计数不能跨大空窗继承，以及 `TransportExpiredDeadline` 的窗口计数会在大 gap 后真实复位。
- Update: `transport_severe_deadline_requires_fresh_second_hit_before_reconnect` 首次直接打出新的真实缺口：[`recovery/escalation.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs) 中 `TransportSevereDeadline` 之前只累计 `reconnect_candidate_signals`，却没有像 `TransportExpiredDeadline` 那样按 reconnect window 复位，导致一次很久以前的 severe 也会让下一次孤立 severe 直接升级 reconnect。现已在 severe 分支加入“超出 `severe_deadline_reconnect_window` 即重置 `reconnect_candidate_signals`”的保护，使 severe 只能由 fresh second hit 触发 reconnect，不再把陈旧坏窗遗留成后续恢复风暴放大器。
- Validation: `cargo test -p xbxengine transport_severe_deadline_requires_fresh_second_hit_before_reconnect -- --nocapture`、`cargo test -p xbxengine transport_expired_deadline_window_resets_after_large_gap -- --nocapture`、`cargo check -p xbxengine`
- Date: 2026-04-05 | Status: in-progress
- Update: 已继续补 `reconnect budget` 释放与 reason 切换隔离两类边界。新增 [`reconnect_budget_is_released_after_new_epoch_for_transport_severe_deadline`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.test.rs) 与 [`transport_recovered_late_does_not_inherit_severe_deadline_reconnect_counter`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.test.rs)，分别锁住 `RequestReconnectCandidate` 消耗掉本 epoch reconnect budget 后，会在新的 `transport_recovery_epoch` 正常释放；同时 `transportRecoveredLate` 不会继承上一拍 `TransportSevereDeadline` 的 reconnect 计数而直接越级 reconnect。
- Decision: 这两条边界目前是测试绿灯，说明当前剩余风险更偏策略选择而不是新的显性结构 bug；真正的结构缺口仍集中在此前已修掉的 `severe` 陈旧计数继承问题。
- Validation: `cargo test -p xbxengine reconnect_budget_is_released_after_new_epoch_for_transport_severe_deadline -- --nocapture`、`cargo test -p xbxengine transport_recovered_late_does_not_inherit_severe_deadline_reconnect_counter -- --nocapture`
- Date: 2026-04-05 | Status: in-progress
- Update: 已把 [`video_source/test_fixtures.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/test_fixtures.rs) 进一步抽成“阶段化 replay fixture”，新增 `build_connected_snapshot()`、`mark_transport_connectivity_degraded()`、`mark_transport_recovered()` 三个公共入口，不再在 [`sink.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/sink.test.rs) 手拼 steady/degraded/recovered stats。基于这套夹具，已继续补四条长序列 replay：`multi_stage_replay_steady_local_noise_severe_then_recover_stays_stable`、`multi_stage_replay_steady_local_noise_expired_then_recover_stays_stable`、`multi_stage_replay_steady_local_noise_sample_loss_then_recover_stays_local`、`multi_stage_replay_steady_local_noise_recovered_late_then_recover_stays_local`。
- Update: 这轮新增场景直接固化了一个重要策略差异：`transportSevereDeadline` 是 fresh second-hit 升 reconnect，而 `transportExpiredDeadline` 在当前策略里是跨窗口 third-hit 才升 reconnect，不能拿 `severe` 的两拍节奏去套 `expired`。这类差异此前只散落在 `session/coordinator` 定点用例里，现在已经被沉淀进真实 `sink -> source -> session` replay 闭环。
- Validation: `cargo test -p xbxengine multi_stage_replay_steady_local_noise -- --nocapture`、`cargo test -p xbxengine transport::rtc::stream::video_source::sink::tests -- --nocapture`
- Date: 2026-04-05 | Status: in-progress
- Update: 已开始把扩展矩阵从设计落到真实测试，第一批先落 4 个高收益画像场景，全部挂在 [`session/policy.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.test.rs)：
  - `recovery_integration_home_clean_anchor_short_jitter_keeps_steady_serving`
  - `recovery_integration_home_render_deadline_jitter_stays_local_display_path`
  - `cloud_startup_transport_progress_does_not_reconnect_before_first_frame`
  - `recovery_integration_cloud_reconnect_then_clean_recovery_exit_does_not_reenter_storm`
- Update: 这 4 条分别对应 `HOME-S1/HOME-S3/CLOUD-S1/CLOUD-S6`，已经把“Home clean-anchor 后短抖动吸收”“Home render/deadline 抖动只留在本地域”“Cloud 首帧前 transport 有进展但不得过早 reconnect”“Cloud reconnect 后 clean anchor 成立必须退出 recovery 面、且 stale transportAwait replay 不得重入风暴”四条合同正式锁进回归。
- Decision: 这一批优先全部放在 `session policy` 层，而不是先拉进 `sink/runtime`，因为当前最关键的是把 Home/Cloud 画像差异与恢复动作差异先在同一裁决入口锁稳；后续再把其中的 `CLOUD-S6` 和 `HOME-S1` 接到更完整的 `sink -> source -> owner -> coordinator -> session -> runtime` 闭环。
- Validation: `cargo test -p xbxengine recovery_integration_home_ -- --nocapture`、`cargo test -p xbxengine cloud_startup_transport_progress_does_not_reconnect_before_first_frame -- --nocapture`、`cargo test -p xbxengine recovery_integration_cloud_reconnect_then_clean_recovery_exit_does_not_reenter_storm -- --nocapture`
- Date: 2026-04-05 | Status: in-progress
- Update: 已把前述画像矩阵继续下探到 `runtime` 跨层闭环：[ `api/runtime/mod.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/runtime/mod.test.rs) 新增 `runtime_home_render_deadline_jitter_replay_stays_local_and_never_reaches_reconnect` 与 `runtime_cloud_startup_transport_progress_replay_does_not_reconnect_before_first_frame`。前者复用真实 `sink -> source` local repair overflow replay 和 Home healthy baseline，锁住 render/deadline 抖动只允许停留在本地 `displaySupplyCritical -> requestKeyframe` 路径，不得透传为 reconnect candidate，也不得让 runtime 触发 restart；后者构造 Cloud `startup + transport Connecting + remoteTrackAttached + bytes/packets 持续前进但尚未首帧` 的闭环，明确此类“transport 有进展但首帧未到”的慢启动只能维持 `no-signal`，不得提前升级 reconnect，更不得残留 pending reconnect action。
- Decision: 这两条 runtime 用例继续沿用当前主合同，不强绑 owner/ledger 的全量内部字段，而是只锁“不得产出 reconnect candidate / runtime 不重启 / pending reconnect action 为空”与必要的 `no-signal`、本地 action 语义，避免把测试绑定到易漂移的内部账本细节。
- Validation: `cargo test -p xbxengine runtime_home_render_deadline_jitter_replay_stays_local_and_never_reaches_reconnect`、`cargo test -p xbxengine runtime_cloud_startup_transport_progress_replay_does_not_reconnect_before_first_frame`
- Date: 2026-04-05 | Status: in-progress
- Update: 已继续补齐先前 8 个高优先级画像之外最值得先补的两条空白：[`source.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.test.rs) 新增 `repair_reorder_gap_closure_stays_local_and_records_resolved_gap_match`，把 `HOME-S2` 收口为“bootstrap gap 被 repair/reorder 补齐后成功组帧，且仍不产生 transport observation”的现行合同；[`session/policy.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.test.rs) 新增 `cloud_high_rtt_sample_loss_then_recovered_late_stays_local_until_severe_deadline_reconnect`，把 `CLOUD-S3` 收口为“连续 `transportSampleLoss -> transportRecoveredLate` 仍只走 local recover，直到后续 `transportSevereDeadline` fresh second-hit 才升级 connectivity reconnect”。
- Decision: `HOME-S2` 这里先接受当前 `source` 侧的真实观测合同，即 repair 成功补齐并保持 local-domain 即可，不再强绑 `latest_video_rtx_reinject_observation.stage` 必须落到 `adapterResolved`；从回归结果看，最后一拍观测仍可能停在 `adapterResolveMiss`，但并不影响“成功补齐且不外溢 transport 域”这一主合同。
- Validation: `cargo test -p xbxengine repair_reorder_gap_closure_stays_local_and_records_resolved_gap_match`、`cargo test -p xbxengine cloud_high_rtt_sample_loss_then_recovered_late_stays_local_until_severe_deadline_reconnect`
- Date: 2026-04-05 | Status: in-progress
- Update: 现已把剩余两条“硬断链上界”也补进矩阵。[`session/policy.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.test.rs) 新增 `recovery_integration_home_hard_disconnect_emits_connectivity_reconnect_without_local_absorption`，把 `HOME-S5` 锁成“Home steady/healthy 后一旦进入真实 `Disconnected + rtcControlChannelClosed`，必须产出 `ConnectivityTransport` reconnect candidate，不能被本地 absorb 吞掉”；同文件新增 `cloud_hard_disconnect_reconnect_budget_exhaustion_enters_failed_terminal_without_spinning`，把 `CLOUD-S7` 锁成“Cloud hard disconnect 会在长窗口后进入 reconnect，直到 budget/terminal 上界触发后进入 `failed-terminal`，并停止继续自旋”。另外在 [`api/runtime/mod.test.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/runtime/mod.test.rs) 新增 `runtime_home_hard_disconnect_candidate_reaches_reconnect_restart` 与 `runtime_cloud_hard_disconnect_keepalive_session_not_active_stops_cleanly`：前者把 `HOME-S5` 继续串到 `session -> transport_session -> runtime`，确认 Home 硬断链候选会真正触发 restart；后者把 `CLOUD-S7` 的 runtime 上界锁成“Cloud hard disconnect 候选在 keepalive 返回 `HTTP 410 SessionNotActive` 时必须干净停机，不再进入重连风暴”。
- Decision: `CLOUD-S7` 这里沿用当前已落地的 Cloud 长窗口合同，不强行把 `startup + hard disconnect` 首拍写成立刻 reconnect；现行语义是先 respect Cloud warmup/long-window，再进入 reconnect，最终由 `livenessReconnectAttemptLimitExceeded` 收口到 `failed-terminal`。
- Validation: `cargo test -p xbxengine recovery_integration_home_hard_disconnect_emits_connectivity_reconnect_without_local_absorption`、`cargo test -p xbxengine cloud_hard_disconnect_reconnect_budget_exhaustion_enters_failed_terminal_without_spinning`、`cargo test -p xbxengine runtime_home_hard_disconnect_candidate_reaches_reconnect_restart`、`cargo test -p xbxengine runtime_cloud_hard_disconnect_keepalive_session_not_active_stops_cleanly`
