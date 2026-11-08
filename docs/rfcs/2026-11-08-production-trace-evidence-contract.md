# 生产 Trace 连续证据与诊断合同 RFC

## Status

- Completion: P1/P2/P3 代码与自动验证完成；Windows 掌机验收待用户执行
- Current State: validation-pending
- Owner: 主窗口直接执行，单窗口
- Last Updated: 2026-11-08
- Task: `2026-11-08-production-trace-evidence-contract`
- 执行顺序：P1 连续证据与消费合同 → P2 输入结果与降噪 → P3 异常窗口与结束闭环。

## Background

用户掌机持续掉帧，两个串流菜单打开后触屏选项与手柄导航失效；锐度 0 的实机对照明显缓解持续掉帧。原 [trace](/Users/guo.xu/Downloads/runtime-trace-1794116093651-1.jsonl) 有 377 行、270,080 字节、205.477 秒，全部为 production。

| 实际观测 | 数量 | 设计意义 |
| --- | --- | --- |
| `sessionStatePolled` + `sessionMonitorSnapshot` | 212 行 / 120,820 字节；56.23% 的行、44.73% 的字节 | 相同控制面状态占用大量记录 |
| 稳态 getStats / renderTelemetry | 各 0 行 | 持续帧率、解码/显示供给差、CPU/GPU 长尾缺证 |
| 单文件预算使用率 | 1.61%，预算 16MiB | 现有预算足以容纳更有价值的连续证据 |
| `traceBudgetNotice` | 0 行 | 本次周期缺证对应前端 production 过滤策略 |
| 文件最后事件 | `cleanupStarted` | 停止原因、末段累计值与写入闭环需要尾摘要 |

安全复算与当前源码身份见 [design metrics](../../dev-docs/validation/2026-11-08-production-trace-design.metrics.json)；[audit 脚本](../../dev-docs/validation/2026-11-08-production-trace-design-audit.py)支持独立复算。

### 当前代码的具体缺口

1. [browser-runtime-trace.ts](../../src/streaming/runtime/browser-runtime-trace.ts) 在生产模式提前过滤详细 stats、renderTelemetry、原因分类等观测，常规保留事件统一使用 `frontend/essential`。本轮已补 1Hz 的 `browserPlaybackHealthObserved`，提供基础补救。
2. [StatsService.ts](../../src/player/app/stats/StatsService.ts) 使用唯一 250ms 采集时钟；当前健康摘要的部分帧率来自最新采集区间，3s/15s 策略窗已有聚合事实。回调计数与 max gap 在策略刷新时归集并清零，健康摘要另有 1s 限频。这些窗口边界需要显式定义，持续健康记录应覆盖所有已提交采样区间。
3. [policy.rs](../../src-tauri/src/mods/runtime_trace/policy.rs) 的 production 固定维度包含八项，默认排除 input；essential 有维度旁路。导航引擎与 [Focusable.vue](../../src/navigation/core/vue/Focusable.vue) 的点击失败入口也缺少结构化诊断。
4. [rpc.rs](../../src-tauri/src/mods/runtime_trace/rpc.rs) 的 `recordEvent` 始终返回 `accepted=true`；recorder 的过滤、超大行、队列压力和 writer 实际写入是独立阶段。
5. [Browser behavior report](../../.agents/skills/analyze-runtime-logs/scripts/trace_browser_webrtc_behavior_report.py) 依赖详细 `browserWebRtcStatsObserved`。新增生产摘要需要同步消费适配，才能完成自动分析闭环。
6. Browser runtime 的 trace 配置读取超时会配置为 off。生产策略应由 Rust 的 effective profile 提供，并记录生产者的配置与采集状态。
7. Rust 的两类控制轮询把 sessionId 放入 payload，envelope 使用 None。会话过滤与跨层关联需要同一会话 ID 的统一入口。

