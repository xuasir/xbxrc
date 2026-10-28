# 串流公共主线可靠性与代码精简 RFC

## Status

- Completion: 实施进行中；实机验收按阶段记录。
- Current State: in-progress
- Owner: 主窗口直接执行与验证，遵循用户要求。
- Last Updated: 2026-10-28
- 代码基线：`02f6db40d6138787ca85a582dedf3b5028ad57ea`。
- 实施授权：用户已明确“按 RFC 去做，直到完成改造”，持续执行 P0–P4。

## Background

承接[桌面实现复杂度审计](../isu/2026-10-27-desktop-implementation-complexity-audit.md)的“串流公共主线”部分。用户明确要求精简逻辑、完整保留功能，并把会话可靠性、稳定性及避免影响游玩的超时错误置于最高优先级。

推荐顺序为：**固定历史故障回归 → 修复启动、取消与退出 → 收敛连接恢复与状态权威 → 精简重复编排 → 长时间和故障验收。** 每次结构调整都要经过同一套行为合同。代码行数作为维护成本指标，游玩连续性和资源闭环作为交付指标。

[浏览器稳定版方案](../isu/2026-10-27-browser-stable-desktop-plan.md)提供可复用的生命周期和验收思路。本任务按当前功能全集制定计划，保留浏览器与 Rust runtime、现有输入后端、画质、超分、麦克风等能力；发行装配与功能裁剪分别沿各自任务处理。

## Goal

1. 已开始播放的会话在远端准备状态回退、短时控制面故障和统计延迟时继续服务用户；真实远端终态、认证失效和硬断链得到准确处理。
2. 每次启动、取消、重试、重连、退出都有明确归属、有限等待、可验证结果；旧操作的晚到结果只能完成自己的清理。
3. 连接恢复动作得到正确放行、执行与效果确认；局部媒体、解码、显示恢复保持各自权限与既有功能。
4. 减少重复状态写入、重复恢复资格判断、重复清理及跨异步网络调用持锁，保留当前 Rust 领域层与 Tauri/Vue 边界。
5. 历史超时、资源泄漏、错误关闭健康会话和重建风暴成为自动化与实机发布阻断项。

可靠性结论需要故障注入和 fresh trace 支持。服务拒绝、网络持续失联等真实失败以明确错误、有界恢复和资源收口验收；客户端误判、误关闭和无限等待按零容忍验收。

## Scope

| 范围 | 入口与责任 |
| --- | --- |
| 公共会话领域 | `crates/xbox-streaming/src/session/{flow,monitor,scheduler,store,recovery}.rs`，`session/api/*`、`session/signaling/*`、`policy/session/*` |
| Tauri 适配 | `src-tauri/src/mods/streaming/{service,types,rpc,events}.rs`，启动取消、错误投影与当前会话校验 |
| 页面执行与状态 | `src/streaming/useStreamExecution.ts`、`execution/view-state.ts`、`session.ts`、`src/shared/rpc/streaming.ts` |
| runtime 接线 | `runtime-host.ts`、`runtime-host-policy.ts`、`runtime-contract.ts`、`browser-runtime.ts`、`xbxengine-runtime.ts`；必要的原生连接恢复接线按 P2 合同限定 |
| 兼容与验证 | `xbox-ios-bridge` 公共调用编译与合同回归；现有测试和 runtime trace 报告工具 |

媒体包接收、ReferenceChain、RecoveryLedger、NACK/PLI/TWCC、解码与 presenter 算法保持各自现有主线。手柄统一接口、几何导航、配置功能和发行依赖保持各自任务边界。涉及原生恢复的修改限定在连接动作 admission/取消/结果接线，并保留引擎最终健康连接 gate。

## 当前证据与处理优先级

下表区分源码确认、当前基线复现与待实机验证的风险；源码缺口说明可达路径，发生频率由运行样本确定。

