# 串流公共主线本地验收

关联：[可靠性改造 RFC](../docs/rfcs/2026-10-28-streaming-common-reliability-simplification.md)。2026-10-28 用户接管实机验证，当前可用组合为 Mac + 云游戏。自动回归覆盖客户端合同，整机结论由本次 fresh trace 补齐。

## 当前代码状态

已实现启动取消、排队/准备独立期限、播放期状态回退保护、当前资源身份校验、统一清理与通信恢复后的后台回收。浏览器与原生各在现有 runtime 内装配一个连接事务：授权、动作提交和稳定新画面观察分别取证；每故障最多两次实际动作，总期限初值 120 秒，稳定窗口 5 秒，帧间隔上限初值 1.5 秒。原生自动候选继续受 receive/domain/健康 gate 约束，控制请求与自动候选共用 engine 事务。

原生控制请求在进入 blocking 队列前捕获 epoch，拿锁后核对 epoch 和 sessionId；Stop 提前撤销正在等待的请求。offer、ICE、keepalive、麦克风协商使用原 epoch 及剩余期限；排队时间计入控制面重连的总预算。浏览器首帧保护定时器随客户端回收，周期统计探测保持 single-flight。

创建响应的网络/解析结果未知时，客户端保留当前服务实例中的目标记录，阻止同目标继续 POST，并记录 `createResultUnknown/retryBlocked`。现有服务网关缺少远端 active-session 查询能力；该情形的服务关联、进程强制终止后的远端回收及跨启动协调继续等待服务证据。门禁将这种记录列为验收阻断，所有远端关闭结论均需要可用的资源身份。

## Mac 云游戏建议顺序

1. 使用当前工作目录构建并启动应用，保持已登录账号和可用云游戏。先选择 `webrtc-direct`；原生运行时作为第二批独立测试。记录游戏、网络、runtime、画质/超分、麦克风设置和开始时间。
2. 打开运行日志，短故障测试可选 dev，连续播放使用 production。保留 lifecycle、网络、恢复、显示和统计的默认诊断维度。运行日志路径见下节。
3. 首轮进行 10 次正常启动/退出和 3 次准备中取消。检查每次正常首帧、声音、手柄、菜单、软 Home、中性释放，以及退出后下一次启动。实际出现云排队时单独测试取消与继续排队。
4. 连续播放至少 30 分钟，切换全屏、菜单、音量/静音、画面设置、实验超分和麦克风。窗口最小化后恢复，记录黑屏、冻结、音轨残留、持续输入与错误时间。
5. 分别测试短断网恢复、持续断网后恢复、断网期间退出；追加睡眠/唤醒和麦克风重协商。每次记录断开、恢复、画面恢复或终止的时间与错误文案。确认新画面恢复和后续操作可用。
6. 首轮日志通过后，按 RFC 扩展到 50 次启动/退出、20 次受控网络故障、2 小时连续播放及发行路径 8 小时 soak。原生运行时单独记录结果；Windows、主机串流等支持组合分别保留待验状态。

这些步骤用于分批取得证据。完整发行验收继续按 RFC 的 P4 矩阵执行。出现 `InvalidCountry`、登录失效或远端服务拒绝时，保留错误原文与日志时间，服务可用性和客户端超时分别分析。

## 日志保留与版本身份

Mac 默认目录：

```text
/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs
```

每批测试正常结束串流并退出应用后，保留该批所有 `runtime-trace-*.jsonl`。生产日志保留预算为 16 MiB × 5 个文件，dev 为 64 MiB × 10 个文件；长测期间定期将整个目录合并复制到同一个验收目录，保留已轮转文件，并用后续版本覆盖同名的仍在写入文件。测试结束再完成一次复制，使操作开始、结束、清理和恢复结果同时存在。

新行包含 `traceRunId`，同一运行批次跨轮转文件保持一致。`runtimeBuildInfo.buildFingerprint` 包含 Git revision、dirty、构建时间、profile 和 `source_fingerprint_sha256`；源码摘要涵盖当前前端、Rust、公共 crate、依赖锁、构建设置和工具链文件，包含未提交与新增源码。每次串流启动均记录构建身份，浏览器和原生路径使用同一取证入口。

日志交接同时附上测试组合、起止时间和异常发生时间即可。当前仓库中的历史日志只承担历史诊断；本次验证以新构建写出的记录为依据。