历史约束沿用 [profile/budget RFC](2026-06-24-runtime-trace-profile-budget.md) 和 [writer/input hardening RFC](2026-11-05-render-trace-input-hardening.md)。已有菜单、USM 和生产基础摘要补丁保留，交付见 [掌机修复报告](../reports/2026-11-08-handheld-stream-menu-and-frame-trace.md)。

## Goal

一份正常导出的生产 trace 应能回答：配置实际应用到了哪条路径；帧在哪一段减少；卡顿何时开始、持续多久；恢复动作是否有效；输入到达了哪个处理层；日志本身有多少有效覆盖。

验收同时覆盖网络健康、恢复链健康、media supply、steady supply、submit/present 长尾和帧供给 delta。每个结论附有效样本数、时间窗、数据来源与证据状态。

## Scope

- 前端观测：既有 Browser StatsService、runtime trace、renderer 数值计时、串流菜单与几何导航的结果投影。
- Rust：既有 runtime_trace policy/recorder/writer/RPC，session monitor 的生产记录聚合，启动与结束时的摘要装配。
- 消费：生产健康报告、现有 Browser behavior report 的适配、schema/playbook 与契约回放。
- 运行边界：Vue + TypeScript、Tauri + Rust、现有 Browser WebRTC、唯一采集时钟、几何导航与单一 writer。
- 媒体决策、网络反馈、恢复策略和控制器路由遵循原有合同。Rust WIP 通过 capability 字段声明可用观测，专项接入沿 `rust` 开发线演进。

## Decisions

### 1. 生产事件采用显式合同

沿 schema v3，payload 增加 `productionContractVersion=1`。Rust policy 保持 effective profile、事件等级、生产准入与预算的权威；前端通过既有 RPC 获取该代次的有效策略，缓存后执行惰性构造与本地限频。新/改事件使用显式 descriptor，包含维度、重要性、字段白名单、采样规则和行预算。跨语言契约使用相同 fixture 验证。

| 事件 | 保留规则 | 核心作用 |
| --- | --- | --- |
| `fileOpened` / `runtimeBuildInfo` | essential，文件/构建边界 | profile、合同版本、预算与源码身份 |
| `playbackObservationManifest` | key，启动/代次/能力改变 | 实际配置、采样周期、能力、clock anchor 与有效策略 |
| `browserPlaybackHealthObserved` | key，每秒整窗摘要 | 连续网络/供帧/渲染/恢复事实 |
| `browserPlaybackAnomalyObserved` | key，进入/升级/退出；硬故障 essential | 异常起点、持续时间、触发事实与效果关联 |
| `browserPlaybackAnomalySample` | key，有界 4Hz 窗口 | 异常前后细节，绑定 episode/sampleId |
| `inputUiHealthObserved` | input/key，弹层可交互期间最多 1Hz | 输入采样、UI 归属、Scope 与焦点有效性 |
| `inputUiInteractionObserved` | input/key，操作结果；异常立即记录 | 触屏/鼠标/手柄意图 → 聚焦 → handler → 异步操作结果 |
| `sessionControlStateObserved` | lifecycle/key，首次/变化/15s 心跳/末次 | 合并轮询事实与派生会话状态 |
| `playbackSessionSummary` | 30s key 检查点，正常结束 essential | 累计值、覆盖、异常、停止与清理结果 |
| `traceBudgetNotice` | 沿既有事件扩展分层计数 | 生产者、IPC、recorder 和 writer 的证据损失 |

握手、认证、HTTP 与会话操作继续保留关键开始/阶段/结果及 operationId。Browser timeline/SDP 和 offer/result 保留既有事件名，payload 采用固定摘要：codec/profile、协商尺寸/码率、反馈扩展、候选类型/协议、阶段耗时、结果码与安全关联 ID。

维度归属使用显式 descriptor；混合健康摘要归 presentation，内部各段使用固定对象。production 加入 input 维度，并仅准入本轮定义的关键输入事件；逐轮询的原始输入沿 dev/raw。

