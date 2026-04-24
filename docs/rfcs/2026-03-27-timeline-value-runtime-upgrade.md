# 时间线价值运行模式完整升级 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- [docs/mode.md](/Users/guo.xu/Documents/code/games/xbxrc/docs/mode.md) 已经明确提出目标运行模式：系统真正管理的是“时间线价值”，总约束是 `reference chain > frame > packet`，NACK 只是恢复投资工具，而不是默认动作。
- 当前 `xbxengine` 已经有部分能力接近该目标，包括 Cloud 高 RTT 下的 `latency-first` NACK admission、`wait-keyframe`、`frame unrecoverable`、decode 前放弃与 latest-slot render，但整体仍是分散实现，尚未形成 `gap / frame / chain / decode / render / timeout` 的清晰 owner 和闭环状态机。
- 如果继续按局部补丁推进，后续会把 `mode.md` 的语义打散到 `video_source / ingress / recovery / pacer / diagnostics` 多处；这会让“理念一致、实现碎片化”的问题持续累积，影响后续 Xbox 远端主线升级。

## Goal

- 以 [docs/mode.md](/Users/guo.xu/Documents/code/games/xbxrc/docs/mode.md) 为目标形态，规划并逐步完成当前 Rust-owned Xbox 远端视频运行时的完整升级。
- 把现有实现从“以 NACK/恢复补丁驱动的局部止损”升级为“以时间线价值驱动的分层调度系统”，让 `packet / gap / frame / reference chain / decode / render / timeout` 的职责边界明确、状态语义一致、观测链路完整。
- 在升级过程中保持当前 canonical 技术路线不变，不引入第二套媒体运行时，不破坏现有 Cloud/Home 主链可用性。

## Scope

- In scope:
  - `docs/mode.md` 目标语义到当前代码的映射与分阶段落地路径
  - `crates/xbxengine/core/src/transport/rtc/stream/video_source/*`
  - `crates/xbxengine/core/src/transport/rtc/stream/nack_scheduler.rs`
  - `crates/xbxengine/core/src/transport/rtc/pipeline/*`
  - `crates/xbxengine/core/src/media/video/ingress/*`
  - `crates/xbxengine/core/src/media/video/decode/*`
  - `crates/xbxengine/core/src/media/video/pacer/*`
  - `crates/xbxengine/core/src/media/video/render/*`
  - `crates/xbxengine/core/src/transport/rtc/recovery/*`
  - `crates/xbxengine/core/src/diagnostics/*`
  - runtime trace / stats / protocol dto / Tauri trace projection 中与恢复、帧生命周期、链路状态相关的可观测性
- Out of scope:
  - 更换 Tauri + Vue + TypeScript + Rust 固定栈
  - 引入 Electron / 浏览器运行时作为平行实现
  - 重写会话层 `xbox-streaming` 启动流程
  - 把成熟的 Rust transport / media 恢复语义迁移到 TypeScript
  - 单独围绕 BWE/TWCC 参数调优来替代运行时语义升级

## Plan

1. 建立总语义映射与升级边界。
   - 产出 `mode.md -> 当前模块 -> 缺失 owner / 状态机 / 观测` 的精确映射。
   - 明确哪些现有实现可以保留并提升，哪些只是过渡补丁，避免升级过程中目标漂移。
   - 统一术语：`gap state / frame receive state / frame recovery disposition / chain state / decode candidate / render candidate / timeout action`。
2. 收包与 NACK 层升级为显式的 `gap + frame receive` 状态机。
   - 在 `video_source + nack_scheduler` 中把当前隐式窗口逻辑提升为明确的 gap 生命周期：`Observed / ReorderPending / NackCandidate / RepairInFlight / Resolved / Expired`。
   - 把 frame 接收态提升为显式状态：`Open / GapPresent / Repairing / CompleteCandidate / Closed`。
   - 把 late RTX、旧时间区间包、chain broken 包的接纳语义前移到收包层，避免后段继续承接无价值对象。
3. 提升 frame 裁决层，使“完整”与“可用”解耦。
   - 在 sample assembly -> ingress 交界层显式表达 `Complete / Poisoned / Dead`，不再只用少量布尔/枚举近似表达。
   - 把 `frame deadline`、`repair budget`、`reference-safe`、`frame still worth decode/present` 变成统一出口条件。
   - 沿用当前 `frame_recovery_ledger`，但升级为稳定的帧级账本与裁决输入，而不是只服务局部 Cloud 策略。
4. 提升 reference chain controller，形成真正的一等状态机。
   - 将当前 `waiting_for_recovery_keyframe`、`UnrecoverableReferenceChain`、`RequestKeyframe` 等离散语义收敛为统一链路状态：`Healthy / Repairing / Broken / Recovering`。
   - 明确链断时的强动作：停旧链 NACK、清未完成旧链帧、清 decode/render backlog、请求新锚点。
   - 将现有 Cloud 高 RTT `latency-first` 工作并入该阶段，作为第一批已落地的链断止损能力，而不是旁路特例。
5. 升级 decode / render 调度为“最新安全时间线优先”。
   - decode 阶段从当前 ingress 前置丢弃 + 小队列模式，升级为显式 `Candidate / Queued / Decoding / Decoded / DroppedBeforeDecode / DroppedAfterDecode` 调度语义。
   - render 阶段从 latest-slot 覆盖的隐式策略，升级为显式“当前时刻最佳安全候选”选择器，并在新锚点到来时强清旧链 backlog。
   - 保持低感知延迟目标不变，不回到按到达顺序偿还历史债务的连续性优先模式。
6. 收口 timeout controller 与可观测性。
   - 统一 repair/frame/decode/render/recovery 各层 deadline 与 timeout action，避免状态悬空。
   - 让 runtime trace / stats / diagnostics 能直接观察 `gap state`、`frame state`、`chain state`、`decode candidate decision`、`render candidate decision`、`timeout transition`。
   - 完成真实 Cloud/Home 运行态 trace 回归，验证 backlog、`packet_to_present_ms`、`wait-keyframe`、恢复切链行为与 `mode.md` 目标一致。

## Milestones

1. M1: 语义对齐与 owner 收口
   - 完成术语、状态机边界、模块 owner 映射
   - 明确现有 Cloud 高 RTT RFC 与本 RFC 的从属关系
2. M2: 收包/NACK/Frame Receive 状态机
   - gap 与 frame receive 状态显式化
   - late RTX / old chain 包前置放弃
3. M3: Chain Controller 与 frame 裁决闭环
   - `Healthy / Repairing / Broken / Recovering`
   - 旧链停损动作完整接线
4. M4: Decode/Render 调度升级
   - decode/render 以“最新安全时间线”驱动
   - backlog 不再作为隐式历史债务处理
5. M5: Timeout 与观测闭环
   - 统一 timeout 语义
   - 完成 trace 回归和验收

6. M6: 收口与防腐化完成态
   - 旧旁路策略与散落阈值收口到统一调度控制面
   - 验收与文档/跟踪更新完成，形成长期维护基线

## M1-M6 执行清单（本轮强约束）

> 本节作为“完整升级落地”执行面板，后续每个阶段都必须在本节记录：目标、实现项、验收标准、验收结果、残余风险。  
> 不允许只记录“已做了什么”，必须记录“是否达标”。

### M1 计划固化与边界冻结

- 目标：
  - 固化统一调度目标函数（Display Continuity / Recovery Efficiency / State Stability）
  - 冻结控制面边界：调度动作只能从统一 policy engine 下发
  - 冻结阶段验收口径与回滚策略
- 实现项：
  - 在本 RFC 落地 M1-M6 明确标准（本节）
  - 统一 owner 红线写入“防腐化红线”
- 验收标准：
  - 本 RFC 中有可执行的 M1-M6 清单与阶段验收项
  - 后续阶段更新不再新增并行流程文档
- 验收结果：
  - Status: ✅ Done (2026-03-28)
  - 证据：本节新增 + 下方“阶段验收记录”开始生效
- 残余风险：
  - 无代码风险，风险在于后续执行偏离；通过阶段验收门禁控制

### M2 控制面收口（SchedulingPolicyEngine）

- 目标：
  - 建立统一调度入口，避免 recovery/reconnect/BWE 继续散落并行决策
- 实现项：
  - 新增 `SchedulingPolicyEngine`，统一承接 recovery/reconnect/BWE proposal
  - `RtcSessionPolicy` 只负责“采样/投喂”，不再承担多处分叉控制逻辑
- 验收标准：
  - `RtcSessionPolicy` 的命令生成通过 engine 单入口
  - planner 只做优先级与命令编排，不做恢复策略分叉
  - `cargo check -p xbxengine` 通过