## 收到日志后的分析顺序

在仓库根目录执行，将 `TRACE` 替换为实际文件。多文件生命周期门禁支持整批文件输入，并按 `traceRunId/seq` 还原顺序。

```sh
python3 -B .agents/skills/analyze-runtime-logs/scripts/summarize_runtime_trace.py TRACE
python3 -B .agents/skills/analyze-runtime-logs/scripts/trace_streaming_session_lifecycle_gate.py TRACE --require-cleanup --require-closed
python3 -B .agents/skills/analyze-runtime-logs/scripts/trace_browser_webrtc_behavior_report.py TRACE
```

故障恢复批次给公共门禁追加 `--require-recovery`。构建对照追加 `--expected-revision REVISION --expected-source-sha256 SHA256`，近期取样追加 `--max-age-seconds`；长测按实际持续时间设置新鲜度窗口。`--allow-active` 只用于正在播放的诊断，完整关闭验收使用默认完整生命周期模式。

原生路径追加：

```sh
python3 -B .agents/skills/analyze-runtime-logs/scripts/trace_receive_feedback_report.py TRACE --fail-on-gate --require-media-recovered --require-display-stable
python3 -B .agents/skills/analyze-runtime-logs/scripts/trace_midsegment_report.py TRACE
python3 -B .agents/skills/analyze-runtime-logs/scripts/trace_webrtc_acceptance_gate.py TRACE --require-lifecycle-reconnect-gate
```

根据实际场景选择恢复或健康网络 gate：正常播放提供 steady 证据，恢复场景证明 ReferenceChain、RecoveryLedger、cleanAnchorCommitted、DisplayStable。公共门禁核对 operation/cleanup 配对、动作授权与次数、停止后的旧提交、误超时、未知创建结果及挂起动作 drain。

每份结论同时覆盖网络健康、恢复链健康、media supply、steady supply、submit/present 长尾和帧供给 delta。浏览器实际呈现使用回调/draw 等可用代理量，真实上屏与 glass-to-glass 留给外部测量；记录代理量的证据边界。

## 功能保留映射

| 功能 | 现有实现归属 | 自动证据与本地验收 |
| --- | --- | --- |
| 主机唤醒、预检查、显式注册补偿 | `session/startup.rs`、Tauri `context.rs` | 公共流程/重建 scope 回归；主机实测待提供设备 |
| 云排队 ETA、准备与随时取消 | startup、monitor、scheduler、execution reducer | 排队与准备预算/取消测试；Mac 云游戏实测 |
| connect token、媒体/chat SDP、增量 ICE、结束标志 | `session/signaling/flow.rs`、`browser-negotiation.ts`、原生 lifecycle | 旧响应/取消/重复快照与结束标志回归；直连、relay、IPv4/IPv6 分批取样 |
| 凭证刷新、保活、真实远端终态 | 公共 scheduler、cleanup | 401/404/429/5xx 分类与通信恢复回收回归；长播放/远端结束 |
| 首帧、音视频与连接恢复 | browser supervision/coordinator、原生 connection_recovery | 快慢授权、并发提交、新帧效果、原生 domain/健康 gate；黑屏/断网/睡眠实测 |
| GamepadBackend、菜单 owner、中性释放、震动、软 Home | 既有统一输入后端、Rust input release、runtime 适配 | 前端输入回归/原生控制回归；实测重连和退出后的卡键、震动与 Home |
| 键盘/触控、文本与远端控制 | 既有 player/controller/RPC | 保留原能力入口；按目标支持能力逐项验收 |
| video/WebGL、CAS、色彩、超分与分辨率 | 既有 browser render policy、原生 media/render | 原 renderer/profile/SR 回归；设置、尺寸变化与冻结/恢复 |
| 音量、静音、麦克风/chat | runtime、串行协商队列 | 麦克风和配置顺序、取消/晚到回归；音轨释放与恢复用户意图 |
| 诊断、截图、错误和跨平台公共调用 | diagnostics、projection、trace、iOS bridge | lint/类型/构建、生命周期 fixtures、iOS 编译；面板与截图实测 |

实施 Report、性能准入值和整体任务收口将在本地日志完成后写入。优先提供 Mac 云游戏 `webrtc-direct` 首轮正常退出与断网恢复两批完整日志。