会话内事件统一填写 envelope.sessionId，payload 保留运行时/资源代次。每个已关闭健康 bucket 分配独立 `healthSequence`，结合 bucket 范围识别生产记录缺口；writer 的 seq 继续表达实际写入顺序。

### 2. 每秒覆盖完整观测窗

复用已提交的 250ms Browser 样本和已有回调/renderer 数值事实，由观测聚合器形成非重叠的约 1s bucket。每条健康记录包含 bucket 起止、实际 elapsed、区间样本数、有效覆盖、首末 sampleId 与各代次。限频在 bucket 发布阶段执行；所有采样区间先进入聚合。

- 累计计数用于追溯；delta 按同一 peer/video/renderer 身份和真实 elapsed 计算。代次切换、计数重置、缺采样产生 partial/reset 状态和原因。
- received/decode/videoPresent/callback/rendererSubmit 的整秒 FPS 按本桶 delta/elapsed 计算；最新 250ms 值和 3s/15s 窗口分别标明 scope。采样缺失保留 unavailable；有效采样中的 0 保留零值。
- 回调 gap、跳帧、最大间隔和 dropped-like 分别累计。renderer overwrite、用户限帧、重复/提交失败各自保留计数与原因。
- CPU/GPU P95/P99 使用原始数值样本或既有计时环的指定范围计算。每组分位数附 count、window 和来源；GPU pending/disjoint/unavailable 状态保留。已有滚动分位数采用自身真实窗口标签。
- 心跳由现有 supervision 的同步 tick 发布已缓存观测，在异步探测等待期间继续说明 `pending/stale/unavailable`。发布只读取已缓存事实；控制路径继续使用原有样本和清零边界。

`playbackObservationManifest` 声明配置和能力：请求/协商/实用分辨率与 FPS、实际 video/WebGL2/SR 路径、USM/CAS、锐度/色彩、码率上限、decoderImplementation、powerEfficientDecoder、GPU timer 状态、窗口可见性与 display refresh 来源。配置改变记录前后值、原因和操作结果。

### 3. 健康摘要的六段最小字段

| 维度 | 每秒必须保留 | 判定依据 |
| --- | --- | --- |
| 网络健康 | RTT/jitter/loss、接收 bytes/packets delta、实际码率、jitter buffer、packet age、选中路径类别 | 区分网络供给变化与本地耗时 |
| 恢复链健康 | decoded/keyframe/freeze/drop 累计与 delta、PLI/FIR/NACK、控制通道、动作/结果/operationId | 请求事实和结果事实各自标明可观测能力 |
| media supply | received/decode FPS、decode time、源尺寸、decoder 与样本新鲜度 | 确认供帧与解码是否覆盖目标 |
| steady supply | videoPresent/callback FPS、回调 gap/jump/最大间隔、3s/15s 覆盖和源节拍 | 区分回调节奏、批量推进和持续供给缺口 |
| submit/present 长尾 | draw/upload/submit/GPU 的 P95/P99/max/count、submit/videoPresent age | 定位上传、CPU 提交、GPU 与上层回调阶段 |
| 帧供给 delta | 五段累计/整窗 delta、overwrite/rateLimit/duplicate/failure、有效目标 FPS | 计算相邻阶段差，解释主动释放与失败 |

Browser 的 `videoPresent` 使用 RVFC/播放质量事实；物理 scanout 标为 unavailable。Browser 内部 NACK/关键帧恢复仅使用公开统计与已观测的解码推进；Rust 的 ReferenceChain、cleanAnchorCommitted、DisplayStable 使用专项 capability 和 trace gate。

### 4. 输入记录必须说明执行结果

在既有 InputDispatcher、geometric engine、Focusable 与菜单 handler 的实际分支投影结果，沿用当前导航执行链。

`inputUiInteractionObserved` 包含固定来源 `touch/mouse/gamepad/keyboard`、意图、静态菜单/action key、输入归属、top scope、当前焦点、target 有效性、结果和耗时。失败原因使用枚举：`disabled`、`inactiveAncestorScope`、`outsideTopScope`、`focusUnavailable`、`noCandidate`、`routingPaused`、`handlerFailed` 等；异步设置/会话动作绑定同一 operationId，区分 handlerInvoked 与 operationCompleted。