| ID | 证据与当前行为 | 可靠性影响 | 处理 |
| --- | --- | --- | --- |
| R1 播放期超时 | [历史任务](../project-task.md) `2026-10-22-session-state-timeout-mislabeled-as-runtime` 于 10-25 闭环：monitor 在 Provisioned 后持续轮询，播放期回退到 ReadyToConnect/Provisioning 曾触发启动期 90 秒超时。当前 [monitor.rs:123](../../crates/xbox-streaming/src/session/monitor.rs:123) 已通过 `reached_provisioned` 保护；[scheduler.rs:400](../../crates/xbox-streaming/src/session/scheduler.rs:400) 保持播放期监控 | 修复已进入代码及单测，任务记录的 fresh trace 复验仍待完成 | P0 固定时间线回归；P1 明确启动与播放阶段合同 |
| R2 恢复去重自抑制 | [runtime-host.ts:241](../../src/streaming/runtime/runtime-host.ts:241) 和 [browser-runtime.ts:2986](../../src/streaming/runtime/browser-runtime.ts:2986) 先判 fact，再以第一次 `nextGate` 判同一个 fact。当前源码复现：t=1000 allowed=true，t=1001 加 reason 后 allowed=false / factWindow | 正常快速返回的恢复授权会在动作执行前被拦截；现有纯去重测试无法覆盖调用组合 | P0 补调用链负样本；P2 提交时单次记账 |
| R3 启动取消与晚到成功 | [useStreamExecution.ts:502](../../src/streaming/useStreamExecution.ts:502) 的配置/开流 await 后直接写 execution/sessionId；attemptId 目前用于启动事件过滤；[service.rs:924](../../src-tauri/src/mods/streaming/service.rs:924) 缺少按启动 attempt 取消入口 | 退出或重试时尚未拿到 sessionId，远端创建可继续，晚到成功可重新接线 | P1 从上下文解析前建立启动作用域及取消合同 |
| R4 重试与失败清理 | [useStreamExecution.ts:544](../../src/streaming/useStreamExecution.ts:544) 重试先清空 sessionId 再停本地 runtime；[flow.rs:579](../../crates/xbox-streaming/src/session/flow.rs:579) 普通 wait failure 返回前缺少统一清理；[useStreamExecution.ts:611](../../src/streaming/useStreamExecution.ts:611) 本地 stop 抛错会中断后续远端 close | 旧远端会话、后台任务和新启动可能重叠；旧资源难以追踪 | P1 同一幂等 teardown，失败也执行 finally 收口 |
| R5 等待范围错位 | [flow.rs:732](../../crates/xbox-streaming/src/session/flow.rs:732) exchange_offer 持续轮询 answer，缺少阶段总期限；[flow.rs:1084](../../crates/xbox-streaming/src/session/flow.rs:1084) wait-started 总计时覆盖云排队。HTTP 当前 [transport.rs:8](../../crates/xbox-webapi/src/transport.rs:8) 为每请求 12 秒、GET 最多 3 次 | 请求级超时无法约束整个协商；健康排队超过约 91 秒可耗尽 startup wait。嵌套重试还会放大总体等待 | P1 deadline 穿透；排队和准备分别计时 |
| R6 关闭持锁与清理分散 | [flow.rs:701](../../crates/xbox-streaming/src/session/flow.rs:701) 在取得最新凭证后才标记 closed，stop HTTP 期间持 mutation 锁；SDP、ICE、connect token、keepalive 也持同一锁跨 await。scheduler 与 flow 各有 clear_session，scheduler 版本缺少 signaling 清理 | 凭证获取失败可遗留本地循环；慢请求拖延取消；清理路径存在差异 | P1 先撤销作用域；P3 统一 registry 与短锁提交 |
| R7 晚到信令与代际 | [flow.rs:774](../../crates/xbox-streaming/src/session/flow.rs:774) answer 就绪优先于 session_exists 判定；[flow.rs:1345](../../crates/xbox-streaming/src/session/flow.rs:1345) poll ICE await 后按 sessionId 创建/更新 signaling 条目。store 的 `same_instance` 已保护 record 回写 | 信令回写与清理对资源代际的保护强度存在差异；sessionId 复用尤其需要防护 | P1 信令提交前校验资源身份，取消后丢弃旧 answer/ICE |
| R8 状态与授权缺口 | [view-state.ts:100](../../src/streaming/execution/view-state.ts:100) sessionProgress 可把已连接页面派生成 waiting；[session.ts:441](../../src/streaming/session.ts:441) 单次 progress RPC 异常进入全局 resolvedError，null 被跳过；[service.rs:1251](../../src-tauri/src/mods/streaming/service.rs:1251) decideRecovery 主要映射事实与调用方 isClosing | 控制面观测故障、实际资源失效、播放状态缺少明确区分；恢复授权缺少当前 session/代际的服务端核验 | P1/P2 权威事实合成、观测健康和当前资源校验 |

### 必须继承的历史约束

- 10-25 播放期超时保护继续保留；`reached_provisioned` 表示远端准备里程碑，实际首帧由 runtime 独立报告。
- [当前注册补偿 RFC](2026-03-24-home-session-server-registration-retry-compensation.md)及当前测试已经收窄旧的“Provisioning 10 秒 recreate”方案。P1 保留当前显式注册错误分类和有界补偿；`Failed + ServerNeverRegistered` code 当前直接 Exhausted，其他匹配消息仍可单次 Retry。这些差异逐项锁定，后续调整需要服务证据。
- [恢复领域边界 RFC](2026-06-02-webrtc-recovery-boundary-redesign.md)要求 receive/decode 完成媒体恢复、presentation 完成显示稳定、diagnostics 单向投影。公共会话服务保持这些边界。
- [lifecycle gate RFC](2026-06-22-lifecycle-liveness-reconnect-gate.md)与当前原生代码提供健康网络 gate。保留硬 ICE 失败、远端终态和显式控制入口；本地停顿先在所属领域处理。
- ICE 历史修复及当前代码分别作为依据；当前原生 exchange 窗口为 10–12 秒，浏览器已有 createOffer 10 秒保护。阶段预算按 runtime 实际合同装配。

## 功能保留清单

P0 为下列每行建立“入口 → 所有者 → 测试/实机证据”映射，P3 精简后逐行复验。现有未通过实机验证的能力继续标注其验证状态。

| 能力 | 保留合同 | 验证出口 |
| --- | --- | --- |
| home 开流 | 主机唤醒、ready/preflight、远端上下文与能力、配置等待、显式注册错误补偿 | 待机与已开机启动、真实注册失败分类、手动重试 |
| cloud 开流 | 区域/设备画像、游戏能力、排队 ETA、取消与准备阶段 | 排队超过 90 秒、取消、分配完成后正常播放 |
| 公共信令 | connect token、媒体与 chat SDP、增量 ICE、候选策略、end-of-candidates、direct-first/TURN fallback | 直连/relay、IPv4/IPv6、restart 与旧 answer/ICE 注入 |
| 会话保活 | 当前 monitor、keepalive、最新凭证获取及刷新、远端终态处理 | 长时间播放、凭证刷新失败/恢复、终态停止与资源回收；排队期保活按服务契约单列 |
| 播放与恢复 | browser/Rust runtime、首帧、音视频、已有关键帧/decoder reset/连接恢复与原生健康 gate | 相同故障合同分别跑两种 runtime，真实恢复以新帧进展证明 |
| 输入与远端控制 | 统一 GamepadBackend 与 owner、菜单/中性释放、键盘/触控及当前支持的原生输入、震动、软 Home、文本输入、关机 | 重连与退出交接、卡键回归、控制 RPC 能力矩阵 |
| 画面与声音 | video/WebGL、CAS/色彩、FSR 实验开关、分辨率变化、全屏、音量/静音、麦克风/chat 生命周期 | 设置前后行为、重连恢复用户意图、渲染/音轨资源释放 |
| 诊断与其他消费者 | 面板、截图/调试入口、错误与 trace、`xbox-ios-bridge` 公共调用 | DTO 兼容、日志脱敏、快照语义与调用方编译 |

## 目标职责与不变量

### 资源由各自领域持有

