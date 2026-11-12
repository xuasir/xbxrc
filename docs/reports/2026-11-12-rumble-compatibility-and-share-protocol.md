# 震动报文兼容修复与 Xbox Share 通讯核对

日期：2026-11-12。范围：xbxrc `main` 当前工作区、better-xcloud `76db611e610c45ea996f6d35a9df5f980e8f0f85`，以及当天从微软 Xbox 网页下载的公开脚本。

## 兼容修复

`RumbleService` 按完整报文长度选择百分比格式的消息头：旧版 12 字节使用 Uint8，V8 13 字节使用 Uint16。较长的旧 10-bit 格式进入独立解析路径。修复覆盖旧版震动、旧版全零停止，以及旧 10-bit 记录首字节为零时被误判成 V8 的情况。

新增用例先复现三个失败，随后相关四文件 53 项测试通过；`pnpm lint:fix` 与 `pnpm typecheck` 通过。回归覆盖两种百分比格式、全零停止、玩家槽位过滤、旧 10-bit 格式、完整长度、消息类型和震动类型。

- [解析器](../../src/player/app/input/RumbleService.ts)
- [回归用例](../../src/player/app/input/RumbleService.test.ts)

## 官方时序与当前实现

官方 FourMotorRumble 报文尾部依次包含 Uint16 `durationMs`、Uint16 `delayMs` 和 Uint8 `repeat`。官方 input 接收器将三者传入 `onVibration`；当前 xbxrc 解析器读取持续时间，派生 `startDelayMs=0`、`repeat=0`。

| 项目 | Xbox 官方实现 | 当前 xbxrc |
| --- | --- | --- |
| 持续时间 | 原始 `durationMs` 交给 actuator；实际硬件边界由浏览器/API 决定 | Browser backend 限制单次为 1000ms |
| 初始延迟 | `delayMs` 非零时，用 setTimeout 等待后播放 | 收包后立即播放 |
| 重复 | 首次播放加 `repeat` 次；相邻开始时间相隔 `durationMs + delayMs` | 单次播放 |
| 新报文 | 清理当前设备的延迟、重复定时器后接收新参数 | 立即调用 actuator；新效果沿 Browser API 抢占前一效果 |
| 弱短脉冲 | 交给 actuator | 持续时间 <8ms 且所有通道 ≤2% 时过滤 |
| 全零停止 | 清理定时器并 reset/清零电机 | 串流归属下 stopRumble/reset |

用实际官方方法和当前源码、模拟计时器及 actuator 回放三组报文，结果见 [results.json](../../dev-docs/validation/2026-11-12-rumble-share-protocol/results.json)：

- `duration=200ms / delay=100ms / repeat=2`：官方在 100、400、700ms 开始三次播放；xbxrc 在 0ms 开始一次，持续 200ms。
- `duration=5000ms / delay=0 / repeat=0`：官方提交 5000ms，xbxrc 提交 1000ms，提交持续时间缩短 80%。
- `duration=4ms / magnitude=1%`：官方提交该脉冲，xbxrc 过滤。

`delay=0 / repeat=0 / duration≤1000ms` 且通过弱短过滤的报文，两侧开始时间和提交持续时间相同。包含延迟、重复或长震动时，当前策略会改变效果节奏。实际游戏报文中这些字段的使用频率需要串流样本；物理电机效果按仓库约定由用户实机验收。

官方播放还包含两项输出细节：支持扳机时先调用 dual-rumble，再调用 trigger-rumble；双路设备将扳机强度的一部分补到右握把。当前 xbxrc 使用单次能力匹配调用，纯扳机效果在双路设备上返回 unsupported。这两项属于输出策略，独立于本次报文格式修复。

## Share 的通讯预留

**Xbox 官方新不可靠输入模型已预留 Share 发送字段。** 官方 `GamepadButtonId` 枚举将 Share 定义为 11；`UnreliableGamepadFrame.transitionCount` 分配 16 个 Uint8 计数器，`generateUnreliableGamepadFrame` 在 Nexus 与 A 之间明确序列化 Share 计数器。

| 编号/结构 | 已核实含义 |
| --- | --- |
| Browser `buttons[17]` | Xbox Series Share 的浏览器输入索引，better-xcloud 在客户端层消费 |
| 官方 `GamepadButtonId.Share=11` | 网络模型中第 12 个按钮状态变化计数器 |
| 37 字节不可靠手柄 payload | 1 字节玩家索引、16 字节按钮计数器、4 字节扳机、8 字节摇杆、8 字节 physicality |
| payload 偏移 12 | Share 的 Uint8 计数器位置；实际编码器注入值 1 后生成该字节为 1 |
| `unreliableinput` | 新输入模型发送通道，ordered=false、maxRetransmits=0；会话依据 SDP 返回版本装配 |
| 当前可靠 `input` GamepadReport | 15 个命名数字按钮通过 Uint16 掩码发送，扳机独立发送；官方同格式枚举覆盖 0xfffe，最低位 0x0001 的功能仍需额外证据 |

协议存在预留的证据来自实际官方枚举、frame 模型和序列化器。官方 `updateCurrentGamepadInputFrame` 当前更新常规按钮计数器；Share 的输入到计数器更新属于待接通链路。编码器回放验证了 Share 可写入报文，服务端接收、远端主机/云游戏截图响应属于实机验证范围。

当前 xbxrc `InputPacketEncoder` 使用可靠 GamepadReport，与上述官方可靠编码一致。接通远端 Share 应沿官方不可靠输入模型的版本协商、计数器与确认机制形成独立实施任务；现有最低位的 Share 语义需要服务端证据后确定。

## 源码与复现

- [官方输入枚举和编码摘录](../../dev-docs/validation/2026-11-12-rumble-share-protocol/official-input.excerpt.js)
- [官方震动方法摘录](../../dev-docs/validation/2026-11-12-rumble-share-protocol/official-vibration.excerpt.js)
- [源码回放脚本](../../dev-docs/validation/2026-11-12-rumble-share-protocol/probe.mjs)
- [URL、SHA-256 和回放结果](../../dev-docs/validation/2026-11-12-rumble-share-protocol/results.json)

官方源文件：[输入模型](https://assets.play.xbox.com/playxbox/static/js/8128.1aca8038.chunk.js)、[WebRTC 输入及震动](https://assets.play.xbox.com/playxbox/static/js/web-rtc-stream.d6f6b903.chunk.js)。摘录和结果保存各自 SHA-256，便于按同一版本复核。

复现命令：`node dev-docs/validation/2026-11-12-rumble-share-protocol/probe.mjs /private/tmp/xbxrc-20261112-official-assets`。脚本使用实际源码方法，强度按直通比较，计时器与 actuator 使用模拟环境；Share 回放直接注入网络计数器。

后续优先对齐 `delayMs / repeat / durationMs` 的游戏效果语义，再按会话支持情况验证官方 Share 预留字段。