- 验收结果：
  - Status: ✅ Done (2026-03-28)
  - 证据：
    - `crates/xbxengine/core/src/transport/rtc/policy/scheduling.rs` 新增统一 `SchedulingPolicyEngine`
    - `crates/xbxengine/core/src/transport/rtc/session/policy.rs` 命令出口已通过 engine 单入口
    - 验证：`cargo check -p xbxengine`、`cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
- 残余风险：
  - recovery profile 切换时 controller 运行态重建仍依赖 runtime stats 口径稳定
  - `SchedulingPolicyEngine` 的 owner-aware BWE hard gate 已落地，后续如果要继续收紧，还需要再评估 `SeekingAnchor/Priming` 与 `StableServing` 的 BWE 允许范围是否需要按场景细分

### M3 恢复状态机升级（双阈值 + 冷却 + profile）

- 目标：
  - 解决“升级过快/升级过慢”两类失配，避免 `healthy <-> repairing` 振荡
- 实现项：
  - 把关键帧/decoder reset/reconnect 的升级节奏收敛为双阈值与冷却策略
  - cloud/home/relay 仅参数分层，不做逻辑分叉
- 验收标准：
  - 新增或更新 recovery 单测覆盖“有界 repair -> 升级 -> 冷却”
  - 关键帧请求不会在冷却窗口内重复风暴
  - `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture` 通过
- 验收结果：
  - Status: ✅ Done (2026-03-28)
  - 证据：
    - `crates/xbxengine/core/src/transport/rtc/recovery/policy.rs`：cloud/home/relay 分层参数化（cooldown/min-interval/upgrade-window）
    - `crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs`：升级节奏改为有界窗口 + 最小间隔，防 keyframe 风暴
    - `crates/xbxengine/core/src/transport/rtc/session/policy.rs`：按 profile 动态刷新 escalation controller
    - 验证：`cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`、`cargo test -p xbxengine transport::rtc::recovery::escalation -- --nocapture`
- 残余风险：
  - profile 参数需结合真实 trace 继续收敛

### M4 供需闭环（显示侧 noPending 信号入调度）

- 目标：
  - 把 `noPendingFrame` 长空窗从“诊断日志”升级为“调度输入”
- 实现项：
  - host present 侧新增供给空窗信号回灌 runtime stats
  - 调度读取该信号并参与恢复动作门控与升级
- 验收标准：
  - runtime stats 可直接观察连续 `noPendingFrame` 指标
  - 调度逻辑中存在对该指标的显式使用（非 trace-only）
  - `cargo check -p xbxrc` + `cargo check -p xbxengine` 通过
- 验收结果：
  - Status: ✅ Done (2026-03-28)
  - 证据：
    - `src-tauri/src/mods/native_video/mod.rs`：WGPU noPending 计数/streak/max-streak 真接线（非占位）
    - `src-tauri/src/mods/native_video/presenters.rs`：WGPU 诊断投影改为真实 noPending 指标，不再固定 0
    - 调度读该信号：`crates/xbxengine/core/src/transport/rtc/session/policy.rs` + `crates/xbxengine/core/src/transport/rtc/policy/scheduling.rs`
    - 验证：
      - 已确认：`cargo check -p xbxrc`、`cargo check -p xbxengine`
      - 已补跑并通过：`cargo test -p xbxrc wgpu_ -- --nocapture`
- 残余风险：
  - host timing 采样抖动可能导致短窗误判，需要平滑窗口
  - `noPending` 高压分支当前仍可能把 `CooldownSuppressed` 直接提升为 `RequestKeyframe`，存在绕过冷却的潜在风险（见下方专项门禁）

### M5 可观测性与回归验收

- 目标：
  - 能明确回答“为何升级、何时升级、升级后是否收敛”
- 实现项：
  - trace 增加调度控制面状态事件（state transition / demand pressure）
  - 增加对应单测与投影测试
- 验收标准：
  - `cargo test -p xbxrc trace_projection -- --nocapture --test-threads=1` 通过
  - `cargo test -p xbxengine transport::rtc::stream::video_source:: -- --nocapture` 通过
  - `cargo test -p xbxengine media::video:: -- --nocapture` 通过
- 验收结果：
  - Status: ⏳ In Progress (2026-03-28)
  - 当前已完成：
    - timeout / chain / stall / decodeCandidate / renderCandidate 的 trace 语义事件与投影测试已落地
    - `videoHealth/primaryIssueChain` 已改为优先反映 display supply，严重 `noPending + stale present/decode` 不再误报 `steady:healthy`
    - `trace_projection` 已投影 `hostPresentState.noPending*`、`directGamingState.videoHealth/primaryIssueChain`、timeline/candidate/stall 事件
    - 2026-03-29: Home 场景 timeline 软/硬恢复分层与小预算收口已完成，clean anchor 短窗内的 delta gap 重入不再直接把链路打回 `Repairing/Broken`
    - 2026-03-29: recovery coordinator 进一步收口，timeline 已 `healthy` 且 clean anchor 新鲜时，`TransportAwaitRecoveryKeyframe` 连续信号不再因为 streak>=3 就绕过 cooldown 硬升到更硬恢复
    - 本轮验收已通过：
      - `cargo test -p xbxengine diagnostics::stats -- --nocapture`
      - `cargo test -p xbxrc trace_projection -- --nocapture --test-threads=1`
  - 当前未完成：
    - 最新 cloud/home 两份真实 trace 的门禁回归尚未闭环
  - 残余风险：
    - 真实 trace 验收仍需与 cloud/home 样本继续对齐

### M6 收口与防腐化完成态

- 目标：
  - 形成可长期维护的统一调度架构，清理旁路与临时策略
- 实现项：
  - 删除/收敛重复入口与旁路判断
  - 更新 `docs/project-task.md` 与 RFC 阶段记录，形成可审计闭环
- 验收标准：
  - 阶段验收记录完整，残余风险清单明确
  - `cargo check -p xbxengine`、`cargo check -p xbxrc` 全绿
- 验收结果：
  - Status: ⏳ In Progress (2026-03-28)
  - 当前已完成：
    - M1-M4 阶段化收口与文档跟踪机制已固化
    - 已完成本轮四步控制面收口：
      - REMB 改为“仅 target 变化才请求”，移除同 target 周期性重发
      - `noPending` 不再在 `SchedulingPolicyEngine` 旁路强拉 `RequestKeyframe`
      - display supply 已成为统一恢复入口，恢复预算继续由单一 `VideoEscalationController` 持有
      - 摘要口径已从 transport 健康优先改为 display supply 优先
      - display supply 阈值已纳入 scenario recovery profile，Cloud/Home/Relay 不再依赖静态常量
    - 本轮验收已通过：
      - `cargo test -p xbxengine transport::rtc::connection::tests::service -- --nocapture`
      - `cargo test -p xbxengine transport::rtc::policy::display_supply -- --nocapture`
      - `cargo test -p xbxengine transport::rtc::policy::scheduling -- --nocapture`
      - `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
      - `cargo test -p xbxengine diagnostics::stats -- --nocapture`
      - `cargo test -p xbxrc trace_projection -- --nocapture --test-threads=1`
      - `cargo check -p xbxengine`
      - `cargo check -p xbxrc`
  - 当前未完成：
    - 最新 cloud/home 真实运行日志上的阈值收敛与节奏门禁尚未闭环
  - 残余风险：
    - `SchedulingPolicyInput.demand` 当前在 `scheduling` 层只作为统一输入透传，尚未形成更多基于 demand 的 planner 级编排
    - 若后续继续扩展 health taxonomy，`stats` 与 `trace_projection` 的字符串口径建议进一步常量化/枚举化

## 重点风险专项门禁（M5/M6）

### 风险项：noPending 高压强制 keyframe 绕过冷却

- 风险描述：
  - 旧实现中，`SchedulingPolicyEngine` 会在 `no_pending_pressure_level in {high, critical}` 且恢复动作为 `WaitForBurst/CooldownSuppressed` 时，直接改写为 `RequestKeyframe`。
  - 该分支虽然改善长空窗恢复速度，但也可能绕过 `VideoEscalationController` 冷却节奏，导致高压区间 keyframe 触发过快。
- 修复进展（截至 2026-03-28）：
  - 已完成：
    - recovery controller 已具备 `cooldown/min-interval/upgrade-window` 参数化
    - 调度层 `noPending` 强制升级旁路已删除，display supply 只负责统一 reason 选择，不再改写恢复动作
    - `RtcSessionPolicy` 已把 display supply 与 recovery diagnosis 收口到单一 `VideoEscalationController`
    - display supply 阈值已纳入 `RecoveryScenarioProfile.display_supply_thresholds`，Cloud/Home/Relay 可按 profile 独立收敛
  - 未完成：
    - 最新 Cloud/Home 真实日志上的阈值与节奏门禁尚未完成最终回归