| 所有者 | 权威事实与动作 | 对外输出 |
| --- | --- | --- |
| Rust SessionFlowService | 远端会话记录、启动作用域、准备/排队、monitor、keepalive、信令代际、终态与远端清理 | 结构化 session 状态、错误、资源有效性和操作结果 |
| 公共 execution/runtime-host | 当前页面执行、资源关联、start/cancel/retry/stop 顺序、共享连接恢复 admission 与输入交接 | 当前 execution 的 UI 派生状态与 runtime 动作 |
| 当前 runtime | PeerConnection/原生连接、协商、音轨、媒体与显示资源、本地恢复执行 | 当前代际的连接/首帧/新帧/局部恢复/连接失败事实 |
| diagnostics/UI | 权威事实的单向投影 | 用户进度、观测健康、错误说明、统计与 trace |

每个 execution 关联 `attemptId`、远端资源 generation、runtime generation、协商 operationId。现有 `SessionCancelToken.same_instance` 和 runtime token 沿用并补齐接线；数值编号用于跨 RPC 取证，实际提交以当前资源身份校验。单一桌面页面只持有一个当前 execution，其他平台消费者保留自己的会话作用域。

固定不变量：

1. 启动超时只作用于仍处启动阶段的资源；进入远端 ready 后撤销该期限。实际播放、短期远端状态回退、健康排队各按所属阶段处理。
2. 一次关闭立即使该作用域失效；当前资源身份匹配才允许快照、answer/ICE、恢复授权及 UI 写入。关闭/取消/失败状态在同代际内吸收晚到普通事件，新启动显式创建下一代际。
3. 同一当前 runtime 同时最多一个连接级协商/恢复事务；同一事务授权和执行提交各发生一次。同一故障的多个来源合并为同一进行中的结果。
4. 局部渲染问题归渲染恢复、解码问题归媒体恢复、硬连接问题归连接恢复；原生 receive 事实与健康 gate 保留最终约束。
5. teardown 覆盖输入中性释放/震动停止、本地 runtime、媒体/音轨、远端 close、monitor/keepalive、信令与订阅；各项失败记录结果，后续收口继续执行。
6. 恢复完成需要当前代际真实新媒体与呈现进展；request 接受、RPC 返回、connected 和旧帧刷新分别记录各自事实。

### 启动与清理的具体合同

`start → resolve context → home preflight → create → queue/preparation → remote ready → negotiate → connected → mediaReady → playing` 沿现有业务顺序运行。home 执行自己的 preflight，cloud 保留自己的 queue；每个 await 前后检查作用域，取消从 sessionId 尚未产生时即可生效。

在 Tauri 现有 streaming 服务增加按 attemptId 管理的启动作用域与取消入口，领域启动入口消费 cancellation/deadline；复用现有桥和 API。取消请求先于启动注册到达时，以有界 attempt tombstone 使后续同 attempt 启动立即结束；所有操作在完成后释放作用域记录。远端 create 已被服务接受且响应晚到时，由该已取消作用域取得返回标识并完成自己的 close；响应永久缺失时记录创建结果未知，并按服务可用的 active-session 关联能力与远端回收契约处理。安全重试保持 HTTP 当前 GET-only 规则，create 的结果未知按显式协调处理。

`retry` 先保存旧执行的资源句柄，调用统一 teardown，再启动下一代际。用户的重复退出/卸载/重试共享同一 close promise；运行时停止失败通过 finally 执行远端清理。关闭先发布取消及 closed intent，短锁下取清理快照，HTTP 在锁外执行，完成时只提交对应资源结果。输入释放和日志使用有限等待，退出的关键清理始终能推进。

远端关闭失败时保留有界 cleanup 记录与重试结果，同一桌面目标的后续自动 create 等待清理结论。可通信时确认 stop/Closed/404；网络失联时本地资源先收口，页面明确呈现待回收状态，恢复通信后按清理记录协调。强制终止与 create 响应未知的回收能力以服务实测记录，普通退出的泄漏作为发布阻断项。

### 状态与错误的具体合同

保留现有 reducer，将 session 资源、runtime 连接、presentation 里程碑作为输入事实；`isLoading/isConnected/sessionUiPhase/statusText` 统一派生。播放期远端 `Provisioning/ReadyToConnect` 的回退记入诊断并保留已就绪事实；远端 `Failed/Closed` 按真实终态处理。播放期收到 WaitingForResources、未知状态和旧 error_details 的组合先做时间线注入，按明确服务语义确定展示与动作。

progress RPC 短期失败表达“观测暂不可用”；保留当前可用播放事实。null 表达本地资源缺失，需要关联 generation、清理/404/重启记录判定，有限窗口内重新核对。真实远端终态或永久认证失败生成结构化终止原因，并触发 teardown。错误增加或复用 stage/category/retryability 信息，启动、信令、认证、网络、远端终态、局部媒体与清理错误分别准确投影；历史超时分类继续回归。

### 连接恢复的具体合同

共享入口按 `proposal → 校验当前远端资源与 execution → single-flight → commit → runtime 执行 → effect observation` 处理。预检查采用纯判定；去重时间、fact/reason 与预算在实际提交一次更新。两阶段流程复用同一 admissionId，RPC 回来后只复核资源有效性；同一个授权对应一次动作提交。

runtime-host 的 transport 事件、browser mediaHealth、TURN fallback 以及其他会触发连接协商的来源进入同一连接事务。具体 runtime 继续拥有既有的 ICE restart/full rebuild 执行。麦克风/chat 与 SDP 配置重协商标注 reconfiguration purpose，与故障恢复共用协商串行化，保留各自用户意图。原生 engine 的自主连接恢复经现有 adapter/bridge 对齐同一 admission 合同；底层接收恢复、关键帧预算和健康 gate 留在引擎。

同一连续故障的连接自动尝试采用有限预算；稳定新媒体达到观察窗口后才结束故障并允许下一轮。计数范围按故障 episode 定义，使长时间游玩的后续独立故障仍有恢复机会。P0 固定各 runtime 当前参数，P2 统一连接预算与退避来源；局部媒体预算保持所属领域。TURN fallback 在首轮直连确有失败/耗尽证据时仅切一次，候选与媒体正在推进时继续观察当前连接。