`inputUiHealthObserved` 聚合实际 poll/publish Hz、poll gap 分位数、UI intent/click/成功/失败计数、Scope active 栈深度与有效性。重复方向导航按秒合并，确认/返回与 Scope 改变保留事件。触屏来源复用 pointer 事件上下文，手柄确认保留原始意图来源。

Scope/焦点结果复用导航已执行的检查；枚举和计数构造保持轻量。动态页面元素使用会话内安全引用，日志只包含静态菜单键、路由键及数值诊断。

### 5. 用状态聚合压缩重复轮询

production 由同一 monitor 观测生成 `sessionControlStateObserved`，包含远端 streamState、派生 phase/playerState、runtimeLaunchState、关键闩锁、错误分类、轮询次数/失败数与本窗耗时。首次、语义变化和失败立即写入；相同状态每 15s 写一条心跳，末次摘要保留累计值。轮询调度与会话行为沿现有实现。

当前文件按首次/变化/15s 心跳/末次边界回放，212 条控制轮询可映射为 11 条，保留 2 次 monitor 语义变化。这是设计回放的数量估计；实际 emitter、边界清理与文件字节变化在实施后验收。

### 6. 有界异常窗口

在现有采样时钟上维护最多 60 个紧凑数值样本，单个 ring payload ≤1KiB、总计 ≤60KiB。触发后保留前 5s + 后 10s 的约 4Hz 样本，逐条 production 行 ≤2KiB；相同 episode 合并触发原因。

初始触发合同：同代次且有效覆盖的 3s 供给窗低于有效目标 90%、freeze/failure 累计增加、回调/submit age 超过 100ms、持续采样缺失或菜单交互失败。目标来自已观测源节拍、用户 FPS 上限与实际 renderer 预算；阈值和有效目标记录在 manifest。该检测只产生诊断事件。

每分钟最多开启一个详细 episode，额外预算 ≤128KiB/min。重复/持续异常由每秒摘要、episode 计数和状态改变持续表示。预窗口不足、source reset、采样缺失、预算合并分别标记 coverage/reason。

### 7. 预算与真实写入反馈

| 层 | 合同 |
| --- | --- |
| 健康行 | 最多 1Hz，完整 JSONL 行 ≤4KiB，单 lane ≤240KiB/min |
| 正常稳态串流 | 总写入目标 ≤320KiB/min；菜单持续输入另有 ≤64KiB/min 预算 |
| 异常窗口 | 额外 ≤128KiB/min，单样本行 ≤2KiB，最多 60 条 ring |
| 前端发布队列 | 沿用 4 在途/32 排队，增加字节记账；累计排队 ≤256KiB，关键结果保留余量 |
| Rust writer | 沿用 8192 行/8MiB、64KiB 单行和重要性余量；16MiB×5 文件 |
| 生产采集成本 | 额外 getStats/DOM/像素回读为 0；固定字段、先准入再构造 |

数值预算为实施验收目标。按当前源码模块回放测量构造/序列化时间及字节，并在 Windows 掌机确认额外成本；健康摘要构造+编码 P95 目标 ≤0.5ms/次。队列压力优先合并旧摘要和异常细节，essential 首末状态与动作结果使用现有余量。

`RuntimeTraceAckResult` 保留 accepted 字段并增加明确 disposition：`enqueued/filtered/queueFull/rowOversized/writerUnavailable`。accepted 描述 Rust 入队结果；written/IO drop 由 writer 计数与检查点确认。前端另计 profileFiltered、coalesced、IPC failed/timeout；主动聚合和证据丢失分别计数。

### 8. 覆盖与结束也是生产证据

