# 公共启动链路深入审查

后续状态：五项问题的代码修复与自动验证已完成，见[修复报告](../../docs/reports/2026-10-30-common-startup-fixes.md)。本文及回放脚本保留修复前证据，当前安全行为回归已进入正式测试。

日期：2026-10-30。结论：确认 5 处逻辑缺口，其中 2 项 P1、3 项 P2。6 组离线回放复现了相关缺陷行为；现有 4 个测试文件的 44 项测试全部通过。本轮交付审查证据与修复建议，生产代码修复待后续实施。

范围：`useStreamExecution`、session progress、runtime host、browser negotiation/runtime、native adapter、Rust startup/cleanup/store/scheduler/signaling。延续此前 home ICE 空快照修复，保留工作区已有改动。

## 1. P1：旧直连启动异常会中断 TURN 交接

位置：`src/streaming/runtime/runtime-host.ts:557–597`；交接入口 `:285–299`，停止逻辑 `:337–374`。

触发：首次 `launch()` 尚在等待 ICE，运行时发出 `failed`，host 开始 TURN fallback；关闭旧 runtime 使原 `launch()` 以 `streamRuntimeSuperseded` 拒绝。

原因：fallback 只更换 `runtimeToken`，沿用 `launchGeneration`。原 `startRuntime()` 的 catch 只检查后者，随后执行 `closeRuntime('launch-failed')`，撤销当前恢复事务并关闭新 runtime。

复现：

- 停止旧 runtime 时立即拒绝：trace 已记录 `fallbackTurnRetry`，TURN `launch` 次数为 0。
- 等 TURN runtime 启动后再拒绝：TURN `launch` 次数为 1，随后收到 `stop('launch-failed')`。

修复建议：为每次 runtime attempt 明确归属；旧 attempt 的失败只结束自身。整体启动失败与 TURN 恢复失败由当前 owner 统一上报，保留关闭/重试的代际保护。

## 2. P1：restart 提交清空去重记录，重新接纳旧 ICE 与结束标志

位置：`crates/xbox-streaming/src/session/signaling/flow.rs:109–112`、`:170–187`；前端完成判定 `src/streaming/runtime/browser-negotiation.ts:264–274`。

触发：远端 GET 暂时保留上一轮 ICE 快照，客户端开始 restart，并提交第一批本地候选。

原因：每个 `submit_ice(..., true)` 都清空 `last_polled_ice` 和 `restart_baseline_ice`；`filter_polled_ice_response()` 忽略 restart 参数。前端本轮候选集合也从空开始，收到旧结束标志后允许完成协商。

复现：首次 poll 返回旧候选和 EOC；重复 poll 正确返回空；提交带新 ufrag 的本地候选后，下一次 poll 又返回 2 项旧数据，其中候选仍带 `old-generation`。旧候选可能被浏览器拒绝；旧 EOC 也可能提前结束本轮收集，使新候选失去轮询机会。

修复建议：以本次协商及 SDP ICE credentials 关联候选和结束标志；旧快照基线在本轮开始时保留一次，增量 POST 保持基线。覆盖真实的 `poll → restart submit → poll` 顺序。

## 3. P2：仅收到 EOC 时进入无期限的连接等待

位置：`src/streaming/runtime/browser-negotiation.ts:256–274`；`src/player/infra/webrtc/WebRtcTransport.ts:148–156`；`src/streaming/runtime/browser-runtime.ts:1810–1815`。

触发：本地收集完成，远端只返回 `a=end-of-candidates`，Peer 保持 `new`。

原因：EOC 直接满足 `remoteExchangeComplete()`，结束 ICE 阶段并释放 45 秒期限。底层 `addIceCandidate()` 跳过 EOC；媒体监督要求已经 connected；host 的初始直连探针仅用于 rust-owned。

复现：真实 browser runtime 配合受控 Peer，协商 Promise 完成；模拟推进 120.1 秒后，poll 为 1 次、恢复动作 0、错误 0，ICE 仍为 `new`，phase 已进入 `connecting`。

修复建议：对零可用远端候选显式结束为失败；将候选交换结束与 transport connected 分成两个有界阶段，并按当前 attempt 处理超时和 TURN 资格。EOC 的标准提交语义一并核对。

## 4. P2：同会话重建 Peer 会接收上一轮 SDP answer

位置：`crates/xbox-streaming/src/session/signaling/flow.rs:37–43`、`:63–66`；`src/streaming/runtime/browser-runtime.ts:1935–1938`、`:2068`。

触发：TURN fallback 或 full rebuild 使用同一远端 sessionId，新 Peer 发送 offer，服务端 GET 暂时仍返回旧 answer。