## 时间与重试策略

耗时判断使用单调时钟，trace 另记录墙钟。父 operation deadline 穿透 token 获取、HTTP 各次尝试、退避、轮询和 runtime 协商。每次请求使用剩余期限约束，期限耗尽同时结束底层等待及后续提交资格。

| 阶段 | 首轮落地规则 | 校准与验收 |
| --- | --- | --- |
| home preflight/ready | 保留当前 90 秒策略及显式注册错误语义；所有子等待受所属 deadline 约束 | 待机唤醒/服务注册真实耗时与失败样本；证明嵌套等待有总上界 |
| cloud queue | 服务仍明确排队时使用独立 queue scope，保留 ETA 与随时取消；分配后再开始准备期限 | 注入 180 秒健康排队，随后准备/出帧成功；另验观测失联及服务终态 |
| session 准备 | 沿用当前 90 秒准备窗口，明确起点与状态迁移；进入 ready 后撤销 | Provisioning/ReadyToConnect 超时只发生在启动作用域；状态反复跳转仍受所属总预算约束 |
| SDP offer/answer | createOffer 当前 10 秒合同保留；给远端 answer 等待增加总期限。45 秒作为初始校准值，覆盖当前 GET 三次请求约 36.75 秒上界 | 初次与 restart、home/cloud 分别取样；最终配置固化到 session/runtime plan，正样本与负样本同时验收 |
| ICE/首帧 | 使用各 runtime 既有计划值与实际进展事实；候选、answer、RTP/track、解码与新帧分别记录 | 原生 ICE 当前 10–12 秒；浏览器现有保护单独核验；足够协商进展避免误触发停顿 |
| 播放期控制面 | 已 ready 的会话沿持续 monitor/keepalive；单次观测失败保留播放事实，持续失联有明确诊断与有限协调 | 注入超过两倍旧 startup timeout 的状态回退；真实 Failed/Closed 仍收口 |
| 自动连接恢复 | 单一连接事务，总期限与最大动作数在 P0 基线后固化；候选为每故障最多 2 次连接动作、沿现有 6 秒去重窗口 | 快/慢 RPC、同时触发、多次独立断网、预算耗尽、稳定后新故障分别验收；hard failure 仍能恢复 |
| stop/cancel | 本地作用域立即失效；本地停止、远端 stop/settle 分别有有限期限与失败记录 | 慢 token/SDP/keepalive 不拖住关闭锁；普通退出确认资源清零，失联回收单独取证 |

45 秒 answer 与恢复次数属于待测初值。P0 保存当前成功样本分布，P2 将最终 home/cloud/runtime 参数写入本 RFC，目标为成功路径兼容、失败等待有界、播放期误超时为零。所有失败结果保留具体阶段和服务事实。

## Plan

| 阶段 | 改动与交付 | 出口与依赖 |
| --- | --- | --- |
| P0：行为基线与故障合同 | 功能清单映射；补 monitor→store→progress→UI 时间线、host→授权→runtime 调用链、启动/关闭 deferred promise 测试；建立 mock gateway 最小注入点；收集 fresh 成功/故障 trace 与现有参数 | 正常路径通过；R2、取消晚到成功、重试泄漏、长排队等缺口成为准确失败的负样本；参数、平台与证据缺口记录清楚 |
| P1：启动、阶段与清理闭环 | 启动作用域/取消入口；queue/preparation 分别计时；deadline 穿透；统一 retry/teardown 与必要短锁提交；信令当前身份校验；结构化观测错误与终态处理；保留 R1 保护 | R1/R3–R8 相关场景通过；旧资源清理后新执行启动；取消与失败各路径资源有界；依赖 P0 |
| P2：恢复入口收敛 | 修复 R2；current-session admission；single-flight 与单次提交；TURN/transport/mediaHealth 合并；麦克风/配置协商串行化；原生 adapter 对齐；动作接受与播放恢复分开记录 | 快慢授权均执行一次；并发故障最多一个连接事务；健康连接局部恢复与真实硬断链各自通过；依赖 P1 |
| P3：结构精简与状态派生 | 统一 registry 清理/更新助手，缩短网络持锁；从 flow 分离 startup/信令编排，从 Tauri service 分离 context/error/projection，从 browser-runtime 分离协商与局部监督；收敛 reducer 派生字段 | 功能清单全量等价；重复决策/清理与持锁数量下降；结构调整分别可回退；依赖 P2 |
| P4：整机验证与交付 | home/cloud、browser/Rust、Windows WebView2/macOS WKWebView 的支持组合测试；soak、故障注入、性能对照、升级/配置回归及 beta 观察 | 下述硬门槛通过；平台待验项保留显式状态；实机完成后写实施 Report |

建议提交边界：P0 测试与证据；P1-a 启动取消/teardown；P1-b 阶段期限/错误；P2 admission/恢复；P3 每个资源域单独精简；P4 验收。每个行为修复及每个结构重排分别交付，复验同一套合同。

P0 的具体落点：Rust 扩展 `session/monitor.rs`、`scheduler.rs` 与 `flow.test.rs` 的时间线/编排测试；前端新增 `execution/view-state.test.ts`、`session.test.ts`、`useStreamExecution.test.ts`、`runtime/runtime-host.test.ts`，复用当前 Vitest 配置并 mock Tauri/RPC 与 runtime。公共 trace 门禁落到现有分析脚本目录的 `trace_streaming_session_lifecycle_gate.py`，fixtures 覆盖误超时、旧写入、授权未执行、关闭泄漏和真实终态正样本；脚本及这些新测试属于待实施产物。

### P3 的删减依据与边界

