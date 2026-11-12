# main 手柄输入链路可靠性与即时性审计

## 结论与范围

2026-11-02，审计基线 `main` / `85c39d6b`。采样连续、发送畅通时，当前链路能够完整发送高频按键；短按漏采、积压丢边沿、模拟扳机峰值丢失和延迟释放均已在真实输入模块组成的模拟链路中复现。建议优先收口这些缺口，再以实机输入到游戏响应的时延验收稳定性。

本轮交付为代码审计、相关回归测试和行为实验。生产实现保持当前版本。`rust` 开发线的 SDL3 / Rust 媒体输入端口属于独立验证范围。

后续状态：用户已授权修复，相关代码与自动验证完成，见[输入交付修复报告](2026-11-02-gamepad-input-delivery-fixes.md)。本文保留 `85c39d6b` 的审计基线与原始实验结果。

## 当前链路与已有保护

| 阶段 | 当前实现 | 对游玩的意义 |
| --- | --- | --- |
| 设备采样 | `BrowserGamepadBackend` 读取 `navigator.getGamepads()`，前台跟随 rAF；异常时发布空设备并继续采样 | 60Hz 连续 rAF 对应约 16.7ms 采样间隔，实际节奏受主线程和 WebView 调度影响 |
| 后台补采 | 50ms 定时器检查 `now-lastSampleAt >= 100ms`，所有采样更新同一 `lastSampleAt` | 理想定时器下补采间隔约 100ms；转入补采的首个间隔可达到约 100–150ms，系统限流会进一步拉长 |
| 输入归属 | 公共 `InputController` 决定活动设备及 `ui/stream/blocked`，交接时逐控件等待释放 | 防止打开菜单、切换设备、恢复输入时把已按住的控件带入新的消费方 |
| 游戏变换 | 公共游戏分支执行按键双射、左右死区、扳机 3% 死区及右 Y 反转 | 几何 UI 导航和游戏配置各自消费同一标准采样；协议 Y 转换在编码时完成 |
| 变化提交 | `GamepadDriver` 同步订阅；状态变化立即调用 `queueGamepadState → enqueue → flush`；相同状态保留约 100ms 心跳 | 状态变化采用即时提交；UI 导航的 450ms 首次连发 / 140ms 连发间隔只作用于界面导航 |
| 本地积压 | 8 个待发送状态；相同数字按键开关状态合并模拟值；普通水位 16KiB，特殊释放增加 1KiB 余量 | 成功发送后才推进序号和出队，短暂拒发可保留按下/释放顺序 |
| 重试与传输 | 50ms 定时重试；DataChannel 使用有序、默认可靠传输 | 保持报文顺序；网络重传和已接受报文的排队会增加远端接收时延 |
| 清理 | 菜单/capture 发布中性态；停止串流释放状态并清理队列、定时器、订阅和触觉 | 已有测试覆盖菜单、设备断开、迟到旧通道事件与会话替换 |

## 确定缺口

### 1. P1：积压超过 8 个状态时丢弃历史按键边沿

位置：[InputService.ts](../../src/player/app/input/InputService.ts)，`enqueue` 第 129–139 行。

当发送拒绝或 `bufferedAmount` 超水位时，按下和松开进入队列。第 9 个不同开关状态到达后，整个队列折叠为中性态和最新状态。实验中 9 个 A 键交替状态在 18KiB 水位下积压，恢复后仅发出 `[释放, 按下]`，前面的完整点击消失。按 60Hz 每拍交替的输入，约 0.15 秒可达到此条件；实际达到时间取决于有效边沿数量和拥塞持续时间。

这个策略能够归零旧状态，并明确牺牲历史动作完整性。建议把边沿超预算的降级明确纳入稳定版验收，并结合输入年龄、积压持续时间和实际边沿率确定队列预算。

### 2. P1：扳机的有效拉动峰值会在拥塞合并中消失

位置：[InputService.ts](../../src/player/app/input/InputService.ts)，第 130–132 行；[preferences.ts](../../src/shared/gamepad/preferences.ts)，`applyGameControls`。