producer manifest 声明 expectedCadence、能力、profile/configuration 状态、runtime/client/peer/video/renderer 代次及各 clockDomain 的墙钟/单调时钟 anchor。每秒记录采集样本 age、成功/缺失/过期/等待状态，epoch 切换明确分段。

30s 检查点和末尾 `playbackSessionSummary` 保留会话时长、received/decoded/present/callback/submit 累计、各类 drop/freeze、异常时长、有效覆盖、日志各层计数和终止原因。正常结束顺序为生产者尾摘要 → 既有 cleanup → `cleanupFinished`/结果 → writer 的有界 sync 回执。沿用现有 sync 机制记录 flushed/timeout/ioError，前端收尾有明确期限。

轮转文件保留 runId、文件序列、seq 连续性和当前检查点，导出携带同 run/session 的有序片段清单。突然关闭的文件由分析器标明 partial ending，并使用已有检查点与最后健康样本解释有效窗口。

### 9. 生产分析报告同步交付

在现有 `analyze-runtime-logs` 工具中增加生产合同适配与健康报告，旧 dev 事件和新 production 摘要投影为相同分析模型。报告输出六段结论、阶段 FPS/累计 delta、P95/P99/max、异常 episode、输入失败及证据覆盖。

- trace 发布覆盖与有效 media 采样覆盖分别计算；前者说明日志记录，后者说明性能结论的数据基础。
- 正常前台 steady 窗口至少 60s，健康摘要覆盖目标 ≥95%，连续缺口 ≤3s；有效媒体覆盖低于 90% 时性能验收为 INSUFFICIENT_EVIDENCE。
- 输出 `PASS / FAIL / INSUFFICIENT_EVIDENCE / UNSUPPORTED`，按 backend capability 验证恢复与物理 present。
- 配置/renderer/peer 改变分别分段，结合源 FPS 与实际 renderer 上限解释帧差。
- 新合同 fixture 固定覆盖 source 30fps、用户限帧、解码慢、上传慢、GPU 慢、回调稀疏但 presented 推进正常、日志丢失与菜单失效等场景。

固定字段白名单在生产者与 Rust 发布边界验证，包含数值、固定枚举、安全操作 ID 和配置结果；地址/URL 使用协议/路径类别，账号/设备/动态 UI 元素使用会话内引用。密钥、授权原文、完整 SDP/HTTP body、用户文本的隐私扫描沿现有 skill 执行。

## Plan

1. **P1 连续证据与消费合同**：Rust event descriptor/effective policy/ack；Browser 完整秒窗、采样状态、manifest 与时钟；分析器新旧适配及字节/覆盖 gate。
2. **P2 输入结果与降噪**：input 生产准入；导航、点击及异步 handler 的结果投影；session monitor 聚合；timeline/SDP/offer/result 固定摘要。
3. **P3 异常窗口与结束闭环**：有界 4Hz ring、episode 与预算；30s 检查点/末尾/flush 状态；轮转导出与统一验收报告。

每阶段由主窗口执行、审核并验证，保持现有菜单/USM/基础摘要补丁。main 开发构建暂停策略沿现有约定；Windows 掌机启动、串流和长测由用户验收。

## Validation

- [x] 原 trace 的 SHA、行数、字节、事件分布与预算复算。
- [x] 15s 控制面聚合方案离线回放，保留两次语义变化，预测 212→11 条。
- [x] 当前 producer、Rust policy/RPC/writer、输入入口和分析器依赖审计。
- [x] 真实生产模块回放：250ms 瞬时缺口进入整秒摘要；所有区间 delta/max 完整、跨代次 reset 正确。
- [x] pending getStats / IPC failure / writer overflow / profile change 各自有可解释覆盖与真实 disposition。
- [x] 健康 30min、持续异常、连续导航压力回放通过行/字节/ring/排队硬预算。
- [x] 两菜单真实组件旧父 Scope 回放输出 `inactiveAncestorScope`；修复版触屏/鼠标/手柄成功结果可配对。
- [x] 网络、源供给、解码、回调、CPU 上传/提交、GPU 及日志缺证 fixture 的分类/分位数/覆盖正确。
- [x] production/dev/off 的媒体提交与输入控制序列一致；额外 getStats/DOM/像素诊断读数为 0。
- [x] 隐私白名单、版本兼容、轮转、尾摘要和有限期限 sync 测试。
- [x] Rust `cargo fmt`、相关 tests/check；前端 `pnpm lint:fix`、tests/typecheck/build；Python 报告测试和 diff/link 检查。
- [ ] Windows 掌机 1440p/60fps、锐度 0/3 各至少 60–90s，正常结束导出；六段指标与菜单结果可完整复盘。