| 精简点 | 实施方式 | 保留依据 |
| --- | --- | --- |
| 多份 sessions/signaling/mutations 清理 | 在现有 store/registry 内提供按资源身份的单一 close/update；flow/scheduler 复用 | cancel token 生命周期、最新 plan/凭证和已就绪里程碑保持完整 |
| 巨型 flow/service 编排 | `flow.rs` 留公共门面，启动与信令归各资源域；Tauri 按现有 Rust mod 规范拆 context/error/projection | 公共泛型 provider 的跨平台复用、请求画像和配置语义继续存在 |
| host/runtime 重复恢复预算 | 单一连接 admission/事务；媒体内部恢复保持所属领域 | TURN、硬连接恢复、关键帧/解码能力和健康 gate 的正样本全部通过 |
| UI 多字段手写同步 | 由 session/runtime/presentation 权威事实派生 view state；单一 reducer 处理操作结果 | 菜单、进度、错误、媒体首帧与恢复提示逐项验收 |
| 兼容壳/失效字段/计时器 | 先列所有消费者、版本和实际数据，再移除确认无效项；迁移与删除分提交 | `xbox-ios-bridge`、两 runtime 与配置迁移分别验证 |

规模基线为原始行数：flow 2,041；scheduler 426；monitor 526；Tauri service 2,030；types 947；execution 899；view-state 254；runtime-host 757；browser-runtime 3,925；xbxengine-runtime 389。P3 同时报告生产逻辑总量、可变控制状态、连接动作入口、清理实现数量和跨 IO 持锁点；单纯拆文件的行数变化单独列示。维护改善以消除重复权威与决策为依据，功能和稳定性门禁优先。

## Validation

### 自动化故障矩阵

| 编号 | 输入时间线 | 必须证明的结果 |
| --- | --- | --- |
| V1 历史播放超时 | Provisioned/首帧后退回 ReadyToConnect、Provisioning，持续 180 秒；追加空 body/未知状态 | 当前代际保持可播放；startup timeout 事件=0；prepare/connect-token 资格准确；再注入 Failed/Closed 验证终态收口 |
| V2 启动超时与排队 | 从未 ready 的准备超过 deadline；状态反复跳转；cloud 健康 queue 180 秒后 ready | 准备阶段有界且报具体原因；排队阶段继续等待并可取消；所有失败远端资源有清理结果 |
| V3 取消竞态 | context/唤醒/create/ready/SDP/ICE/首帧各点 cancel；cancel 早于 start 注册；晚到成功/失败；sessionId 复用 | 新 UI/runtime 写入=0；旧响应只归旧作用域；晚到创建自行清理；后台任务、信令与订阅有界 |
| V4 重试与关闭 | 连续 50 次 start/retry/stop，重复 stop、卸载；本地 stop/远端 stop/token/trace 分别失败或延迟 | 一个 close promise；finally 完整执行；旧资源清理结论可追踪；可通信条件下 active remote、本地 PC/音轨/定时器均清零 |
| V5 去重与并发 | recovery RPC 在 1ms、5s、7s 返回；同 fact/reason、多个来源同时触发、关闭后 grant 晚到 | 正常 grant 提交一次；重复来源复用结果；停止后提交=0；两个协商操作互斥；预算只在真实动作处记一次 |
| V6 恢复权限与效果 | 健康 ICE 下显示/解码停顿；真实 ICE failed、无响应、短 disconnected；故障期间再触发麦克风/配置变更 | 局部问题进入局部恢复；硬断链得到连接恢复；持续真实新帧才确认 recovered；配置与麦克风意图保留 |
| V7 控制面异常 | progress RPC 超时/null、GET/keepalive 400/404/401/429/5xx；认证短期刷新失败；远端真实终态 | 短期观测失败保留已播放事实；永久失效按准确类别终止；安全 GET retry 与限流退避受总预算约束；保活规则逐类验证 |
| V8 信令隔离 | SDP/ICE 永远 pending、旧 answer/候选晚到、restart 返回相同快照、end marker、TURN fallback 与重连竞态 | 阶段总期限可取消；当前身份检查先于提交；增量候选与结束标志保留；TURN 仅一次且资源数量有界 |
| V9 睡眠与长播放 | 手动改变墙钟、最小化/恢复、睡眠/唤醒、统计采样停顿、两次独立网络故障 | 单调耗时正确；暂停期间按 runtime 供给事实处理；唤醒重取当前事实；长期恢复预算按故障 episode 正确重置 |

测试复用 Vitest fake timers/deferred promises、Rust 纯状态机和现有 provider；HTTP 编排通过最小 gateway/本地 mock server 注入。正样本同时覆盖真实失败的正确超时与终态，使抑制规则的范围可检验。mock、IPC 集成与真实服务各承担相应证据范围。

实施验证命令按改动范围执行：

```sh
cargo fmt
cargo test -p xbox-streaming --lib
cargo check -p xbox-ios-bridge
node scripts/with-ffmpeg-sdk.mjs -- cargo check -p xbxrc
node scripts/with-ffmpeg-sdk.mjs -- cargo test -p xbxrc --lib mods::streaming
pnpm lint:fix
pnpm exec vue-tsc --noEmit
pnpm test
pnpm build
git diff --check
```

原生连接接线发生变化时，额外执行 `xbxengine` lifecycle/reconnect matrix 与对应 Rust 检查。编译或链接环境的既有问题单列，并保留该平台待验证状态。

### 实机与 fresh trace 硬门槛

每个声明支持的关键平台 × home/cloud × runtime 组合单列结果；浏览器主线优先跑完整实机矩阵，原生路径按实际支持范围独立验收。Windows/ROG 与 macOS 按实际可用环境记录，公共测试和本机验证持续推进。

