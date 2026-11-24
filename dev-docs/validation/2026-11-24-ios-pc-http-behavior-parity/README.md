# iOS PC 接口与错误行为对齐验收

本目录保存 2026-11-24 P0–P2 续接的最小验证证据，相关 [RFC](../../../docs/rfcs/2026-11-24-ios-pc-http-behavior-parity.md) 与 [实施报告](../../../docs/reports/2026-11-24-ios-pc-http-behavior-parity.md)。

- `validation.json`：最终结果、验收边界和命令。
- `swift-*-summary.json`：xcresult 原始测试摘要；39 / 7 / 3 全部通过，共 42 项唯一场景。
- `rust-tests.log`：三包 200 项测试与 doc-test 结果。
- `diagnostics.json`：本轮失败及最终修复记录，保留沙箱与夹具错误事实。
- `device-build-summary.txt`、`swift-test-summary.txt`、`binding-reproduction.log`：最小最终输出。
- `task-changes.diff`、`source-hashes.json`、`baseline-metadata.json`：续接差异及源码指纹；独立差异仅覆盖本轮直接执行的文件，前轮已有 P0/P1 改动由现有工作区和最终回归覆盖。
- `concurrent-changes.json`：续接期间其他任务来源变动，仅记录并保留。
- `cleanup.json`：本轮临时目录与构建产物清理；共享缓存与其他任务数据保留。

自动验收采用本地无凭据 fixture。真实手机的主机连接、Cloud 401、相册服务响应与长测仍待用户手动验收。原生下载使用 300 秒总期限，连接专用预算保持平台差异。
