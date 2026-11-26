# PC 执行域优化工程验收

对应 [RFC](../../../docs/rfcs/2026-11-26-pc-streaming-execution-domains.md) 和 [Report](../../../docs/reports/2026-11-26-pc-streaming-execution-domains.md)。M0–M4 工程交付，用户统一实机回归待验。

- 最近基线：[facts.json](../2026-11-26-pc-latest-log-1795692428673/facts.json)，1440p60 / USM3、SR关闭；texImage2D平均0.439ms，上传P95/P99 1ms、read→submit P95 2ms/P99 3ms。
- [test-summary.json](test-summary.json)：266项唯一前端定向检查，覆盖原渲染、直接帧链、Worker门面、输入核心/采样/背压、截图批次、生命周期、时钟和局部恢复。
- [checks.json](checks.json)：lint、typecheck、Vite三个Worker、2项生产日志预算、4项Rust合同和fmt通过，包含最终清理结果。
- [initial-failures.json](initial-failures.json)：首次失败及修复说明；[source-changes.json](source-changes.json)记录本轮源文件基线/最终指纹与任务外lint/fmt变动检查。
- [Rust验证](rust-trace-contract-final.log)、[生产预算验证](production-budget-verified.log)：共享描述表与投影同源；原行宽、队列和分钟预算通过。

运行时会验证真实Offscreen context、当前shader、VideoFrame与像素截图，输入Worker另验证时钟与实际DataChannel转移。各消息带作用域，Reader/Render直接交接一次原生帧与一次CPU处理ACK；快照单个在途、100ms数字事实、1Hz聚合，应用帧单link上界3、现行双renderer交接短暂上界6。

工程测试使用可控替身验证调度与所有权；实机宿主能力、H264/NV12驱动路径、画面/听感和远端短按识别在用户一次回归中确认。GPU fence记录完成代理，物理scanout按能力缺证保留。

本轮临时目录：`/private/tmp/xbxrc-pc-execution-domains-20261126-qto4kquy`。结束后按清单清理，最终状态见checks.json；保留最近基线原件、归档证据及共享构建缓存。