- 验收门禁（未全部满足前，不标记 M6 Done）：
  - 代码门禁：
    - `noPending` 高压升级不能直接绕过冷却；必须受统一节奏门控（时间窗口/最小间隔/升级预算）约束。
  - 测试门禁：
    - 新增单测覆盖“高压连续窗口下不会 keyframe 风暴”
    - 新增单测覆盖“高压解除后恢复到常规冷却路径”
  - 运行态门禁（真实日志）：
    - Cloud：pending frame 长空窗时恢复速度提升，但 keyframe 请求频率受控
    - Home：不出现低 RTT 下过早升级与误触发 keyframe
  - 回归门禁：
    - `cargo test -p xbxengine transport::rtc::policy::scheduling -- --nocapture`
    - `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
    - `cargo test -p xbxengine transport::rtc::stack::transport_session -- --nocapture`
    - `cargo test -p xbxrc trace_projection -- --nocapture --test-threads=1`

## Transport Recovery Episode 收口（2026-03-28）

### 最新 Cloud trace 暴露的硬问题

- `recovery epoch` 被 repeated observation 持续抬高，导致“看起来一直在恢复”的假活跃状态。
- clean anchor 事实为 0（或长期不可用），说明恢复完成证据链没有真正建立。
- `episode active` 语义当前仍可被“epoch 差值”间接猜测，这在 repeated observation 下会失真，不能继续作为主判定。

### 本轮收口目标（不是再调阈值）

- 将 transport recovery 升级为显式 episode 状态机，`episode active/inactive` 由 episode 本身状态驱动，不再由 `epoch delta` 推断。
- 固化单一事实源：
  - owner/recovery 判断只消费 canonical episode/anchor 事实；
  - diagnostics/trace 只投影，不做二次推导补偿。
- 把“恢复完成”收口到 anchor 证据链，明确以 clean anchor + current episode 对齐为闭环条件。

### 本轮落地项（执行清单）

1. episode 状态化
   - 引入/收口显式 episode 字段（active、started_at、last_transition、reason）。
   - repeated observation 只能刷新观测时间，不得无条件推进 episode/epoch。
2. episode 与 owner 主链接线
   - owner 输出与 recovery proposal 只读取 episode 状态，不再依赖 epoch 差值猜 active。
   - reconnect/keyframe/reset 动作预算绑定当前 episode。
3. anchor candidate ledger 收口（当前唯一未完成项）
   - 建立并接线 `anchor candidate ledger`：候选、确认、失效与 episode 对齐。
   - 恢复完成必须消费 ledger 中“属于当前 episode 的有效 anchor”。
4. diagnostics/trace 契约门禁
   - `videoHealth/primaryIssueChain/latestDecisionSummary` 继续服从 owner。
   - `recovery_coupling_*` 保留辅助投影，不得驱动 owner 语义。

### Gate（达标前不关闭 M6）

- `GATE-EP-001`：repeated observation 不得抬高 episode/epoch。
  - 判定：同一 episode 内重复观测不会触发新 episode，也不会无条件 +epoch。
- `GATE-EP-002`：episode active 判定不得依赖 epoch 差值。
  - 判定：active 仅由 episode state 决定，代码中不可再出现 `epoch delta => active` 逻辑。
- `GATE-EP-003`：owner 恢复完成必须依赖 current-episode anchor 证据。
  - 判定：无有效 anchor candidate ledger 证据时，不得退出 recovering。
- `GATE-EP-004`：trace/stats 不得把 legacy coupling 反向驱动 owner。
  - 判定：`video_owner_*` 仅来自 canonical owner/episode 字段，`recovery_coupling_*` 仅辅助展示。

### 当前状态

- 已完成：owner 主链收口、观测契约分层、recovery/trace 多项门禁。
- 未完成（唯一重点）：**anchor candidate ledger**。
- 结论：当前阻塞已明确为 ledger 闭环，而不是阈值调优。

## Owner 纠偏与日志驱动验收（2026-03-28 新增）

### 上一轮未解决的根因（必须明确）

- 上一轮主要把控制动作收口到了 `session/scheduling/recovery`，但“是否仍有时间线价值”的真实 owner 仍未完全前移到 `video_source timeline/chain`。
- 直接后果是：控制面可以在 `transport` 指标看起来可用时继续输出 `healthy` 语义，而上游已出现 `gap-expired-*`，导致“链路摘要健康、时间线实质断供”的错配。
- 这属于 owner 选错层级，不是阈值微调可解决的问题；继续调参数只会放大架构腐化。

### 本轮 owner 纠偏决策（必须执行）

- 真实 owner 固定为：`crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.rs` 的 `gap/frame/chain` 状态机。
- `session/scheduling/recovery` 只做“消耗 owner 输出并执行动作”，不再定义“chain 是否健康”的源事实。
- `diagnostics/trace_projection` 只做 owner 状态投影，不得反向推导覆盖 owner 结论。

### 阶段目标与验收标准（替代泛化总结）

1. M5-OwnerConsistency
   - 目标：`chain.state` 与 `gap` 生命周期语义一致，不再出现“expired 但链仍 healthy”。
   - 验收标准：
     - 当出现 `source_event in {gap-expired, gap-expired-chain-flush}` 时，同窗口 `chain.state` 不能为 `healthy`。
     - 当 `chain.state=healthy` 时，不得存在未收敛的 `gap-expired-*` 事件残留。
   - 失败判定：任一日志窗口出现 `expired gap + healthy chain` 即判定未通过。

2. M5-DisplaySupplyCoupling
   - 目标：显示断供通过 `timeline/chain` 语义进入恢复，不再靠摘要层补救。
   - 验收标准：
     - `hostPresentState.noPending*` 高压窗口内，必须能观察到 `timeline/chain` 的恢复迁移（例如 `stalled/repairing/recovering`），而不是长期 `healthy`。
     - 不允许仅出现 `videoHealth` 降级而缺少对应 `timeline` 状态迁移。

3. M6-ActionBudgetConvergence
   - 目标：恢复节奏由单一预算 owner 控制，避免日志里动作风暴掩盖 owner 缺陷。
   - 验收标准：
     - 高压窗口内 `videoEscalation` 频率受冷却约束，且动作触发必须可追溯到 `timeline/chain` 迁移。
     - 不允许出现“无 owner 状态变化但连续升级动作”。

### 重点门禁（阻断 M6 关闭）

- 门禁名称：`GATE-TL-001: NoExpiredGapWithHealthyChain`
- 定义：
  - 对 Cloud/Home 最新两份运行日志，逐条扫描 `latest_video_timeline_observation`：
  - 若 `source_event` 命中 `gap-expired*`，则该条及其相邻恢复窗口内 `chain.state` 必须属于 `{repairing, broken, recovering, stalled}`，不得为 `healthy`。
  - 若发现任意一条 `gap-expired*` 对应 `chain.state=healthy`，判定门禁失败，M6 不得标记 Done。
- 当前状态：`In Progress`
  - 代码级门禁已就位：`timeline.rs` 已补齐匿名 Cloud 低价值 delta gap 的 chain debt 语义，并新增 snapshot 级测试
    - `anonymous_cloud_low_value_delta_gap_breaks_chain`
    - `anonymous_delta_gap_debt_survives_frame_observed_projection`
  - 运行态门禁仍待复核：必须使用补丁后的最新 Cloud/Home trace 重新验证，旧 trace 仅可作为补丁前基线，不再作为当前代码反证。

- 当前门禁进度（2026-03-28）：
  - 已通过：
    - `cargo test -p xbxengine transport::rtc::policy::scheduling -- --nocapture`
    - `cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`
    - `cargo test -p xbxengine transport::rtc::stack::transport_session -- --nocapture`
    - `cargo test -p xbxengine transport::rtc::connection::tests::service -- --nocapture`
    - `cargo test -p xbxengine diagnostics::stats -- --nocapture`
    - `cargo test -p xbxrc trace_projection -- --nocapture --test-threads=1`
    - `cargo check -p xbxengine`
    - `cargo check -p xbxrc`
  - 本轮新增收口：
    - `transport_session` 仅在 `RequestReconnectCandidate` 真正成功 stage 时写回 `latest_video_escalation_observation`，避免 pending 已占用时误写 escalation。
    - 新增 `transport_session` 定点测试，锁定 escalation observation 写回链路，防止后续回退。
    - `RtcConnectionService` 已移除同 target REMB 周期性重发，避免 `rtcTargetRembRequested` 在稳态高频刷屏。
    - display supply 已成为统一恢复 reason 入口，`high noPending + fresh output` 不再被误升级。
  - 仍待完成：
    - 真实 Cloud/Home 新口径日志门禁（`hostPresentState.noPending*` + `videoEscalation/keyframeRequested` 频率约束）尚未完成，M6 不能关。

## 防腐化红线（执行期）

- 恢复动作只能由统一调度控制面发出，不允许在 transport/media/render 层新增旁路触发。
- Cloud/Home/Relay 仅允许参数分层，不允许新增逻辑分叉实现。
- `noPendingFrame` / present 空窗必须作为调度输入，不允许只落在 trace 诊断。
- 同名指标单一口径：禁止 core/tauri 双写覆盖漂移。
- 所有阶段必须给出“验收结果 + 残余风险”，不能只更新完成描述。

## Validation

- [x] 为 `gap / frame / chain / decode / render / timeout` 各层新增或更新定点单测，覆盖关键状态迁移
- [x] `cargo check -p xbxengine`
- [x] `cargo check -p xbxrc`
- [x] 回归 `transport::rtc::stream::*`、`media::video::*`、`transport::rtc::recovery::*` 相关测试
- [ ] 基于新的 Cloud runtime trace 验证 backlog、`packet_to_present_ms`、`nackSkipped`、`frameRecoveryObserved`、链断切换行为
- [ ] 基于新的 Home runtime trace 验证低 RTT 下不出现过度激进的误判链断和过早 wait-keyframe

## Risks

- 当前运行时语义横跨 `video_source / ingress / recovery / decode / render / diagnostics`，如果没有先收口 owner，就很容易把 `mode.md` 继续做成“概念正确、实现补丁化”。
- `reference chain` 状态机如果抽象过重，容易和现有 `sample_builder / h264 inspection / wait-keyframe` 路径重复建模，增加复杂度但不增益。
- decode/render 调度升级如果过快切换，可能在 Home 低 RTT 场景引入不必要的抖动或切链过敏。
- 可观测性若不先行，后续真实 trace 将很难区分“策略正确但阈值不对”和“状态机本身断链”。

## Progress

- [x] Step 1: 已完成 `docs/mode.md` 与当前实现的代码级评估，确认总体方向一致，现有主缺口在 `gap/frame/chain` 状态机与 decode/render 显式调度
- [x] Step 2: 已起草本 RFC，作为后续整轮升级的总追踪文档
- [x] Step 3: 已开始第一轮架构收口，在 `video_source` 内建立显式 `timeline` 状态 owner，并把 `waiting_for_recovery_keyframe` / frame recovery ledger / chain broken 推进统一接入该 owner
- [x] Step 4: 已完成第一轮 M2/M3 接线：`video_source` 新增 `timeline.rs`，`source.rs` / `nack.rs` / `mod.rs` 已改为通过统一状态 owner 推进 wait-keyframe、gap repair、chain broken、clean keyframe 恢复
- [ ] Step 5: 继续扩展 `gap/frame/chain` 显式状态与可观测性，并推进 decode/render/timeout 闭环；当前已完成 decode/pacer/render 结构化 drop 首轮闭环，后续转向 timeout 与更强裁决状态机；完成后产出 Report

## Execution Notes

- Date: 2026-03-27 | Status: in-progress
- Update: 本轮不直接改实现，先建立整轮升级 RFC，把 `docs/mode.md` 从设计理念正式收口为可执行的多阶段升级计划。
- Decision: 现有 [`docs/rfcs/2026-03-27-cloud-high-rtt-latency-first-recovery.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-03-27-cloud-high-rtt-latency-first-recovery.md) 视为本 RFC 的已启动子阶段，不再作为最终架构目标的替代品。
- Decision: 升级优先级固定为 `chain > frame > packet` 的语义收口，不再继续以局部 NACK/BWE 参数微调充当主方案。
- Decision: 第一轮实现优先保持现有 Rust-owned 主链可运行，在当前模块内渐进收口 owner，再决定是否需要进一步模块拆分。
- Update: 第一轮已不再把 `waiting_for_recovery_keyframe` 和 frame recovery ledger 作为松散布尔/Map 组合维护，而是在 [`crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.rs) 建立统一状态 owner，并接到 [`source.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs)、[`nack.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs)、[`mod.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/video_source/mod.rs) 主链。
- Update: 本轮验证已通过 `cargo check -p xbxengine`、`cargo test -p xbxengine transport::rtc::stream::video_source::timeline -- --nocapture`、`cargo test -p xbxengine transport::rtc::stream::nack_scheduler -- --nocapture`。
- Update: 本轮继续把 `video_source` 从“有 owner、但观测不全”推进到“owner + observation 单链闭环”：清理了 `timeline.rs` 中重复的局部 snapshot/重复状态字符串实现，统一保留 `VideoTimelineObservation` 为唯一对外结构；并将 `gap/frame/chain` 的关键迁移直接投影到 runtime stats / protocol dto / Tauri trace，包括 `gap-observed-* / gap-nack-candidate / gap-repair-in-flight / gap-resolved / gap-expired-* / chain-broken / chain-recovery-keyframe-requested / frame-observed / frame-complete-candidate / frame-inspection-*-await-keyframe / frame-recovery-ledger-write / frame-recovery-ledger-consume`。
- Decision: diagnostics/trace 不再额外维护第二套 timeline 状态仓，只消费 `video_source::timeline` owner 在状态推进点生成的 `VideoTimelineObservation`；后续 decode/render/timeout 也必须沿这条 owner-first 路径扩展，而不是在 diagnostics 层补推导状态。
- Update: 本轮已进入 M4 首段，完成 decode/pacer/render 的结构化 `frameDrop` 首轮闭环：`frameDrop` DTO/trace/rpc 已升级为 `stage/action/detail + frame metadata`；`decode` output queue 现在以 `DecodedFrame` 为 owner，能够在 `outputQueueOverflow` 与 `pacerBackpressure/pacerDisconnected` 时记录结构化 drop；`pacer` 已区分 `deadline`、`rendererBackpressure`、`rendererDisconnected`；`render` latest-slot overwrite 与 `presentError` 已统一走 `record_pipeline_frame_drop(...)`，同时 `render_state` 的 `present_submit/overwrite` 计数已同步回 runtime stats。
- Update: 本轮新增定点测试覆盖 `present_frame_reports_overwritten_latest_metadata` 与 `enqueue_decoded_frame_returns_dropped_oldest_frame`，并再次通过 `cargo check -p xbxengine`、`cargo test -p xbxengine transport::rtc::stream::video_source::timeline -- --nocapture`、`cargo test -p xbxengine transport::rtc::stream::nack_scheduler -- --nocapture`。
- Decision: decode output queue 的 oldest-drop owner 固定在 `video_decode.rs`，actor 只负责把 owner 返回的 drop 转成 observation；render latest-slot overwrite owner 固定在 `renderer.present_frame(...) -> render/actor` 这一条链上，不在 diagnostics 层做二次推导。
- Update: 本轮继续推进 M4 的 owner 收口，补齐 `latest-slot -> host present success` 的 ACK 链：`XbxEngineMediaBackend` 与 `XbxMediaStackPort` 已新增 `acknowledge_latest_render_frame(frame_seq)`，并在 runtime `present_frame` 成功后回写 ACK；`render_state` 增加 `last_acknowledged_present_time_ms`，`render_signal_snapshot` 现在优先使用 ACK 时间，避免 ACK 清槽后把最近输出时刻丢失。
- Decision: host 成功 present 的消费确认必须回到 render owner（`renderer.rs`）而不是继续由宿主侧 telemetry 间接推导；这样 timeout/controller 才能基于同一 owner 链判断“有帧未消费/已消费但超时”。
- Update: 本轮新增定点测试 `acknowledge_keeps_last_present_time_for_snapshot`，并完成回归：`cargo test -p xbxengine acknowledge_keeps_last_present_time_for_snapshot -- --nocapture`、`cargo test -p xbxengine present_frame_reports_overwritten_latest_metadata -- --nocapture`、`cargo test -p xbxengine enqueue_decoded_frame_returns_dropped_oldest_frame -- --nocapture`、`cargo test -p xbxengine transport::rtc::stream::video_source::timeline -- --nocapture`、`cargo test -p xbxengine transport::rtc::stream::nack_scheduler -- --nocapture`。
- Date: 2026-03-28 | Status: in-progress
- Update: 为修复 P1“文档与真实验证状态漂移”，补跑并确认通过：`cargo check -p xbxengine`、`cargo check -p xbxrc`、`cargo test -p xbxengine transport::rtc::stream::video_source::timeline -- --nocapture`、`cargo test -p xbxengine transport::rtc::stream::nack_scheduler -- --nocapture`；据此将 Validation 中 `cargo check` 两项更新为已完成。
- Date: 2026-03-28 | Status: in-progress
- Update: 进入“功能完成阶段”后先落地 M5 的最小切片：在 `video_source::timeline` 增加 `timeout_reason` 观测记账（`chain.reason` 在无 frame close reason 时可回退到 timeout 原因），并在 `recv_frame_inner` 的 `idle_timeout / thin_stream_stall` 分支新增 `timeout-stream-idle` / `timeout-stream-thin-stall` timeline 事件；该变更只增强观测，不改变 wait-keyframe 或恢复策略。新增定点测试：`timeout_reason_is_exposed_via_chain_reason_when_no_frame_close_reason`、`timeout_reason_is_cleared_after_new_frame_observed`；回归通过：`cargo test -p xbxengine timeline:: -- --nocapture`、`cargo test -p xbxengine video_source:: -- --nocapture`、`cargo check -p xbxengine`。
- Date: 2026-03-28 | Status: in-progress
- Update: 继续推进 M5 观测闭环，在 Tauri trace projection 中为 timeout 型 timeline observation 增加语义化事件 `videoTimeoutTransition`（保留 `videoTimelineObserved` 兼容输出）。触发条件采用 `source_event.starts_with("timeout-")`，当前覆盖 `timeout-stream-idle` 与 `timeout-stream-thin-stall`。新增测试：`timeout_video_timeline_observation_projects_timeout_transition_event`，并补充断言普通 timeline 不会产生 timeout 事件。回归通过：`cargo test -p xbxrc video_timeline_observation_projects_event_and_snapshot -- --nocapture`、`cargo test -p xbxrc timeout_video_timeline_observation_projects_timeout_transition_event -- --nocapture`、`cargo check -p xbxrc`。
- Date: 2026-03-28 | Status: in-progress
- Update: 继续推进“timeout -> decode/render candidate”的观测收口，在 trace projection 增加 `videoDecoderStallTransition` 与 `videoRendererStallTransition` 事件：当 `video_decoder_stalled` / `video_renderer_stalled` 状态发生变化时，输出 `stalled/previousStalled + packet/decode/present age`，便于直接对齐“进入/退出 stalled”的时刻。新增测试覆盖“不重复触发、false->true/true->false 触发、decoder/render 独立触发”。回归通过：`cargo test -p xbxrc stall_transition -- --nocapture --test-threads=1`、`cargo test -p xbxrc trace_projection -- --nocapture --test-threads=1`、`cargo check -p xbxrc`。
- Date: 2026-03-28 | Status: in-progress
- Update: 继续推进“chain break 强裁决”可观测收口，在 trace projection 增加 `videoChainTransition` 与 `videoBacklogFlushed` 语义事件：`videoChainTransition` 在 `chain state` 变化或 `chain-broken/chain-recovery-keyframe-requested/chain-clean-keyframe-submitted` 关键 source event 触发时输出 `previous/current chain state+reason`；`videoBacklogFlushed` 在 `gap-expired-chain-flush` 触发时输出 flush payload（gap/frame/chain），用于直接观测“链断 -> backlog 清理 -> 恢复请求”路径。新增测试：`chain_broken_timeline_projects_chain_transition_event`、`chain_flush_timeline_projects_backlog_flushed_event`。回归通过：`cargo test -p xbxrc trace_projection -- --nocapture --test-threads=1`、`cargo check -p xbxrc`。
- Date: 2026-03-28 | Status: in-progress
- Update: 补跑 Validation 中的模块级回归并确认通过：`cargo test -p xbxengine transport::rtc::stream:: -- --nocapture`、`cargo test -p xbxengine media::video:: -- --nocapture`、`cargo test -p xbxengine transport::rtc::recovery:: -- --nocapture`；据此将“stream/media/recovery 相关测试回归”更新为已完成。
- Date: 2026-03-28 | Status: in-progress
- Update: 按“先 owner 再观测”的原则，继续把 timeout 收口到 `video_source::timeline` 状态机：新增 `ChainState::Stalled` 与 `on_timeout_detected()`，timeout 分支不再只写 `timeout_reason`，而是推进显式链状态；同时在 `observe_frame` / `mark_frame_complete_candidate` 中补齐 `Stalled -> Repairing -> Healthy` 恢复路径，避免继续依赖 trace 层猜测。`source.rs` 的 `idle_timeout/thin_stream_stall` 已接入该 owner 入口。新增定点测试：`timeout_detected_sets_stalled_chain_state`、`frame_observed_after_timeout_moves_stalled_to_repairing_then_healthy`、`timeout_does_not_override_recovering_chain`。回归通过：`cargo test -p xbxengine transport::rtc::stream::video_source::timeline -- --nocapture`、`cargo test -p xbxengine transport::rtc::stream::video_source:: -- --nocapture`、`cargo check -p xbxengine`。
- Date: 2026-03-28 | Status: in-progress
- Update: 继续推进“candidate state owner 化”最小闭环：`video_decode` owner 新增 `decode_candidate_state`（`Nominal/Backpressure`）和 `latest_decode_candidate_decision`，在 `outputQueueOverflow` 时进入 backpressure，并在压力解除后通过 `queueRecovered` 回到 nominal；`render` owner 新增 `render_candidate_state`（`Nominal/LatestOverwrite`）和 `latest_render_candidate_decision`，在 `latestSlotOverwrite` 后记录 replace 决策，并在 overwrite 清除后的下一帧回到 nominal。`decode/renderer actor` 现在会在决策序号变化时把 `decodeCandidateState` / `renderCandidateState` 写入 `latest_observation_label/summary`，让 runtime stats 链路可直接看见 owner 决策迁移。新增测试：`decode_candidate_state_recovers_to_nominal_after_pressure_is_relieved`、`render_candidate_state_recovers_after_latest_slot_overwrite_is_cleared`。回归通过：`cargo test -p xbxengine media::video::decode::video_decode -- --nocapture`、`cargo test -p xbxengine media::video::render::renderer -- --nocapture`、`cargo test -p xbxengine media::video:: -- --nocapture`、`cargo check -p xbxengine`。
- Date: 2026-03-28 | Status: in-progress
- Update: 在 owner 状态收口基础上补齐 trace/protocol/前端契约链：`trace_projection` 新增 `decodeCandidateStateTransition` / `renderCandidateStateTransition`，并通过 `cargo test -p xbxrc trace_projection -- --nocapture --test-threads=1` 验证投影；同时通过 `pnpm -s exec tsc --noEmit` 校验 TS 契约未破坏。
- Risk/Blocker: timeout/candidate/chain-break 的 owner 状态机最小闭环已落地，当前剩余高优先项转为真实运行态验收：仍需基于新的 Cloud/Home runtime trace 完成 RFC Validation 末两项（高 RTT backlog/切链行为与低 RTT 误判约束），并据此决定是否继续迭代阈值与策略。
- Date: 2026-03-28 | Status: in-progress
- Update: 完成 present 指标单一来源收口：`XbxEngineMediaBackend` / `XbxMediaStackPort` / `RtcStackRuntimePort` 新增 `update_host_video_present_metrics(...)`，Tauri runtime 在 `sync_native_video_host_feedback(...)` 中将 host present `fps/submit/drop/overwrite/descriptor telemetry` 统一回灌到 core runtime stats；移除 `runtime_state` 对 DTO 的二次覆盖，避免同一指标跨 core/tauri 双写漂移。
- Update: `render_state.render_signal_snapshot` 收口为 render owner 的时间语义（`latest_video_present_time_ms` + `video_renderer_stalled`），不再承担 host present 计数/帧率口径；`diagnostics::build_xbxengine_stats` 已改为直接消费 runtime stats 中的 host present counters 与 descriptor 字段。
- Update: host stale-drop 已接回统一 owner 链：native presenter/scheduling 通过 `take_pending_host_frame_drops` 向 runtime 回灌，core 侧 `record_host_video_frame_drop(...)` 统一转为 `latest_video_frame_drop` observation，避免停留在宿主 telemetry 孤岛。
- Update: 本轮回归通过：`cargo check -p xbxengine`、`cargo check -p xbxrc`、`cargo test -p xbxengine acknowledge_keeps_last_present_time_for_snapshot -- --nocapture`、`cargo test -p xbxengine render_signal_snapshot_marks_stall_when_latest_present_is_stale -- --nocapture`、`cargo test -p xbxengine present_frame_reports_overwritten_latest_metadata -- --nocapture`、`cargo test -p xbxengine enqueue_decoded_frame_returns_dropped_oldest_frame -- --nocapture`、`cargo test -p xbxrc trace_projection -- --nocapture --test-threads=1`。
- Date: 2026-03-28 | Status: in-progress
- Update: 阶段状态回归：M1-M4 已完成，M5/M6 继续推进。当前最高优先风险为“`noPending` 高压强制 keyframe 绕过冷却”；已通过 M3 参数化缓解恢复风暴，但调度层高压分支仍需门控收口与真实 Cloud/Home trace 验收后，方可关闭 M6。
- Date: 2026-03-28 | Status: in-progress
- Update: 本轮补齐“恢复动作执行后可观测链路”与防回退门禁：`transport_session` 在 `RequestKeyframe/RequestDecoderReset/RequestReconnectCandidate` 成功执行后写回 `latest_video_escalation_observation`（`observation_id/reason/action/observed_at_ms`）；其中 reconnect 分支仅在 `stage_reconnect_candidate=true` 时写回，避免 pending 已占用导致误记升级。新增定点测试 `reconnect_candidate_records_escalation_observation_when_staged` / `reconnect_candidate_does_not_overwrite_escalation_when_not_staged`，并通过：`cargo test -p xbxengine transport::rtc::stack::transport_session -- --nocapture`、`cargo test -p xbxengine transport::rtc::policy::scheduling -- --nocapture`、`cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`、`cargo test -p xbxrc trace_projection -- --nocapture --test-threads=1`、`cargo check -p xbxengine`、`cargo check -p xbxrc`。
- Date: 2026-03-28 | Status: in-progress
- Update: 补跑 M4 剩余门禁并通过：`cargo test -p xbxrc wgpu_ -- --nocapture`（3 passed）。至此 M1-M4 阶段验收门禁全部闭环；M5/M6 当前剩余门禁仅为 Cloud/Home 新口径日志回归。
- Date: 2026-03-28 | Status: in-progress
- Update: 回归 `timeline owner` 的残留风险后，确认当前明显漏口不在 `observe_frame / mark_frame_complete_candidate / apply_wait_keyframe_gate(false)` 等状态迁移函数本身，而在于运行态验收曾混用了补丁前 trace。现已将 snapshot 级门禁补齐到 owner 层：`anonymous_cloud_low_value_delta_gap_breaks_chain` 锁定 `frame-complete-candidate` 不得把匿名 expired debt 投为 `healthy`，`anonymous_delta_gap_debt_survives_frame_observed_projection` 锁定 `frame-observed` 投影同样不得回到 `healthy`。回归通过：`cargo test -p xbxengine transport::rtc::stream::video_source::timeline -- --nocapture`。
- Decision: `runtime-trace-1774696306179.jsonl` 中残留的 7 次 `expired gap + healthy chain` 继续保留为“补丁前基线样本”，不再用于否定当前代码；`GATE-TL-001` 的关闭条件改为“必须在补丁后的最新 Cloud/Home trace 上复核通过”。
- Date: 2026-03-28 | Status: in-progress
- Update: 继续收口“摘要从属关系漂移”问题。`video_source::source` / `timeline` 已把 inspection reject 的结构化原因直接回灌 owner：`InvalidSliceHeader / MissingSps / MissingPps / NonIdrVcl / NoVcl / inspectionError` 会进入 `timeline.frame.close_reason` 与 `timeline.chain.reason`，`await recovery keyframe` 分支也改为携带具体 reason，避免 trace 只能看到抽象 `awaitRecoveryAnchor`。同时 `recovery::escalation` 移除了 `TransportAwaitRecoveryKeyframe -> RequestDecoderReset` 的快速直通升级，持续 await 现在只允许留在 keyframe 节流重试链，不再因为单纯“等恢复帧时间长”就反复触发 reset 风暴。
- Update: `diagnostics::stats` 已把 `directGamingState / statsSnapshot` 的 owner 优先级收口到 `latest_video_timeline_observation.chain`。当最近 timeline 处于 `broken/recovering/stalled` 时，`videoHealth` 会优先投成 `waitingKeyframe/stalled`，`primaryIssueChain` 优先输出 `recovery:<timeline reason>`，`latestDecisionSummary` 优先输出 `timeline:<state>:<reason>`，不再在同窗口里把 `timeline.chain.state=broken/recovering` 摘要成 `steady:healthy`。
- Update: 本轮验收通过：`cargo test -p xbxengine transport::rtc::stream::video_source::timeline -- --nocapture`、`cargo test -p xbxengine transport::rtc::recovery::escalation -- --nocapture`、`cargo test -p xbxengine diagnostics::stats -- --nocapture`、`cargo test -p xbxrc trace_projection -- --nocapture --test-threads=1`。
- Date: 2026-03-28 | Status: in-progress
- Update: 基于新 Cloud trace `runtime-trace-1774698555592.jsonl` 的运行态复盘，继续推进 owner 主链收口：`timeline.rs` 引入“稳定恢复门禁”，`Repairing/Recovering/Stalled` 不再因单个 `frame-complete-candidate` 直接回到 `Healthy`，而是要求跨过最小稳定窗口并看到连续 clean frame 后才允许恢复；`on_clean_keyframe_submitted()` 仍保留 clean keyframe 直达 `Healthy` 的锚点语义。新增测试覆盖：`single_complete_candidate_does_not_whiten_recovering_chain_without_stable_window`、`recovering_chain_requires_stable_clean_frames_before_healthy`、`frame_observed_after_timeout_moves_stalled_to_repairing_then_healthy`。
- Update: `recovery::escalation` 继续从“冷却驱动重试”收口到“recovery epoch 驱动重试”：新增 `keyframe_epoch_active`，同一 epoch 内 `TransportAwaitRecoveryKeyframe/TransportExpiredDeadline` 等理由不会持续重发 `requestKeyframe`；只有显式 `reset_keyframe_epoch()` 或 reason class 切换后才允许下一次 keyframe 请求。保留 sample-loss / idle-timeout / thin-stream 到 decoder-reset / reconnect 的既有升级优先级。新增测试覆盖：`await_recovery_keyframe_is_throttled_within_same_epoch`、`keyframe_epoch_resets_on_reason_change`、`keyframe_epoch_can_be_reset_explicitly_after_recovery`、`repeated_transport_deadline_failures_are_throttled_within_epoch`。
- Update: 本轮把“clean anchor 已建立”从局部 timeline 事件提升为 runtime 持久事实，并与 `recovery_epoch` 显式绑定：[`crates/xbxengine/core/src/api/backend.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/api/backend.rs) 新增 `video_anchor_clean_epoch / video_anchor_clean_observed_at_ms / video_anchor_clean_source_event`；[`crates/xbxengine/core/src/diagnostics/observation_bus.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/diagnostics/observation_bus.rs) 在 `chain-clean-anchor-submitted` 到来时写入该事实；[`crates/xbxengine/core/src/runtime_stats_sink.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/runtime_stats_sink.rs) 在新 `recovery episode` 开始时主动失效旧事实。由此，clean anchor 不再只是 trace 可见事件，而成为 owner / recovery 主链可消费的恢复完成证据。
- Update: owner 恢复完成契约已进一步收紧：[`crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs) 的 `resolve_recovery_completion_evidence(...)` 现在在 `RebuildingSupply -> StableServing` 路径上要求“存在属于当前 `recovery_epoch` 的 clean anchor fact”，`frame-observed / frame-complete-candidate + fresh present/decode` 不再足以单独退出 recovering；[`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 已把 runtime stats 中的 clean anchor fact 接到 owner 输入，并补齐 `frame_observed_without_clean_anchor_fact_cannot_exit_recovering`、`owner_exits_recovering_after_recovery_completion_evidence` 等回归。
- Update: clean anchor 现在会主动结束上一轮 anchor 恢复上下文，而不是只让 owner 变绿：[`crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 新增 `acknowledge_clean_anchor()`，在 owner 稳定回到 `StableServing` 且 clean anchor fact 属于当前 epoch 时重置 `await_recovery_keyframe_streak` 并调用 `reset_keyframe_epoch()`；[`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 在连接层进入 `Recovering` 时会先废弃陈旧 clean anchor fact，避免旧锚点跨 reconnect/lifecycle recovering 泄漏到新一轮恢复。
- Update: 本轮新增验收通过：`cargo test -p xbxengine transport::rtc::stream::video_source::timeline -- --nocapture`、`cargo test -p xbxengine transport::rtc::recovery::escalation -- --nocapture`、`cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`。当前残余风险已收敛为“运行态是否需要把 `keyframe epoch` 的显式 reset 真正绑定到稳定 `healthy` 窗口的上层 session/trace 信号”，需要继续用补丁后的最新 Cloud/Home trace 复核。
- Update: 本轮新增验收通过：`cargo test -p xbxengine transport::rtc::policy::video_scheduling_owner -- --nocapture`、`cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`、`cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`、`cargo test -p xbxengine transport::rtc::recovery::escalation -- --nocapture`、`cargo test -p xbxengine transport::rtc::policy::scheduling -- --nocapture`。
- Date: 2026-03-28 | Status: in-progress
- Decision: 停止以“局部阈值/局部 if-else”继续推进 M5/M6，改为一次完整的顶层调度重构。判断依据：最新 Cloud trace 已证明当前主问题不是某个恢复条件漏判，而是“低延迟优先”尚未成为统一目标函数；现有 `timeline / recovery / display supply / diagnostics` 仍是并列事实源，导致在当前网络事实下天然无法稳定运行。
- Decision: 进入 1-4 架构改造阶段，后续实现必须遵守以下固定边界，不允许退回补丁式代码：
  1. 统一顶层状态机：引入单一 `VideoSchedulingOwnerState`，目标形态固定为 `SeekingAnchor / RebuildingSupply / StableServing / SupplyStarved`（允许保留启动态），禁止继续由 `timeline/recovery/display_supply` 分散宣布 `healthy`。
  2. 统一动作 owner：`NACK / requestKeyframe / requestDecoderReset / reconnect` 只能由顶层 owner 输出；底层模块可提供事实与候选动作，但不能再直接主导最终升级。
  3. 统一价值判断：每个 gap/frame/repair 候选必须先经过“是否仍具播放价值、是否有助于 anchor/supply 恢复”的统一裁决；Cloud 高 RTT 下默认优先放弃无价值 delta repair。
  4. 统一健康定义：`healthy` 必须由“连续新鲜帧已成功恢复 supply”派生，不允许由单个 `frame-complete-candidate`、单个 NACK 恢复或 transport 局部信号直接回白。
- Execution Plan (1-4 并行落地约束):
  - Workstream 1: 顶层状态机与 owner 契约
    - 写入范围：`crates/xbxengine/core/src/transport/rtc/session/policy.rs`、`crates/xbxengine/core/src/transport/rtc/policy/scheduling.rs`、必要时新增 `crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`
    - 交付：统一 owner state、状态迁移输入/输出契约、session/scheduling 接线
  - Workstream 2: 动作 owner 与 recovery epoch
    - 写入范围：`crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs`、`crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`、`crates/xbxengine/core/src/transport/rtc/stack/transport_session.rs`
    - 交付：顶层 owner 到动作输出的单链路、keyframe/reset/reconnect 的明确预算与 epoch reset 规则
  - Workstream 3: 价值判断与收包裁决
    - 写入范围：`crates/xbxengine/core/src/transport/rtc/stream/video_source/source.rs`、`crates/xbxengine/core/src/transport/rtc/stream/video_source/timeline.rs`、`crates/xbxengine/core/src/transport/rtc/stream/video_source/nack.rs`、`crates/xbxengine/core/src/transport/rtc/stream/nack_scheduler.rs`
    - 交付：anchor/supply 导向的 frame/gap/repair 价值裁决，消除“无价值 repair 仍推进主线”的结构
  - Workstream 4: 健康定义与观测/摘要契约
    - 写入范围：`crates/xbxengine/core/src/diagnostics/stats.rs`、`src-tauri/src/mods/xbxengine/trace_projection.rs`、必要时 `crates/xbxengine/protocol/src/runtime.rs` / `src/shared/rpc/xbxengine.ts`
    - 交付：`videoHealth/primaryIssueChain/latestDecisionSummary` 全部服从顶层 owner，不再直接从局部 timeline/recovery 事件推导 healthy
- Acceptance Gate (1-4 阶段):
  - Gate-1: 代码结构上存在单一 `VideoSchedulingOwnerState` 或等价顶层 owner 契约，且 `session policy` 只能消费其结果
  - Gate-2: 同一恢复 epoch 内不会出现重复 `requestKeyframe` 风暴，且 epoch reset 规则可从代码读出
  - Gate-3: Cloud 高 RTT 路径下无价值 delta repair 会在 owner/ingress 层被明确放弃，不再靠后段摘要掩盖
  - Gate-4: `healthy` 的进入条件只能由稳定 supply 恢复触发，trace/stats 不得再把局部事件回写成 `steady:healthy`
  - Gate-5: `timeline/nack -> owner/recovery` 的 reason label 必须为直接消费契约，不允许依赖多层字符串转译或漂移兜底；新增/修改 label 时，`video_source / session policy / recovery coordinator / diagnostics/trace` 必须同步验收

### 1-4 阶段真实达成情况回填（2026-03-28）

1. Workstream 1: 顶层状态机与 owner 契约
   - 当前达成：
     - owner 已接入 `session` 主线，`session policy` 不再把 `chain 是否健康` 作为独立事实源。
     - canonical `video_owner_*` 已由 runtime stats/trace 直接投影，观测主链已具备 owner-first 语义。
     - owner 已感知 `recovery epoch`，后续恢复节奏不再完全脱离 owner 上下文。
   - 未达成：
     - 尚未完全收敛为单一 `VideoSchedulingOwnerState`；当前仍是“owner 主链 + 局部兼容层”并存。
   - 残余风险：
     - 顶层 owner 契约虽已成立，但 reconnect 仍未完全纳入同一 owner 输出链，存在边界回退风险。

2. Workstream 2: 动作 owner 与 recovery epoch
   - 当前达成：
     - `recovery proposal` 主链已走 `coordinator`，`keyframe/reset` 的主升级节奏已不再分散在多处局部判断。
     - `recovery epoch` 已进入恢复节流语义，owner 与恢复预算开始共享同一轮次上下文。
   - 未达成：
     - reconnect 仍存在 `session` 内独立 proposal 旁路风险，尚未做到“所有恢复动作都只从统一 owner 输出”。
   - 残余风险：
     - 若 reconnect 继续保留 session 内部旁路，后续容易再次出现“owner 结论正确，但最终动作由旁路主导”的架构腐化。

3. Workstream 3: 价值判断与收包裁决
   - 当前达成：
     - `timeline/nack admission` 已前移低价值 repair 裁决，Cloud 高 RTT 下的低价值 repair 不再完全依赖后段摘要补救。
     - `video_source timeline/chain` 已成为价值判断主入口，不再只是 diagnostics 侧的解释层。
   - 未达成：
     - 当前 value contract 仍未完全显式化为统一 owner dto，局部路径仍靠现有 reason/label 兼容推进。
   - 残余风险：
     - 如果后续新增 value 语义仍以字符串拼接扩展，而不是先补契约，再补消费链，容易重新出现 label 漂移。

4. Workstream 4: 健康定义与观测/摘要契约
   - 当前达成：
     - canonical `video_owner_*` 与相关 timeline/recovery 事实已能直接投影到 runtime stats/trace。
     - `videoHealth/primaryIssueChain/latestDecisionSummary` 已改为 canonical owner contract 驱动：`stats` 仅消费 `video_owner_state/reason/source/observed_at_ms`，不再从 `recovery_coupling_*` 或局部 timeline/recovery 事件反推 owner healthy。
   - 未达成：
     - owner 语义已收口；当前剩余高优先项不在观测层，而是 `anchor candidate ledger` 尚未形成完整 episode 闭环。
   - 残余风险：
     - `recovery_coupling_*` 仍需作为 legacy/辅助字段保留，后续若被误用于 owner 判定会导致语义回退，需要持续门禁约束。

### Workstream C（2026-03-29）1-6 点映射与阶段验收

1. owner canonical 字段来源映射
   - 映射：`video_owner_state/reason/source/observed_at_ms` 作为唯一 owner contract 输入。
   - 现状：`diagnostics::stats::project_video_owner_contract(...)` 已直接读取上述四字段。
2. diagnostics owner 推导收口
   - 映射：`stats.rs` 不再自行构造/推导 owner state/reason/source。
   - 现状：`stats` 仅做 owner 投影与摘要拼接，不再以 `recovery_coupling_*` 充当 owner 来源。
3. video health 严格映射
   - 映射：`videoHealth` 仅由 owner state 做严格映射，不增加局部健康推导。
   - 现状：`map_owner_state_to_video_health(...)` 仅消费 owner state；本地 `video_owner_source` 兜底值已移除，避免伪语义注入。
4. 摘要语义 owner-first
   - 映射：`primaryIssueChain/latestDecisionSummary` 仅围绕 owner contract 生成。
   - 现状：摘要链路不再反向消费 coupling/timeline 事件去改写 owner 语义。
5. trace/projection 分层
   - 映射：`video_owner_*` 作为 owner 主语义；`recovery_coupling_*` 仅 legacy/辅助可观测。
   - 现状：`trace_projection` 同时投影两类字段，但 owner transition 断言以 `video_owner_*` 为准。
6. 本轮阶段验收标准与结果
   - 验收标准：
     - `diagnostics::stats` 测试必须以 canonical owner 字段驱动；
     - `trace_projection` 在 owner 不变时不重复 transition，timeline/nack/frameDrop 变化不改 owner 语义；
     - `recovery_coupling_*` 不得驱动 owner 断言。
   - 验收结果（2026-03-29）：
     - `cargo test -p xbxengine diagnostics::stats -- --nocapture`：22 passed
     - `cargo test -p xbxrc trace_projection -- --nocapture --test-threads=1`：25 passed

### 契约一致性验收（新增硬门禁）

- 门禁名称：`GATE-OWN-002: ReasonLabelContractConsistency`
- 目标：
  - `timeline/nack -> owner/recovery` 的 reason label 必须是直接消费契约，而不是依赖 `session policy`/`diagnostics` 的字符串猜测与兜底映射。
- 验收要求：
  - `video_source` 输出的 reason label 在 `session policy / recovery coordinator / diagnostics/trace` 中必须可直接消费，不允许同义多名。
  - 新增 reason label 时，必须同步更新消费点与测试，不允许只在摘要层补映射。
  - `diagnostics/stats` 若仍需保留兼容性推导，必须明确标记为过渡层，不得覆盖 canonical owner label。
- 未满足前置条件：
  - reconnect 仍存在 session 内独立 proposal 旁路。
  - `diagnostics/stats` 仍有少量语义推导，尚未完全退化为纯投影。

### 1-6 点收口映射（2026-03-29）

1. 顶层状态机
   - 当前状态：已建立 `VideoSchedulingOwnerState` 主链，并由 [`video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs) 驱动 `SeekingAnchor / Priming / RebuildingSupply / StableServing / SupplyStarved`。
   - 本轮收口：owner 的 anchor 判定不再反向依赖 `recovery.latest_diagnosis_label`，而是优先消费 timeline raw fact（`chain.state/reason + source_event`）。
   - 剩余门禁：owner state/health 已开始约束 BWE，后续如果要继续收紧，还需要再评估 `SeekingAnchor/Priming` 与 `StableServing` 的 BWE 允许范围是否按场景细分。

2. 动作 owner
   - 当前状态：`keyframe / decoder reset / reconnect` 已统一进入 `owner signal -> coordinator -> planner -> transport command` 主链。
   - 本轮收口：lifecycle reconnect 不再保留独立 proposal 语义，`propose_lifecycle_reconnect()` 已退化为 `propose_from_owner_signal()` 的兼容包装。
   - 剩余门禁：session 仍保留连接态 `Recovering` 的特判入口，后续要继续评估是否能完全外提为 owner 输入事实。

3. 价值判断
   - 当前状态：timeline/nack admission 已成为 Cloud 高 RTT 下的主裁决入口，低价值 delta repair 不再依赖后段摘要止损。
   - 本轮收口：anchor candidate ledger 对匿名 repair/expire 的 frame timestamp 绑定已补齐，同 epoch repair 不再把 candidate 漂白为无帧锚点。
   - 剩余门禁：value contract 仍主要以 reason label 体现，后续可继续评估是否要抽成更强类型 owner DTO。

4. 健康定义
   - 当前状态：`healthy` 进入条件已经被压到“current epoch clean anchor + fresh supply + healthy timeline chain”的组合证据，不再允许单个 `frame-complete-candidate` 洗白。
   - 本轮收口：owner 对 recovering 的进入与退出都以 timeline/anchor raw fact 为准，不再吃 recovery diagnosis 的二次推导。
   - 剩余门禁：真实 trace 仍需验证 `rebuilding-supply` 占比和退出时长是否明显下降。

5. 观测/摘要契约
   - 当前状态：`videoHealth / primaryIssueChain / latestDecisionSummary` 已 owner-first。
   - 本轮收口：明确要求 diagnostics/trace 只投影 canonical `video_owner_*`；`display phase / stall kind / coupling` 仅保留辅助解释，不得覆盖 owner 语义。
   - 剩余门禁：`recovery_coupling_*` 仍在 trace 快照中保留 legacy 字段，下游消费需继续压缩。

6. 阶段验收与防腐化
   - 当前状态：M1-M6 与 1-6 点已建立对应关系，当前核心验收门禁聚焦 owner 单入口、clean anchor 退出契约、Cloud trace 回归。
   - 本轮新增验收关注：
     - `owner` 不再读取 `recovery.latest_diagnosis_label` 决定 anchor issue
     - lifecycle reconnect 只走 owner signal 主链
     - diagnostics 不得把非 canonical owner 事实回写成 `steady:healthy`

### Update（2026-04-01, pass8）

- 新 trace [`runtime-trace-1775031750214.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775031750214.jsonl) 表明 connecting 侧已明显收敛，但主故障已下沉为两类：A. `TransportAwaitRecoveryKeyframe` 在 `keyframe / decoder reset / reconnect` 间摆动；B. reconnect 后 cloud `TWCC` 仍要经历 `builder-configured / missing-local-feedback / local-feedback` warmup，但该阶段此前没有进入调度模型。
- 针对 A，本轮在 [`crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/escalation.rs) 与 [`crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 收口为 staged recovery：`TransportAwaitRecoveryKeyframe` 的同 reason epoch 自动释放只在未升级到 decoder reset / reconnect 时才兜底重发 keyframe；transport-await hard fallback 只有在出现过 decoder reset 证据后才允许 reconnect，避免 media-domain 过早跳入 reconnect。
- 针对 B，本轮在 [`crates/xbxengine/core/src/transport/rtc/policy/scheduling.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/scheduling.rs) 引入最小 `TwccWarmupState`，并在 [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 从 runtime stats 解析 cloud warmup：`BuilderConfigured / MissingLocalFeedback` 阶段继续允许 recovery，但 BWE proposal 与调度下发都被硬 gate；仅当 `local-feedback` 且 `twcc_sample_valid=true` 时恢复 BWE。
- 配套在 [`crates/xbxengine/core/src/diagnostics/stats.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/diagnostics/stats.rs) 明确 `twccObservationState` 为 diagnostics 展示与 session warmup 共享契约，避免状态名变更与调度消费脱节。
- 本轮验收通过：`cargo test -p xbxengine transport::rtc::policy::scheduling -- --nocapture`、`cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`、`cargo test -p xbxengine diagnostics::stats -- --nocapture`、`cargo test -p xbxengine await_recovery_keyframe_is_throttled_within_window_and_releases_after_window -- --nocapture`、`cargo test -p xbxengine coordinator_staged_recovery_avoids_single_keyframe_hang_for_transport_await -- --nocapture`、`cargo test -p xbxengine transport_await_hard_fallback_requires_decoder_reset_attempt_before_reconnect -- --nocapture`。

### Update（2026-04-01, pass9）

- 在 pass8 基础上继续沿 trace 中“cloud reconnect 后 feedback 尚未 ready 就再次升级 reconnect”这一残余问题向下收口。
- 本轮在 [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 将 `TwccWarmupState` 进一步接入 recovery proposal 末端：当 cloud 仍处于 `BuilderConfigured / MissingLocalFeedback` 时，仅拦住 media-domain 的 `RequestReconnectCandidate`，统一降为 `CooldownSuppressed`，保持恢复链优先在 `keyframe / decoder reset` 本地收敛；连接域 `LifecycleRecovering` 不受此 gate 影响。
- 这一步的目标不是禁止 reconnect，而是把 reconnect 升级时机继续后移到“feedback warmup 至少完成基本建链之后”，避免 `builder-configured` 阶段在 recovery 与 reconnect 之间反复摆动。
- 新增验收通过：`cargo test -p xbxengine cloud_builder_configured_warmup_holds_media_reconnect_candidate -- --nocapture`、`cargo test -p xbxengine cloud_builder_configured_warmup_does_not_block_lifecycle_reconnect -- --nocapture`、`cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`。

### Update（2026-04-01, pass10）

- 在 pass9 基础上继续把 `TwccWarmupState` 从二值 gate 推进成分阶段 reconnect proposal 节流。
- 本轮仍在 [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 内完成最小实现：`lifecycle_reconnect_proposal_interval_ms()` 现在显式消费 `TwccWarmupState`，对 cloud 分成三档：
  - `BuilderConfigured`: 4.5s
  - `MissingLocalFeedback`: 3.5s
  - `LocalFeedbackReady`: 2.5s（恢复现有 cloud 默认）
- 这一步的目标是让 reconnect 节流强度与 feedback warmup 阶段一致，而不是继续用单一 cloud interval 覆盖整个 warmup 周期。
- 新增验收通过：`cargo test -p xbxengine cloud_builder_configured_uses_more_relaxed_lifecycle_reconnect_interval_than_missing_feedback -- --nocapture`、`cargo test -p xbxengine cloud_local_feedback_ready_restores_default_cloud_reconnect_interval -- --nocapture`、`cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`。

### Update（2026-04-01, pass11）

- 新 trace [`runtime-trace-1775037211971.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775037211971.jsonl) 显示 pass8-pass10 对 warmup / media-domain 已继续收敛，但主瓶颈重新上浮到 connecting 首帧前：`streamIdleTimeout=0`、`twcc-gcc-cloud-await-feedback=0`、`local-feedback` 明显增加，同时 `livenessNoProgressTimeout` 与 `livenessReconnectAttemptLimitExceeded` 大幅反弹，且第一次 failed-terminal 在第 3 次 no-progress reconnect 后即出现。
- 针对这一回归，本轮只在 [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 收口 connecting 域，不回碰 pass8-pass10 的 cloud warmup / media-domain 逻辑：
  - 为 `connecting + 首帧前` 单独引入更保守的 lifecycle reconnect proposal interval（4.5s），避免在尚未拿到稳定 transport/cloud 语义时继续按 1.5s 短节奏推进 reconnect。
  - 将 `connecting + 首帧前` 的 `livenessReconnectAttemptLimitExceeded` 改成“双门槛”：attempt limit 先作为软上限，只有 no-progress 长窗持续 90s 以上时，才允许真正进入 failed-terminal。
  - `Recovering/Connected` 的 failed-terminal 语义与 cloud warmup 阶段化节流保持原样，确保连接后半段收敛结果不被反向打散。
- 这一步的目标不是“继续提高 reconnect 频率”，而是把 connecting 域从“短促、偏猛的硬终态”拉回“慢一点但不断线的软推进”，避免 trace 中 20 秒左右就锁进 failed-terminal、随后长时间失去重试机会。
- 新增验收通过：`cargo test -p xbxengine cloud_pre_first_frame_timeout_is_relaxed_and_failed_terminal_waits_for_long_window -- --nocapture`、`cargo test -p xbxengine connecting_without_target_type_keeps_reconnecting_before_long_terminal_window -- --nocapture`、`cargo test -p xbxengine lifecycle_reconnect_attempt_limit_enters_failed_terminal -- --nocapture`、`cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`。

### Update（2026-04-02, pass12）

- 新 trace [`runtime-trace-1775093548670.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775093548670.jsonl) 证明 pass11 的 soft hold 仍然命中过晚：第一次 `failed-terminal` 出现在 `seq=6782 / ts=1775093566698`，但首次 `builder-configured` 要到 `seq=196007` 才出现，首次 `Connected` 要到 `seq=199671`，说明主故障仍落在 cloud early connecting 的 pre-builder-configured 首窗。
- 针对这一点，本轮继续只在 [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 做最小收口，不回碰 pass8-pass10 的 warmup/media reconnect 调度：
  - 将 `should_enter_connecting_pre_first_frame_failed_terminal()` 的 soft hold 条件从“cloud warmup 已建立后”前移到“cloud + pre-first-frame + Connecting/Recovering”窗口，不再要求 `twcc_warmup_state.blocks_bwe_updates()` 已成立后才开始保护 early connecting。
  - 保留 non-cloud 不受影响，且仍要求 no-progress 长窗达到 90s 后才允许真正进入 `failed-terminal`，避免把 terminal 完全取消。
  - warmup 已建立后的首帧前路径继续复用同一条 soft hold，不与 pass11 形成平行语义。
- 这一步的目标不是“进一步推高 reconnect 频率”，而是让 cloud 的 soft hold 真正覆盖最早出问题的 connecting 首窗，避免 trace 中 `builder` 还没出现就已经被 `livenessReconnectAttemptLimitExceeded` 判死。
- 新增验收通过：`cargo test -p xbxengine cloud_early_connecting_without_builder_waits_for_long_terminal_window -- --nocapture`、`cargo test -p xbxengine connecting_without_target_type_keeps_reconnecting_before_long_terminal_window -- --nocapture`、`cargo test -p xbxengine lifecycle_reconnect_attempt_limit_enters_failed_terminal -- --nocapture`、`cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`。

### Update（2026-04-02, pass13）

- 新 trace [`runtime-trace-1775096033088.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775096033088.jsonl) 显示 pass12 已经把 “首次 `failed-terminal` 后长期锁死” 压掉，`Connected` 从坏样本的 `529s` 级别收敛到约 `28.858s`；但第一次 `failed-terminal` 仍稳定落在约 `18.026s`，说明 cloud 首帧前还有一段 pre-builder / `twccObservationState=unavailable` 的 `lifecycle=New` 首窗没有吃到 soft hold。
- 针对这一残点，本轮继续只在 [`crates/xbxengine/core/src/transport/rtc/session/policy.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/session/policy.rs) 内最小收口，不扩散到 warmup/media reconnect 语义：
  - 将 `should_soft_hold_cloud_early_connecting_failed_terminal()` 的 cloud pre-first-frame 保护从 `Connecting/Recovering` 进一步补齐到 `New`，让 `builder` 尚未出现、TWCC 仍 unavailable 的首窗也复用同一条 soft hold。
  - 这样 `livenessReconnectAttemptLimitExceeded` 在 cloud early `New` 首窗里继续只作为软上限，必须等 no-progress 长窗达到 `90s` 后才允许真正进入 `failed-terminal`，而不是在约 `18s` 就被短促硬判死。
  - non-cloud 仍不受影响，pass8-pass10/pass12 已经落好的 cloud reconnect interval、warmup 分阶段节流、media-domain reconnect gate 也不变。
- 这一步的目标不是“继续延长所有 timeout”，而是把 early terminal 的残余错层补齐到真正与 trace 对应的 `New` 首窗，让 `attempt limit + 长时间窗` 的联合判定在 cloud 首帧前路径上闭合。
- 新增验收通过：`cargo test -p xbxengine cloud_early_new_without_builder_waits_for_long_terminal_window -- --nocapture`、`cargo test -p xbxengine cloud_early_connecting_without_builder_waits_for_long_terminal_window -- --nocapture`、`cargo test -p xbxengine connecting_without_target_type_keeps_reconnecting_before_long_terminal_window -- --nocapture`、`cargo test -p xbxengine transport::rtc::session::policy -- --nocapture`。

### Update（2026-04-02, pass14）

- 对 trace [`runtime-trace-1775096033088.jsonl`](/Users/guo.xu/Documents/code/games/xbxrc/runtime-logs/runtime-trace-1775096033088.jsonl) 继续下钻后确认，主矛盾已经从 early terminal 切到后段帧率：`Connected` 之后并非云侧无流量，反而能看到 `TWCC=stable`、`inbound_video_bitrate_kbps` 持续在数 Mbps 到十余 Mbps，但 `presentFps` 会掉到约 `1`，同时反复出现 `transportAwaitRecoveryKeyframe / waitKeyframe / referenceChainUnrecoverable / retryBudgetExhausted / noPendingFrame`。
- 本轮继续做最小改动，只收口两个和 trace 直接对上的残点：
  - 在 [`crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 放宽 `transportAwaitRecoveryAnchor` 的 streak 判定窗口，让 trace 中 1.5s-3s 级别的稀疏重复上报仍能累计到同一轮 staged escalation；避免 `requestKeyframe` 之后因为事件不够密，就长期重新掉回 `cooldownSuppressed`，无法及时升级到 `requestDecoderReset`。
  - 在 [`crates/xbxengine/core/src/transport/rtc/stream/nack_scheduler.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/stream/nack_scheduler.rs) 为 `cloudHighRttLowValueAdmission` 增加关键帧旁路：当缺失帧本身是 keyframe/高价值恢复帧时，不再直接按 `SkippedLowValue` 放弃，而是允许进入正常 NACK pending，避免高 RTT 云侧把恢复关键路径本身“低价值化”。
  - 不动 early terminal、warmup 分阶段 reconnect 节流、media-domain reconnect gate 的既有语义，保证前半段已经收敛的结果不回退。
- 这一步的目标不是“把所有 recovering 都改激进”，而是只缩短 `Connected` 后“已有码流但长期等恢复关键帧”的坏窗，并减少云侧高 RTT admission 对关键恢复帧的反向伤害。
- 新增验收通过：`cargo test -p xbxengine coordinator_staged_recovery_handles_sparse_transport_await_signals -- --nocapture`、`cargo test -p xbxengine low_value_admission_does_not_skip_keyframe_recovery -- --nocapture`、`cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`、`cargo test -p xbxengine transport::rtc::stream::nack_scheduler -- --nocapture`。

### Update（2026-04-03, pass15）

- 本轮把 `transport recovery episode` 的最后一块锚点语义收口到 current episode only：[`crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/policy/video_scheduling_owner.rs) 与 [`crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs`](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbxengine/core/src/transport/rtc/recovery/coordinator.rs) 现在都只接受当前 recovery epoch 的 clean anchor / candidate ledger，不再允许 epoch 差值 grace 跨 episode 复用旧锚点。
- 同时补齐两条回归：同 episode 的 clean anchor candidate ledger 仍可把 owner / coordinator 收回到稳定态，跨 episode 的旧锚点不会再把 `rebuilding-supply` 漂白成 `stable-serving`。
- 验收通过：`cargo test -p xbxengine transport::rtc::policy::video_scheduling_owner -- --nocapture`、`cargo test -p xbxengine transport::rtc::recovery::coordinator -- --nocapture`、`cargo check -p xbxengine`。