1. 每组合至少 50 次启动/正常退出，包含准备中和排队中取消；成功网络条件下客户端误超时、重复连接、普通退出资源泄漏、崩溃和持续卡键为零。
2. home/cloud 各至少 2 小时连续播放，核心稳定发行路径追加一次 8 小时 soak；穿过凭证刷新、菜单/全屏、音量与显示设置变化。记录内存、连接、音轨、定时器/订阅和任务数量的增长趋势。
3. 20 次受控网络断开/恢复，另跑丢包/抖动、TURN、睡眠/唤醒、服务状态回退、远端结束、麦克风重协商及渲染故障；每次故障输出原因、动作、期限、播放恢复效果和清理结果。
4. R1 时间线在实际运行中重放，健康播放期间 startup timeout=0；健康网络下由局部问题触发的连接 rebuild=0；真实硬断链正样本仍能恢复或在预算内清晰终止。
5. 同机、同源 profile、同网络与同渲染设置比较精简前后：启动/首帧 P50/P95/P99、最长停顿、恢复耗时、steady 帧率、submit/present 长尾和资源使用。P0 固定具体性能准入值；P4 达到已固化门槛后交付，所有功能保留项具备对应结果。

fresh trace 必须绑定实际构建 revision、平台/WebView、target/runtime、配置摘要、operationId 与资源 generation；mtime 检查之外核对实际运行二进制。历史 trace 用于负样本，当前运行证据用于交付。50 次成功表达本批通过；可靠率趋势由后续 beta 样本持续估计。

| 必报维度 | browser 路径 | Rust 路径 |
| --- | --- | --- |
| 网络健康 | ICE/candidate pair、RTT、bytes/packets/loss/jitter、可用 RTCP 增量 | TWCC/NACK/PLI、ICE/TURN、RTCP target 与传输事实 |
| 恢复链健康 | 授权/动作/效果、关键帧/解码与当前新帧；内部 ReferenceChain 可观测性明确标注 | ReferenceChain、RecoveryLedger、cleanAnchorCommitted、DisplayStable 按当前 epoch 关联 |
| media supply | received/decoded/video callback/audio progress | ingress/decode/output 供给 |
| steady supply | 同窗帧率、冻结时长、帧间隔 | steady supply 与帧预算/供给门禁 |
| submit/present 长尾 | draw/回调/processingDuration 等可用代理量，真实上屏需外部测量 | submit-to-present、host present、旧帧 refresh 与真实新帧分开 |
| 帧供给 delta | 同窗 packets→received→decoded→callback→draw 增量 | receive→decode→submit→present 同窗增量 |

浏览器使用现有 `trace_browser_webrtc_behavior_report.py <trace>` 核验协商、连接、首帧与统计证据；该报告对全会话超时/资源泄漏覆盖有限，P0 补公共 execution/session 的生命周期门禁，并为 V1–V9 建立通过/失败 fixtures。原生使用 `trace_receive_feedback_report.py <trace> --fail-on-gate`、`trace_midsegment_report.py <trace>` 和 `trace_webrtc_acceptance_gate.py <trace> --require-lifecycle-reconnect-gate`；针对健康局部故障注入使用 `trace_lifecycle_reconnect_gate.py <trace> --require-lifecycle-block`。正常无故障播放按实际场景选择 gate，缺失或跳过证据单列。

公共 trace 补 `operationStarted/Finished/Cancelled/TimedOut`、`recoveryProposed/Granted/Committed/Observed`、`cleanupStarted/Finished/Pending` 对应语义，字段复用现有事件系统。事件包含 stage、原因、身份、耗时、剩余预算及结果。token、账户标识、敏感 URL、原始 SDP/ICE 按现有脱敏策略处理；诊断队列有界，写盘延迟对关键清理使用有限等待。

## Risks 与回退

| 风险 | 控制方式 | 回退条件 |
| --- | --- | --- |
| 抑制误超时后掩盖真实失败 | V1 同时注入 Failed/Closed、认证失效、硬断链；远端事实与实际媒体分别取证 | 真正终态处理失效或恢复动作未执行即阻断并回退当前阶段 |
| 串行化/短锁改造改变竞态 | 当前身份校验、关闭先失效、cancel/late-response mock、同 sessionId 新代际验证 | 双连接、旧写入复活、重复 create/close 或卡键即回退 |
| deadline 误杀慢网络/健康排队 | P0 固定成功分布，按阶段与 runtime 分别配置，真实进展与超时负样本同时验证 | 成功率/首帧/恢复耗时越过固化门槛即停止后续精简 |
| 拆模块导致功能或跨平台合同丢失 | 功能清单逐行映射、两 runtime/其他消费者编译、行为与结构分提交 | 功能合同差异或平台缺证时保留待验状态 |

每阶段通过后才进入下一阶段；回退以阶段提交为单位，保持单一执行路径。发布阻断项出现时停止后续结构精简，先恢复已验证版本并固定失败样本。Beta 的观察周期、样本量与版本准入在 P4 记录，正式发布仍沿既有 release 流程授权。

## Progress

- [x] 基于当前代码核对公共主线及历史决策，区分 10-25 已修保护、当前缺口与待实机证据。
- [x] 当前源码复现 R2 的两阶段 factWindow 自抑制。
- [x] 建立功能保留清单、资源/状态/恢复合同、P0–P4 顺序、V1–V9 故障矩阵及回退条件。
- [x] 运行当前基线：`cargo test -p xbox-streaming --lib`，107 passed；`pnpm test`，24 files / 188 passed。
- [x] 用户已授权完整实施；已加入启动取消、统一清理、订阅和恢复调用链故障回归。
- [x] P1/P2 客户端主体、原生连接事务与 P3 结构精简已实施，并通过当前自动回归。
- [x] 公共生命周期门禁与功能保留映射已完成；本地验收手册已交接。
- [ ] P4 已分析首批 Mac 云游戏 fresh trace：构建身份、协商、首帧和播放区间通过；可见启动段有供给停顿，后续包含后台阶段；用户强制关闭，正常退出及断网恢复保留待验。[样本报告](../reports/2026-10-28-cloud-streaming-reliability-trace-validation.md)。服务侧未知创建结果回收、支持平台矩阵与性能准入继续收口。
- [ ] 产品实施完全完成后产出 Report 并收口任务。

## Execution Notes

