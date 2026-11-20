# 官方触屏协议验证

2026-11-13 完成源码审计，使用 2026-11-12 冻结的三份公开官方发布脚本。原件 URL、文件大小和 SHA-256 见 [sources.json](sources.json)，正文见[协议拆解](../../../docs/references/official-touchscreen-protocol.md)。

`replay.cjs` 从原件提取并执行 InputReporter、InputChannelBase、传统/状态输入通道和领域模型，执行原 CircularBuffer。宿主提供模拟 WebRTC 通道、日志、属性初始化 helper 和确定性时钟，polyfill require 在 Node 中为空操作。回放始终使用原文；源码摘录由 TypeScript AST 排版并记录原件偏移与内容哈希。

运行方式：

```sh
node dev-docs/validation/2026-11-13-official-touchscreen-protocol/replay.cjs
# 可指定存放三份原件的目录；文件名与哈希必须匹配：
node dev-docs/validation/2026-11-13-official-touchscreen-protocol/replay.cjs /path/to/snapshot
```

29 项回放通过，覆盖 v6/v7/v8+ 头、UInt32 token 回绕、ClientMetadata 版本门槛与 255 上限、20/21 字节记录、双指多帧、up 字段、归一化触点、默认尺寸、整数截断/回绕、两类计数器、未确认 up、累计/重复 ACK、16ms 重发、ServerMetadata、传统发送尺寸门槛、ACK 队列增长、通道工厂和实际创建选项。[完整结果](replay-results.json)。

五份摘录及来源节点偏移见 [source-excerpts/index.json](source-excerpts/index.json)。布局消息、版本能力和 TAK→session bridge 复用[前序源码证据](../2026-11-12-tak-layout-and-engine/README.md)。

验证范围为官方客户端 wire 输出、状态更新与接收解析的隔离行为。真实服务端接收、ACK 通道选择、计数器转换还原、游戏响应、丢包恢复及生命周期释放仍需端到端验收。末尾固定零字节的服务端字段名保持待核对。