原因：新 Peer 使用 `restart: false`；服务层仅在 `restart: true` 时读取并过滤旧 SDP。资源 token 证明会话归属，当前实现同时缺少该会话内的协商轮次校验。

复现：同一 session 连续发送两份不同 offer，第二次调用第一次 GET 就接收了携带 `old-generation` 的旧 answer。若随后远端生成新 credentials 或媒体段，这会使当前 Peer 应用错误的 answer，产生握手失败或候选不匹配。

修复建议：让同 session 内的每轮 offer，包括新 Peer 初次 offer，具备可核验的协商归属。结合服务端响应语义处理 SDP version/ICE credentials，避免把 `iceRestart` 布尔值用作“是否存在旧协商”的判据。

## 5. P2：原生恢复 RPC 拒绝后丢失 UI 错误与清理

位置：`src/streaming/runtime/runtime-host.ts:154–169`、`:253–256`；`src/streaming/runtime/xbxengine-runtime.ts:231–238`。

触发：原生 runtime 收到 `failed`，`requestReconnect` RPC 返回异常。

原因：`.then(success, failure)` 的第二个回调只处理输入 Promise 的拒绝；success 内返回的 `tryRecoverByTransportState()` 拒绝继续传播到整条被 `void` 丢弃的 Promise。当前异常分支还存在关闭后检查旧 token 的问题：`closeRuntime()` 已递增 token，后面的错误回传条件失效。

复现：注入一次指定 RPC 错误，观察到 `unhandledRejection=1`、UI state updates=0、UI errors=0、runtime stop=0。

修复建议：对整条 fallback/recovery 链使用统一 catch，按当前 launch/attempt 归属上报一次错误并启动有界清理；清理前捕获上报资格。

## 验证与证据边界

| 验证 | 结果 |
| --- | --- |
| TypeScript 真实模块回放 | 4 项通过，分别断言上述缺陷行为 |
| Rust 临时源码副本 + 本地 HTTP 回环 | 2 项通过，使用真实 public signaling API |
| 现有 runtime-host / browser-negotiation / connection-recovery / runtime-host-policy 测试 | 4 文件、44 项通过 |
| 生产源码改动 | 本轮仅新增审查产物和任务记录 |
| home/cloud 实机启动与恢复 | 本轮证据来自离线回放，实机效果待修复后验证 |

TypeScript 回放复用已有测试夹具，加载当前生产模块。原生 RPC 场景对指定审查错误进行 `unhandledRejection` 精确计数；Vitest 配置只排除该已断言错误，其他未捕获错误继续使回放失败。

Rust 测试在临时副本追加审查用例，生产 Rust 文件保持原状。HTTP 夹具期限由 120ms 调为 5s，给客户端初始化留出时间；回环 socket 需沙箱外执行，命令仅对 localhost 排除代理。最初的权限错误、120ms 超时和代理 502 均发生在夹具环境，最终两项测试在回环请求正常到达后通过。

这些回放使用模拟失败及合成信令。此前日志里三次 home 启动的具体因果仍以各自 trace 为准，本轮发现说明额外可触发的失败路径。

启动回放尚未进入真实媒体阶段；网络质量、恢复链、media supply、steady supply、submit/present 长尾及帧供给 delta 均无实机样本。修复后需通过新 trace 检查首包、首帧、稳定供给和退出清理。

现有测试缺口：restart 快照去重测试省略本地 POST；host 缺少直连 Promise 与 fallback 并发；EOC 用例均同时提供可用候选；native recovery 只覆盖成功的 RPC。

## 已有约束

`createOutcomeUnresolved` 在同服务实例内阻止同目标重试，当前没有服务端 active-session 查询来解除未知创建归属。这是 [公共可靠性 RFC](../../docs/rfcs/2026-10-28-streaming-common-reliability-simplification.md) 明确保留的限制，单列为后续产品闭环项。

## 复现产物

- [结构化观测与源码 SHA-256](2026-10-30-common-startup-audit.metrics.json)
- [TypeScript 回放入口](2026-10-30-common-startup-audit.cjs)：`node dev-docs/validation/2026-10-30-common-startup-audit.cjs`
- [Rust 副本准备脚本](2026-10-30-common-startup-audit-prepare.py)：`python3 dev-docs/validation/2026-10-30-common-startup-audit-prepare.py`，执行输出的格式化及测试命令。
- [Rust 回放用例](2026-10-30-common-startup-audit.rs)

建议按 1 → 2/4 → 3 → 5 的顺序修复，将缺陷回放转换为安全行为回归，并补一次 home 直连失败转 TURN 的完整实机 trace。