- Date: 2026-10-28 | Status: in-progress
- Update: 规划交付完成，任务与复杂度审计已关联到本 RFC；本轮只修改文档。
- Decision: 以用户的可靠性与功能保留要求确定顺序，先交付 P0 故障合同与 P1 生命周期闭环，再收敛恢复和结构；当前媒体与输入功能继续保留。
- Evidence Boundary: 107 项 Rust 与 188 项前端基线通过，R2 当前源码复现成立；新增故障矩阵、性能阈值和 Windows/ROG/长时间实机证据由实施阶段交付。
- Implementation Entry: 按 `task-run` 的 complex 路径，在本计划完成后取得实施确认；建议首批范围为 P0 与 P1-a（启动取消、重试和 teardown）。


### 2026-10-28 实施进展

- 生命周期：上下文解析前建立 startup scope；按 attempt 取消，晚到 create 由原 scope 回收。UI execution generation 与 runtime launch generation 吸收退出、重试后的旧结果。
- 清理：本地撤销与远端 stop 归属独立任务；并发 close 共享结果，失败保留清理记录与原资源标识；runtime stop 失败继续远端 close，原生输入清理失败继续 native stop。同目标旧 startup/create/cleanup 未收口时阻止重叠启动。
- 期限：共享单调时钟；健康 cloud queue 独立于准备预算，queue 观测失联 45 秒有界。offer/answer 与 ICE poll 初值 45 秒（写入 session schedule，并提供旧序列化默认值），submit ICE、keepalive、connect token 和远端清理有界；浏览器 ICE gathering 可取消并移除监听器。
- 状态：播放后的远端准备回退、短期 progress/媒体统计观测失败保留播放事实；持续本地资源缺失、401/404 与真实 Failed/Closed 按各自合同处理。自动 xHome recreate 保留启动 scope，用户关闭撤销 scope。
- 恢复：transport、browser mediaHealth 与 TURN 共用 connection coordinator，预检查保持纯判定，实际 commit 记一次；授权 1ms/5s/7s 调用链均执行一次，关闭后 grant 归旧资源。SDP/麦克风/配置协商按序执行；新帧连续稳定 5 秒后重置故障 episode，旧帧 refresh 保留独立标记。
- 当前连接预算初值：每故障 episode 最多 2 次已提交连接动作；browser restart 失败后的 full rebuild 计入第二次动作。6 秒 fact/reason 去重沿用基线，稳定新帧窗口为 5 秒，允许帧间隙初值 1.5 秒。原生引擎自主恢复 gate 与预算保持原有主线，公共 host 的动作资格和结果接线继续核验。
- 精简：页面展示由 9 个可变字段收敛为 1 个 reducer 状态对象；Tauri 会话/启动/信令/错误映射拆到 `projection.rs`。flow/scheduler 共用按资源身份的清理；SDP、ICE、connect token 和 keepalive 的网络等待释放生命周期 state 锁。信令事务移到 `session/signaling/flow.rs`；公共 Promise timeout 与协商队列复用，浏览器旧输入空钩子和重复 fact/reason gate 已移除。
- 证据：122 项 Rust 公共层、226 项前端、14 项 Tauri 串流、47 项原生 reconnect lifecycle 回归通过；包含真实回环 HTTP、50 次创建/重复关闭、180 秒状态回退、401/404/429/5xx、旧信令隔离、原生适配和自动重建 scope。iOS 公共调用编译、前端 lint/类型检查及结构调整后的生产构建通过。
- 验收边界：回环 HTTP 验证客户端编排合同；真实 Xbox 服务、Windows/WebView2、macOS/WKWebView、home/cloud × runtime 的实机、2h/8h soak、网络故障注入、性能阈值与 fresh trace 仍待执行。公共生命周期 trace gate 与完整功能映射仍待补齐，整体任务保持 in-progress。

### 2026-10-28 本地验证交接前更新

- 用户明确由其在本地执行实机验证，当前环境可验证 Mac 云游戏；后续根据用户日志继续分析。验收步骤、日志保留和完整功能实现映射见 [本地验收手册](../../dev-docs/streaming-reliability-local-validation.md)。整体任务保持 In Progress，实施 Report 在真实证据收口后产出。
- 恢复总期限：授权、协商和真实新画面观察共用 120 秒初值；浏览器超时即时撤销作用域，原始动作 drain 完成前保留屏障。动作 RPC 完成记录 `actionCompleted`，稳定新画面连续 5 秒后记录 `recoveryObserved/stableMedia`。单调计时覆盖 browser、host、execution、diagnostics 和主机缓存 TTL。
- 原生接线：policy 自动候选和控制请求统一进入 engine 的 `connection_recovery`；receive/domain/健康 gate 保留。当前远端资源资格由既有公共 `decide_recovery` 判定，Tauri 宿主控制台避免重复授权。每故障最多两次实际连接提交；事务保持 single-flight 到稳定画面或明确终止，旧帧刷新及仅解码进展分别保留独立事实。
- 原生排队隔离：RPC 入队前捕获 epoch，Stop 提前撤销旧请求；拿 runtime 锁后校验 epoch/sessionId，Start 替换会话时推进 epoch。捕获身份穿过启动、连接和 chat 协商；重连的排队时间计入控制面总预算。宿主 HTTP 等待支持取消和剩余期限，answer 等待初值统一为 45 秒。
- 远端回收：失败后以 1/2/4/8/16/30 秒退避自动重试，每次清理有 15 秒上界；后台任务使用 Weak，等待期间释放服务强引用。失败后的新调用重新获取凭证，同批并发调用共享结果。关闭与 shutdown 保留当前资源和活动 create 的独立归属，退出等待上界为 30 秒。
- 未知创建结果：网络/解析失败和不可解析资源路径记录原目标/startup generation，同一服务实例内的后续入口阻止同目标 POST，容量上界与活动作用域共享背压。现有网关缺少远端 active-session 查询能力，服务关联、强制终止后的回收及跨启动协调仍等待服务事实；门禁对此输出 `createOutcomeUnresolved`。
- 精简：公共 flow 拆出 startup、cleanup、operation 和 signaling；Tauri service 拆出 context、config_policy、projection；browser runtime 拆出 negotiation 与 supervision。移除未接入生产的 stale ICE helper，使用真实 HTTP 快照去重/结束标志回归替代；合并局部恢复抑制 trace，移除重复连接退避。监督组件持有首帧保护和统计定时器，客户端关闭撤销旧定时器，慢探测保持 single-flight。
- trace：新增 operation/cleanup start-terminal、recovery propose/grant/commit/observe、动作 drain 与远端资源 generation。每个 recorder 的 `traceRunId` 跨轮转保持一致，门禁按运行批次、阶段、runtime generation 和 operationId 配对，并按 seq 复原并发写入。`runtimeBuildInfo` 对浏览器与原生均写出源码 SHA-256，包含当前未提交与新增源码。