## Risks

- Browser/GPU/decoder 公开观测存在平台差异；能力和 unavailable 保持显式，专项 gate 使用对应 backend。
- 常开连续观测与异常记录增加少量成本；行/内存/速率预算及 profile 对照验证控制开销。
- event/ack 合同调整影响多个生产者和旧消费者；schema v3 加法字段、共享 fixture 和逐阶段适配保持兼容。

## Progress

- [x] 掌机事故证据与当前实现缺口审计。
- [x] 生产合同、字段/窗口、预算、输入结果和验收设计。
- [x] 用户确认按方案实施 P1/P2/P3。
- [x] P1/P2/P3 实现与定向验证。
- [x] 自动验证完成，结果见[验证记录](../../dev-docs/validation/2026-11-08-production-trace-validation.md)。
- [ ] 新 Windows 实机 trace 验收，完成后落最终 Report。

## Execution Notes

- 2026-11-08：用户要求重新设计 production trace。完成 read-only 事故复算和当前代码审计，新增 RFC、audit 与安全 metrics；生产代码实施待确认。当前每秒健康摘要及菜单/USM 补丁继续保留。
- 2026-11-08：用户选择“按方案实施”，授权完整三阶段实现与验证；主窗口开始执行。
- 2026-11-08：P1 完成共享 TS/Rust 生产事件合同、完整 1Hz bucket、manifest/epoch、采集状态、真实入队 disposition 和六段分析器。每秒保留核心累计、全量区间 delta 与有效覆盖，30s 检查点及结束摘要保留完整累计和 writer 状态；相同 3s/15s 覆盖使用紧凑字段，使最大健康 JSONL 行为 3,744 字节。
- 2026-11-08：P2 完成触屏/鼠标/手柄/键盘来源、菜单 handler 和异步结果配对、Scope/焦点失败枚举、连续导航 1Hz 聚合、15s 控制面心跳与 SDP/timeline 固定摘要；晚到结果保持启动时 clientGeneration。既有父 Scope 与 USM 9→5 采样修复持续保留。
- 2026-11-08：P3 完成前 5s/后 10s 的 4Hz 有界异常窗口、30s 检查点/末段尾摘要、生产者 drain/清理/writer 有限期限 flush、同 run 有序轮转导出；诊断菜单及失败页提供“导出运行日志”，取消保存记录 cancelled。
- 2026-11-08：最终前端 77 文件 / 709 项、Rust xbxrc 120 项 + xbox-streaming 141 项、Python 36 项通过；fmt/lint/typecheck/build/check 完成。独立 `pnpm test:trace-production` 成本门通过，30min 正常写入约 222KiB/min，P95 0.145ms，菜单压力约 56.7KiB/min。全量功能回归持续检查字节/行/ring/队列预算，成本门独立运行以固定 CPU 测量条件。
- 2026-11-08：原日志六段均为 INSUFFICIENT_EVIDENCE；真实生产模块合成回放的六段与性能 gate 均 PASS，五段均 60fps，发布覆盖 100%、有效媒体覆盖 99.99%。回放沿生产者写入入口构造 JSONL，cleanup/writer 回执由专项测试覆盖；该回放文件的 acceptanceGate 保持 INSUFFICIENT_EVIDENCE，ending 明确 partial。Windows 实机验证继续按本 RFC 最后验收项执行。
