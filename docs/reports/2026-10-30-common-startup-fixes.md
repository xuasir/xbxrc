# 公共启动五项问题修复

日期：2026-10-30。用户授权修复[审查报告](../../dev-docs/validation/2026-10-30-common-startup-audit.md)列出的五项问题；代码修复及自动验证已完成，实机 home/TURN 启动效果待新 trace 验证。

实机跟进：[14:39 新日志](../../dev-docs/validation/2026-10-30-home-startup-fresh-trace.md)与当前源码指纹一致。home 前两次 ICE 空快照后继续轮询，第三次拿到候选并直连成功，connected 后 526ms 首帧，约91秒后正常退出且资源清零。TURN/restart/Peer重建/原生恢复异常仍待对应故障场景；SR供给缺口已由video回退吸收。

## 修复结果

| 问题 | 当前行为 | 回归证据 |
| --- | --- | --- |
| 旧直连 launch 拒绝撤销 TURN | launch 的异常先核验 runtime token 和 launch generation；被替换的 attempt 结束自身，当前 TURN 继续执行 | 旧 stop 中立即拒绝、TURN launch 后晚到拒绝两种时序均保留新 runtime |
| restart POST 清空去重 | 新 ICE 轮次在 offer 开始时保存一次候选基线；多个本地 POST 保持基线；候选按当前 answer ufrag 和旧快照过滤 | 两个增量 POST 后旧候选持续被拦截；同地址新 ufrag 正常通过；混合代际快照的旧结束标志被拦截 |
| EOC-only 无期限等待 | 可用候选为零时抛出 `remoteIceCandidatesMissing`；当前 SDP 内嵌候选也计入可用性；EOC 按标准空 candidate 传入 Peer | 纯结束标志及时失败并清理监听器；SDP 已内嵌候选时正常完成；底层收到带正确 mid/index 的结束标志 |
| 新 Peer 接收旧 SDP | 新媒体 ICE 凭证触发基线校验，覆盖 `restart=false` 的 Peer 重建；按 media/chat 分别保留上次 answer | 新 offer 的首次旧 answer 被忽略，后续新 answer 被接受；同 Peer 画质协商和 chat 保持媒体 ICE 代际 |
| 原生恢复异常遗漏收尾 | fallback/recovery 整条 Promise 链统一 catch；先确定当前错误归属，再撤销 token、通知 execution 并清理；清理失败独立记录 | 原生 RPC 拒绝只上报一次；stop 拒绝被消费并保留可重试资源；新 runtime 启动后的旧 RPC 拒绝被吸收 |

browser runtime 完成信令后，host 对首次 connected 等待设置 15 秒期限。直连耗尽时使用既有恢复事务尝试一次符合资格的 TURN；TURN 仍超时则结束执行。connected、stop、替换都会清理该计时器。原有 offer/ICE 45 秒阶段期限及恢复总期限继续生效。

## 实现边界

- 复用现有 runtime token、session cancel token、SDP/ICE 锁和恢复事务。公共 RPC DTO、runtime 装配及媒体处理边界保持既有合同。
- Rust 新增 `session/signaling/state.rs` 保存协商基线，原 flow 内部状态移入该模块；候选提交和读取在等待 ICE 锁后复核入队时的代际。
- 媒体 offer 的 ICE credentials 发生变化或明确 restart 时建立新 ICE 轮次；同 Peer 的画质协商允许复用兼容 answer，chat 的 answer 记录独立保存。
- 远端 answer 明确复用 ICE ufrag 时允许复用同地址候选。缺少 ufrag 且与旧基线完全相同的快照按保守策略等待，最终受现有阶段期限约束。
- 新诊断为 `initialConnectionTimedOut` 与 `runtimeCleanupFailed`，继续使用现有 recorder；SDP、候选和凭证保留在内存中的有限状态，新增诊断只包含期限和错误信息。

## 验证

- 新增 22 项回归：runtime host 9 项、browser negotiation 2 项、WebRTC transport 1 项、Rust signaling state 6 项、Rust HTTP signaling 4 项。参数化用例覆盖额外时序。
- 前端串流及 WebRTC transport：23 文件、216 项通过。
- Rust `xbox-streaming --lib`：140 项通过。HTTP 用例使用本地回环；命令对 localhost 排除环境代理，并在具备 socket 监听权限的环境运行。
- `cargo check -p xbxrc -p xbox-ios-bridge --offline` 通过；保留 xbxengine 两条既有未使用代码告警。
- `cargo fmt -p xbox-streaming`、`pnpm lint:fix`、`vue-tsc --noEmit`、`pnpm build`、`git diff --check` 通过。
- [结构化验证与源码摘要](../../dev-docs/validation/2026-10-30-common-startup-fixes.metrics.json)。前序审查脚本及其 metrics 保留修复前证据，当前回归入口位于正式测试文件。

回归入口：

```sh
./node_modules/.bin/vitest run src/streaming src/player/infra/webrtc
NO_PROXY=127.0.0.1,localhost no_proxy=127.0.0.1,localhost cargo test -p xbox-streaming --lib --offline
```

本轮验证覆盖控制流、信令归属、超时和资源收尾。真实网络健康、恢复链、media supply、steady supply、submit/present 长尾及帧供给 delta 需在重启后的 home/TURN 实机 trace 中检查；当前测试结果支持代码修复结论，首帧和稳定播放效果仍待该验证。