本次自动验证：

| 范围 | 当前结果 | 证据边界 |
| --- | --- | --- |
| 公共 xbox-streaming | 130 passed | 真实回环 HTTP、取消晚到、状态回退、清理重试、未知创建阻断、ICE 去重/结束标志；回环测试需沙箱外监听权限 |
| 前端全量 | 31 files / 234 passed | runtime host/native adapter、两种恢复调用链、监督定时器、reducer、既有输入/画面行为 |
| Tauri 全量库测试 | 201 passed | 包含 streaming 14 项及 xbxengine 宿主控制、取消、trace recorder 的测试 |
| 原生 runtime/lifecycle | 104 passed / 6 ignored | 包含自动与控制入口 single-flight、旧 epoch 阻断、既有 reconnect/健康 gate；ignored 场景保留待实机状态 |
| 公共生命周期门禁 | 3 tests / 22 场景通过 | 正负 fixtures、drain、代际、多个运行批次、跨文件轮转与源码摘要匹配 |
| 构建与静态检查 | fmt、lint、vue-tsc、Vite build、Tauri cargo check、iOS bridge check 通过 | 当前 Mac 开发编译；Windows 与实机媒体呈现另行验收 |

当前源码对照 SHA-256：`59c93f67fa6cf6a5690b93d53cd3b852746136692d59d34197944ac8c710e595`，已与本次 Tauri cargo check 的构建输出匹配。Git 基线为 `02f6db40d6138787ca85a582dedf3b5028ad57ea`，工作区包含未提交改动；后续修改源码或构建输入时重新生成摘要。

### P3 维护指标

以下原始行数统计包含拆出的新生产文件，源行数统计排除空行、独立注释及 `cfg(test)` 块。范围为本 RFC 明列的公共会话、Tauri streaming、前端 execution/runtime 文件集合；原生事务和 trace 构建身份改动另列其责任。

| 文件集合 | 原始行数：基线 → 当前 | 源行数：基线 → 当前 |
| --- | --- | --- |
| 公共 flow/scheduler/monitor/store 及新增作用域/启动/清理/信令 | 3,191 → 4,238 | 2,549 → 3,424 |
| Tauri service/types 及新增 context/config_policy/projection | 2,977 → 3,109 | 2,698 → 2,822 |
| 前端 execution/view-state/runtime 及新增公共计时/事务/监督/协商 | 6,374 → 6,771 | 5,980 → 6,361 |

本轮新增了取消、有限等待、后台回收和效果取证合同，关联生产源量增加。结构精简的结果体现在权威与维护范围收敛：flow 门面 2,041 → 1,173，Tauri service 2,030 → 1,148，browser runtime 3,925 → 3,429；拆文件的行数下降单独展示。

| 维护项 | 当前收敛结果 |
| --- | --- |
| UI 操作状态 | 9 份可变展示状态由 1 个 reducer 对象持有，view projection 从权威状态派生 |
| 连接动作入口 | browser transport/media/TURN 归单一 coordinator；原生 policy/runtime/旧 health 归单一 engine 事务；runtime 按现有模式装配 |
| 远端清理实现 | flow/scheduler/shutdown 复用同一资源记录及 cleanup 任务；前后台共用结果锁和 completion revision |
| 生命周期 state 锁与网络 IO | SDP、ICE、connect token、keepalive 的网络等待释放生命周期 state 锁；每资源 negotiation/ICE 锁保持协商顺序 |
| 保留的 IO 串行点 | 原生 runtime 的同步访问、TURN provider 与 ready 主机快照的缓存 single-flight 按现有所有权保留；等待/取消具有明确上界 |

性能差值、长期资源趋势和发布准入值由用户提供同组合实测与日志后固化。实机阶段优先核对已播放会话误超时为零、健康局部故障连接 rebuild 为零、硬断链正确恢复或明确终止，以及正常退出资源清零。

### 2026-10-28 Mac 云游戏首批 fresh trace 验证

- 样本：`runtime-trace-1793169376818-1.jsonl`，14:36:16–14:39:39，macOS / cloud / webrtc-direct / WESTUS3；源码摘要与当前 `59c93f67…` 一致。结果见[独立样本报告](../reports/2026-10-28-cloud-streaming-reliability-trace-validation.md)。
- 已通过：公共启动26.718秒完成；SDP/ICE、关键帧解码和首帧完整，connected→firstPresented=1.204秒；播放区间生命周期门禁通过，startup timeout=0、connection rebuild=0。
- 分窗：首帧后8.179秒可见启动段有7次冻结、3.021秒累计冻结，最大callback gap=1.297秒；后续43.791秒及7.326秒后台窗口分别记录。整体48次/15.097秒冻结包含后台统计，前台稳态需要独立长窗口。
- 已确认观测差异：监督路径使用未合并的client stats，39个恢复诊断窗口decodeFpsAvg=0、jitterMsAvg=-1；展示路径已合并browser RTP。源mediaTime不推进导致sourceFpsEstimate缺失；刷新估算触发90次策略重算，贡献继续定位。
- 结束边界：用户明确强制关闭程序，完整关闭门禁输出cleanupEvidenceMissing/remoteResourceUnclosed。正常退出、服务最终回收、实际断网恢复、V1–V9剩余故障合同、性能与支持组合矩阵继续保留待验，整体In Progress。
