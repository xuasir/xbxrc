# iOS / PC HTTP 差异审计证据

- [最终报告](/Users/guo.xu/Documents/code/games/xbxrc/docs/reports/2026-11-24-ios-pc-http-parity-audit.md)：核心结论、验证边界和未实施的后续优先级。
- [完整发起边界与差异矩阵](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-pc-http-parity-audit/http-diff-matrix.md)：15 个共享 provider/55 个公开异步方法、12 处 Swift 托管调用；发起、预算、重试、401、缓存、取消、错误和下载对照。
- [锁定版本上游证据](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-pc-http-parity-audit/upstream-evidence.md)：reqwest 0.13.2 / hyper-util 0.1.20 的包 checksum、系统代理平台条件与 TLS 摘录。
- [验证摘要](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-pc-http-parity-audit/validation.json)：原快照18条源码断言、146候选来源与续接范围；不把原快照结果当作当前工作树全量PASS。
- [来源指纹](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-pc-http-parity-audit/source-hashes.json)：146项快照文件、原8处并发变化及补充关键来源的哈希。
- [续接验收原始记录](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-pc-http-parity-audit/closure-verification.json)：覆盖/链接/离线依赖等13项PASS，工作树快照一致性3项FAIL，均保留。
- [快照后变更](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-pc-http-parity-audit/post-snapshot-drift.json)：UTC07:41:59观测到9个来源变化，非本任务写入，未纳入快照验收。
- [清理摘要](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-pc-http-parity-audit/cleanup.json)：两个任务临时目录、25文件清理完成；约0.95MiB已分配文件块，原件与共享缓存保留。

审计结论固定于UTC2026-11-24 07:09:23的main工作树快照，后续源码不锁定。静态PASS不等于手机网络或登录功能验收；未改业务代码，未运行构建或认证网络试验。设备与PC运行包来源、真实代理/TUN/DNS/连接路径仍需配对验证。
