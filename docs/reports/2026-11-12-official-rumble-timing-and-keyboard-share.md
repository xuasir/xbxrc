# 官方震动时序与 Share 试验撤销报告

日期：2026-11-12。工作区：`main`。关联 [RFC](../rfcs/2026-11-12-official-rumble-timing-and-keyboard-share.md) 与 [验证记录](../../dev-docs/validation/2026-11-12-official-rumble-share/README.md)。

## 完成范围

游戏震动已使用官方 delay、duration、repeat 开始时钟，四路强度继续使用现有配置。用户本机确认远端对 Share 输入无响应，按明确指令移除了临时 F9、Share 状态、专用通道、协议协商、提示和 trace。当前串流沿原有输入路径执行；撤销后的代码与自动验证完成。

## 震动

- 兼容 12/13 字节百分比报文和原有 10-bit 记录；读取 delay/repeat，保留 4ms、1% 弱短脉冲和 5000ms 原始持续时间。
- 首次播放等待 delay，之后每隔 duration+delay 追加 repeat 次。duration=200、delay=100、repeat=2 时开始点为 100/400/700ms；硬件完成 Promise 独立于开始时钟。
- 新效果、停止、退出、菜单接管、禁用震动、切换输入来源、切换设备及断连清理定时器，取消时结算待播 Promise。
- 每次播放读取当前四路强度曲线，支持统一设置与单路微调；重复配置相同输入来源保持待播效果。

实际官方类与当前 `RumbleService → InputController/RumbleScheduler → BrowserGamepadBackend → 模拟 actuator` 的五组回放一致，覆盖延迟重复、长效果、弱短脉冲、新效果取消延迟和停止取消重复。对比范围为提交时序与参数；项目强度策略和控制器物理表现分别按配置及硬件执行。

## Share 试验记录（已撤销）

试验阶段在同一 PeerConnection 上按 SDP 答复选择输入模型。以下通道、编码和重发描述记录当时的实现，当前产品接入已撤销。

| 项目 | 实现 |
| --- | --- |
| `unreliableinput` | protocol 2.0，无序，maxRetransmits=0；发送 gamepad 和视频反馈 metadata |
| `reliableinput` | protocol 2.0，有序；发送 client metadata |
| Gamepad report / ACK | type 512 / 1024 |
| Gamepad payload | 37 字节；V9 完整包 60 字节，V10 为 61 字节 |
| Share | `GamepadButtonId=11`，独立 Uint8 `transitionCount[11]`；payload 偏移 12、完整包偏移 31 |
| 丢包恢复 | 16ms 重发未确认状态；frameId 保持、包 token 递增；接收已实际发送帧的 ACK |
| 背压 | 累计按钮边沿，保留最新轴态；Share 松开及重发保留释放余量直到确认 |

F9 通过公共 InputController 管理 Share 状态，普通手柄与键盘游戏输入继续共用原有归属机制。菜单、独占录入、文本焦点、失焦、隐藏、设备/来源切换和会话退出释放 Share。新模型下闲置 legacy 输入通道的消息和关闭事件保持隔离；已关闭或被替换的会话吸收迟到答复和状态事件。

试验阶段贯通的 Rust 信令、Tauri RPC、PlayerClient、runtime host、Stream 提示及生产 trace 已随 Share 输入一起撤销。官方协议字节样本归档到 [验证目录](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/unreliable-share-official.json)。

## 首次实现验证（历史）

| 检查 | 结果 |
| --- | --- |
| Player、公共 gamepad、streaming、diagnostics 回归 | 72 文件，774 项通过 |
| `cargo test -p xbox-webapi -p xbox-streaming` | 70 + 164 项通过 |
| `cargo test -p xbxrc --lib trace_contract` | 5 项通过 |
| 独立生产 trace 成本门 | 2 项通过，0.5ms P95 与字节预算保持 |
| `pnpm lint:fix` / `pnpm typecheck` / `pnpm build` | 通过 |
| Rust 格式及 `cargo check --workspace` | 通过 |
| 官方实际方法回放、V9/V10 实际编码夹具 | 五组时序一致；Share 字节、所有常规按钮和轴态回归通过 |

回归修正了相同来源重复配置的错误取消期望，并将一个旧 dev 健康日志断言对齐已有 trace 合同；当前 dev 与 production 都保留健康摘要。Rust HTTP 测试首次运行遇到沙盒本地端口限制，按权限要求重跑后全部通过。生产成本门与前端编译并发时超出阈值，随后按独立 runner 的测量条件执行通过。初次输出与最终通过输出均保存在验证目录。

工作区 Rust 检查保留数据模块三条既有 dead-code 警告，退出码为 0。本轮通过工作区局部补丁实施，保留用户已有暂存内容。

## 撤销后的验证

| 检查 | 结果 |
| --- | --- |
| 原有 Player/gamepad/streaming/diagnostics 回归 | 69 文件，730 项通过 |
| Rust 协议及 trace 合同 | 70 + 162 + 4 项通过 |
| 独立生产 trace 成本门 | 2 项通过 |
| lint:fix / typecheck / build / cargo fmt / workspace check | 通过 |
| 官方震动实际方法回放 | 五组一致，100/400/700ms 开始时钟保持 |
| Share 专用源码及构建标识 | 清理完成 |

32 个 Share 专用文件通过已审阅的局部反向补丁恢复，混合文件仅移除 Share 部分。已保留用户暂存内容、震动时序和四路强度配置；源码中的专用编码器与测试已删除，官方夹具保留在验证目录。最终检查见 [撤销记录](../../dev-docs/validation/2026-11-12-official-rumble-share/removal/README.md)。

重启应用后使用原有手柄输入继续游戏；四路电机物理表现按现有配置验收。
