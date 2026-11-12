# 官方震动时序与 Share 试验记录

> 2026-11-12 整理：本页保留验证摘要；带 Git 链接的截图、夹具和中间输出按采样时提交读取。

日期：2026-11-12。对应 [交付报告](../../../docs/reports/2026-11-12-official-rumble-timing-and-keyboard-share.md)。用户实机确认 Share 远端无响应，产品输入已撤销；当前结果见 [撤销后的验证](removal/README.md)。本目录保留首次实验与官方协议历史证据，震动时序及强度配置继续交付。

## 官方证据

- [results.json](results.json)：官方资源 URL、SHA-256、Share 字段、实际编码夹具来源和五组逐次时序结果。
- [official-vibration.excerpt.js](official-vibration.excerpt.js)：从官方公开资源 AST 提取的实际震动类。
- [official-input.excerpt.js](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/official-input.excerpt.js)：官方枚举、通道、序列化和 ACK 相关方法。
- [Share 实际编码夹具](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/unreliable-share-official.json)：官方编码器生成的 V9/V10 历史字节样本，已归档到本目录。
- [probe.mjs](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/probe.mjs)：回放实际官方方法与当前震动提交路径。

官方资源缓存目录为 `/private/tmp/xbxrc-20261112-official-assets`。资源来源和摘要见 results.json；验证使用当天固定资源。

## 首次实现检查（历史）

| 检查 | 结果 | 输出 |
| --- | --- | --- |
| 相关前端回归 | 72 文件，774 项通过 | [vitest.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/checks/vitest.txt) |
| Rust 协议 | xbox-webapi 70、xbox-streaming 164 项通过 | [rust-protocol.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/checks/rust-protocol.txt) |
| Rust trace 合同 | 5 项通过 | [rust-trace.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/checks/rust-trace.txt) |
| 生产成本门 | 独立运行 2 项通过 | [trace-production.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/checks/trace-production.txt) |
| ESLint | 通过 | [lint.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/checks/lint.txt) |
| Typecheck | 通过 | [typecheck.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/checks/typecheck.txt) |
| Build | 通过 | [build.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/checks/build.txt) |
| Rust 格式 | 通过 | [rust-format.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/checks/rust-format.txt) |
| Rust workspace | 通过，三条既有数据模块 dead-code 警告 | [rust-workspace.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/checks/rust-workspace.txt) |
| Diff | 通过 | [diff.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/checks/diff.txt) |

[checks/results.json](checks/results.json) 保存最终命令、退出码及验收边界。官方五组对比已包含在 results.json；时序开始点示例为 100/400/700ms。

初次检查输出单独保留：

- [vitest-pre-fix.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/checks/vitest-pre-fix.txt)：相同来源配置取消期望及旧 dev 日志断言；对应回归已修正。
- [runtime-trace-isolated.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/checks/runtime-trace-isolated.txt)：旧 dev 日志断言的独立复现。
- [rust-protocol-sandbox.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/checks/rust-protocol-sandbox.txt)：沙盒拒绝本地 HTTP 监听；授权后协议测试通过。
- [trace-production-concurrent.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/checks/trace-production-concurrent.txt)：与编译共享 CPU 时 P95 超阈值；独立成本门通过。

## 复现

在仓库根目录执行：

```sh
node dev-docs/validation/2026-11-12-official-rumble-share/probe.mjs /private/tmp/xbxrc-20261112-official-assets
./node_modules/.bin/vitest run src/player src/shared/gamepad src/streaming src/shared/diagnostics
cargo test -p xbox-webapi -p xbox-streaming
cargo test -p xbxrc --lib trace_contract
pnpm lint:fix
pnpm typecheck
pnpm build
cargo fmt --all -- --check
cargo check --workspace
```

编译和其他测试完成后独立执行 `pnpm test:trace-production`，按 runner 的条件验证实际 CPU 成本。Rust HTTP 测试需要本地临时监听端口。

Share 本机试验结论为远端无响应，F9 与专用输入接入已撤销。当前源码回归与构建结果见 removal/README.md；震动物理触觉继续由用户验收。