合并条件只比较所有 `buttons[key] > 0`。RT 经过死区后从 `0.1 → 0.9 → 0.1`，三个样本具有相同的开关状态。实验在发送受阻时采到深按和回落，恢复后只发出 `0.1`，`0.9` 峰值丢失。以扳机阈值触发射击的游戏可能漏掉这次开火。

建议为扳机保留足以表达有效动作的变化证据，并单列快速拉动、连续射击和轻压保持的回归。

### 3. P1（后台）/ P2（前台）：轮询快照存在短按和短释放漏采

位置：[browser-backend.ts](../../src/shared/gamepad/browser-backend.ts)，第 37–38、106–123 行。

完整位于两次 `getGamepads()` 读取之间的按下/释放会消失。60Hz 模拟中，`t=4..12ms` 的 8ms 按下发送次数为 0；两次按下之间 `t=20..28ms` 的 8ms 释放落在 `17/33ms` 两次采样之间，远端输入报文表现为一次长按。前台主线程停顿会扩大这类漏采窗口。

rAF 暂停后，定时器自身也更新 `lastSampleAt`，导致理想补采在 `100/200/300ms` 发生。实验中 `t=20..70ms` 的 50ms 按下完全漏采。`keep-active` 允许公共层消费后台输入，实际采样频率由浏览器和 WebView 调度决定。

建议收紧后台补采与前台卡顿的采样预算，实测目标平台的 Gamepad API 更新频率及 rAF / timer 长尾。任何快照轮询方案都需要验证按住和松开各自的最短可辨识时长。

### 4. P2：普通松开和设备断开释放受普通水位阻挡

位置：[InputService.ts](../../src/player/app/input/InputService.ts)，第 124–126、164–167 行。

特殊释放优先级由 `neutral && owner != stream` 触发，溢出降级也设置 `releasePending`。物理设备断开后 owner 继续为 `stream`，中性态按普通输入排队。在 `16KiB + 500B` 水位下，实验于 34ms 采到断开，50ms 后允许缓冲排空，100ms 的轮询才发送释放，延迟 66ms。同水位下，菜单切换归属的释放在当前调用中立即提交。

持续拥塞会让远端继续保留已收到的按住状态。特殊释放余量只帮助报文进入本地 DataChannel；已进入有序通道的报文仍按原顺序传输。建议显式识别普通释放和设备失联释放，并验证拥塞时的远端持键持续时间。

### 5. P2：恢复发送依赖 50ms 轮询，输入源时间在发送边界丢失

位置：[InputService.ts](../../src/player/app/input/InputService.ts)，第 77–85 行；[DataChannelHub.ts](../../src/player/infra/webrtc/DataChannelHub.ts)；[GamepadDriver.ts](../../src/player/infra/input/GamepadDriver.ts)，第 31 行。

通道缓冲排空事件没有接入 flush。发送恢复后，下一次输入变化或 50ms 定时器负责重试。实验于 17/33ms 采到按下/释放、34ms 恢复发送，两份报文都在 50ms 提交；对应采样后等待 33/17ms。

公共快照携带 `sequence/observedAt`，Driver 输出的 `GamepadFrame` 只携带设备槽位和状态，编码时间取发送时的 `performance.now()`。积压输入缺少源时间与年龄约束，恢复后可能批量重放旧动作。服务器与游戏对同批按下/释放的处理需要实机验证。建议接入缓冲恢复的即时 flush，并让采样身份和时间贯通诊断及积压年龄判断。

## 即时性判断

- 前台采样连续、通道畅通：状态变化沿同步调用链提交。60Hz 下，物理动作等待下一次轮询的理论区间约 `0–16.7ms`，均匀相位平均约 `8.3ms`；120Hz 对应 `0–8.3ms`、平均约 `4.2ms`。这些值还需叠加手柄、系统、浏览器状态更新及 JS 执行时间。
- 100ms 心跳补发相同状态；按键变化在采到的当次调用立即尝试发送。
- 拒发或水位阻挡：增加拥塞等待，以及理想调度下至多约 50ms 的下一轮重试等待；新输入变化可以提前触发重试。后台补采还增加约 100ms 采样间隔，WebView 冻结会进一步扩大延迟。
- input DataChannel 使用 `ordered:true`，`maxRetransmits/maxPacketLifeTime` 采用默认可靠语义。`channel.send()` 成功表示本地接受报文；远端接收、游戏轮询、画面返回分别需要额外证据。
- 当前采样到远端动作的 P95/P99 尚待实测。现有 `gamepadFrameSent` 记录部分按键、左摇杆及输入序号，缺少源采样时间、队列等待、完整 RT/LT 和远端接收确认；RPC 写入时间还晚于实际发送调用。

