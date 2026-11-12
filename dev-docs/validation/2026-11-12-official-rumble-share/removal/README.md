# Share 试验撤销验证

> 2026-11-12 整理：本页保留验证摘要；带 Git 链接的截图、夹具和中间输出按采样时提交读取。

日期：2026-11-12。用户实机确认远端对 Share 输入无响应，并明确要求删除。F9、Share 状态、专用编码器、输入通道、协商字段、提示和 trace 已撤销，串流恢复原有输入路径。官方震动时序和四路强度配置保留。

| 检查 | 结果 | 输出 |
| --- | --- | --- |
| 前端相关回归 | 69 文件，730 项通过 | [vitest.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/removal/vitest.txt) |
| Rust 协议 | xbox-webapi 70、xbox-streaming 162 项通过 | [rust-protocol.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/removal/rust-protocol.txt) |
| Rust trace 合同 | 4 项通过 | [rust-trace.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/removal/rust-trace.txt) |
| 生产成本门 | 独立 2 项通过 | [trace-production.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/removal/trace-production.txt) |
| ESLint | 通过 | [lint.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/removal/lint.txt) |
| Typecheck | 通过 | [typecheck.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/removal/typecheck.txt) |
| Build | 通过 | [build.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/removal/build.txt) |
| Rust 格式 | 通过 | [rust-format.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/removal/rust-format.txt) |
| Rust workspace | 通过，三条既有数据模块 dead-code 警告 | [rust-workspace.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/removal/rust-workspace.txt) |
| 官方震动回放 | 五组一致 | [rumble-probe.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/removal/rumble-probe.txt) |
| 源码与构建标识清理 | 通过 | [source-scan.json](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/removal/source-scan.json) |
| Diff | 通过 | [diff.txt](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/removal/diff.txt) |

[scope.json](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/removal/scope.json) 记录已审阅并撤销的 32 个 Share 专用文件及暂存状态核验。混合文件仅移除 Share 部分；六个专用源文件/测试/夹具路径已清理。官方字节夹具归档到 [历史证据](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-official-rumble-share/unreliable-share-official.json)，回放脚本将夹具写入验证目录。

[results.json](results.json) 汇总最终检查。首次实现输出继续保留在父目录的 checks/；本目录对应用户要求撤销后的代码。