## 验证证据

相关现有测试：10 个文件、50 项通过，涵盖 Browser 后端、公共控制器、键盘桥、运行时装配、设置模型、InputService、触觉、编码、会话替换和串流 capture。

执行命令：

```sh
./node_modules/.bin/vitest run src/shared/gamepad src/player/app/input src/player/protocol/input/InputPacketEncoder.test.ts src/player/app/session/SessionService.test.ts src/pages/stream/stream-input-route-controller.test.ts
./node_modules/.bin/vitest run --config .codex-tmp/gamepad-chain-audit.config.ts
```

本地审计夹具位于忽略目录 `.codex-tmp/gamepad-chain-audit.test.ts`，组合真实 `BrowserGamepadBackend → InputController → GamepadDriver → InputService → InputPacketEncoder`。硬件、rAF、时钟、RPC 和 DataChannel 为模拟；9 项行为实验通过。断言复现当前行为，问题场景的通过代表现象已复现。

| 实验 | 观测结果 |
| --- | --- |
| 60Hz 采样，20 次/秒，每次按住/松开各 25ms，持续 10 秒 | 200 次按下全部发送；模拟时钟下采样到同步 send 的等待为 0ms |
| 60Hz 两次采样之间完整 8ms 短按 | 1 次物理动作，0 次发送按下 |
| 连按之间的 8ms 松开落在两次采样之间 | 2 次物理点击，报文仅保留 1 次按下 |
| rAF 暂停，20–70ms 的 50ms 短按 | 实际补采间隔 100ms，0 次发送按下 |
| 18KiB 积压，9 个交替按键状态 | 恢复后仅 `[0,16]` 按键掩码；记录 `edge-budget-exhausted / discarded=9` |
| RT `0.1→0.9→0.1`，深按/回落期间拥塞 | 恢复仅约 0.1，深按峰值消失 |
| 16KiB+500B 下断开设备 | 34ms 采到断开，100ms 发释放，等待 66ms |
| 相同水位下菜单交接 | 中性释放当次提交 |
| 按下/释放分别在 17/33ms，34ms 恢复发送 | 两份报文于 50ms 提交，采样后等待 33/17ms |

结构化实验结果：[gamepad-input-chain-audit-results.json](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-02-gamepad-input-chain-audit-results.json)。0ms 来自假时钟，表达同步调用行为；真实 CPU 耗时由实机采样确认。

`runtime-logs` 当前保留 10 份历史 trace，最新文件 `runtime-trace-1787121327620-1.jsonl` 为 2026-08-19。这些日志早于当前统一输入和发行裁剪代码，仅作为历史资料。此次没有启动实际应用或远端游戏，实机启动、串流和长测遵循用户手动验收边界。

## 后续验收建议

1. 优先处理边沿溢出和 RT/LT 峰值丢失，再收紧后台补采、普通释放和缓冲恢复等待。
2. 保留当前公共归属、游戏映射和标准传输合同；对采样或队列合同的跨层修改先落 RFC。
3. 输入诊断贯通采样身份与时间，报告 sample gap、sample-to-send、queue age/depth、bufferedAmount、按键边沿数及溢出量；使用独立计数器减少逐包 IPC 对主线程的影响。
4. 手动验收覆盖 5/10/20Hz 连按、短按/短松开、RT 轻压保持快速拉动、摇杆+扳机+按键并发、菜单/覆盖层/后台往返、持键断开及弱网恢复；同时观察远端持键和批量旧动作重放。稳定性证据需包含实际输入到游戏响应的时延分位数及长测。
