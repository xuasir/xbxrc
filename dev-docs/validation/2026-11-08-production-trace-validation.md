# 生产 Trace 自动验证记录

- 日期：2026-11-08
- 状态：P1/P2/P3 代码与自动验证完成；Windows 掌机验收待用户执行。
- 执行合同：[RFC](../../docs/rfcs/2026-11-08-production-trace-evidence-contract.md)。
- 实测回放指标：[runtime metrics](2026-11-08-production-trace-runtime.metrics.json)。
- 自动分析：[原日志报告](2026-11-08-production-trace-original-report.json)、[生产模块回放报告](2026-11-08-production-trace-replay-report.json)。

## 已交付

P1 复用原有 250ms Browser 采集时钟，完整区间进入 1Hz bucket；网络、恢复、接收/解码/视频推进/回调/提交五段、CPU/GPU 分位数、采样状态、代次和配置均有固定合同。每秒使用紧凑累计与完整 delta/coverage，完整累计和 writer 状态进入 30s 检查点及结束摘要。TS/Rust 使用共享字段合同与 fixture，RPC 区分真实入队、过滤、队列满、超大行和 writer 缺失。

P2 在实际几何导航、Focusable、菜单 handler 与页面异步操作入口记录来源、Scope、焦点、结果、耗时和 operationId；晚到结果保留开始时的 clientGeneration。边界导航使用明确结果，交互故障触发异常窗口。连续导航按秒聚合，控制面首次/变化/15s 心跳/末次聚合，SDP/timeline 使用固定摘要。既有父 Scope 修复和 USM 9→5 采样优化保留，详见[掌机修复报告](../../docs/reports/2026-11-08-handheld-stream-menu-and-frame-trace.md)。

P3 提供前 5s/后 10s 的 4Hz 数值窗口、60 样本 ring、异常 episode 和冷却预算；停止时依次记录尾摘要、生产者 drain、执行清理与有限期限 writer flush。导出携带同 run 有序片段、截止 sequence、缺失/截断/保留状态。诊断菜单和失败页提供“导出运行日志”，沿用 app 文件保存 RPC，取消保存记录 cancelled。

生产分析器与既有 Browser behavior report 共用新旧数据投影；输出六段结论、分段有效覆盖、输入配对、异常和分层日志损失，并使用 PASS / FAIL / INSUFFICIENT_EVIDENCE / UNSUPPORTED。Browser 物理 scanout 与 Native ReferenceChain 保留独立能力声明。

## 自动检查

| 检查 | 结果 |
| --- | --- |
| `pnpm test` | 77 文件 / 709 项通过 |
| `pnpm test:trace-production` | 2 项独立成本与预算回放通过 |
| `cargo test -p xbxrc -p xbox-streaming --lib` | xbxrc 120 项 / xbox-streaming 141 项通过 |
| Python runtime trace unittest discovery | 36 项通过 |
| `pnpm lint:fix` / `pnpm typecheck` / `pnpm build` | 通过；构建 361 modules |
| `cargo fmt` / `cargo check -p xbxrc` | 通过 |
| diff / 本轮新增与更新的本地链接检查 | 通过 |

覆盖场景包括短时供给缺口、真实零值、采样 pending/迟到、代次/配置重置、IPC timeout/失败、writer 压力/IO失败、30min 稳态与持续异常、连续导航、旧父 Scope 失败、触屏/鼠标/手柄成功结果配对、源 30fps/主动限帧、解码/CPU 上传/GPU 长尾、稀疏回调、日志缺口、轮转/导出/尾记录顺序及隐私投影。

production/dev/off 对照保持原有媒体决策和提交/输入路径；生产观测使用已采集数值，额外 getStats、DOM 和像素诊断采集为 0。

## 真实生产模块回放预算

回放沿真实 `createBrowserPlaybackTrace` / `createBrowserRuntimeTrace` 发布与投影入口构造 schema v3 JSONL，采样时间使用虚拟 250ms 时钟。成本门独立运行，使用真实高精度计时；全量功能回归持续验证行、字节、ring 与队列预算。

| 指标 | 回放结果 | RFC 预算 |
| --- | --- | --- |
| 30min 健康记录 | 1,801 行 | 最多 1Hz，结束保留末段 |
| 正常日志 | 227,739 bytes/min（约 222KiB/min） | ≤320KiB/min |
| 最大健康 JSONL 行 | 3,744 bytes | ≤4KiB |
| 健康构造/编码 P95 | 0.145ms | ≤0.5ms |
| 持续异常细节 | 56,406 bytes/min（约 55.1KiB/min） | ≤128KiB/min |
| 菜单连续导航/确认 | 58,051 bytes/min（约 56.7KiB/min） | ≤64KiB/min |
| 最大菜单健康行 | 844 bytes | ≤1KiB |
| 生产者排队峰值 | 21,250 bytes | ≤256KiB |
| producer drop / 超大行 / IPC failure | 各 0 | 明确计数并解释覆盖 |

成本数据来自当前开发机自动回放；掌机额外成本与持续帧率收益使用新实机 trace 验收。

## 新旧分析结论

原日志 377 行、270,080 字节，健康记录 0。网络、恢复、media supply、steady supply、submit/present 长尾与帧供给 delta 六段均为 INSUFFICIENT_EVIDENCE，结束为 partial。

生产模块合成回放共 2,834 行，六段性能结论与 performance gate 为 PASS。发布覆盖 100%，有效媒体覆盖 99.99%，五段供给均为 60fps，累计各 108,000 帧。该文件覆盖生产者发布路径；清理和 writer 回执由专项集成测试验证，合成文件的 ending 为 partial、acceptanceGate 为 INSUFFICIENT_EVIDENCE。

## Windows 掌机验收

1. 保持 1440p/60fps、SR 关闭，锐度 0 和 3 各持续串流 60–90s；保存源场景与前台状态，包含稳定运动画面。
2. 两种配置分别打开操作菜单和诊断菜单，验证触屏点击、手柄方向/确认/返回与菜单关闭后的焦点承接。
3. 正常结束串流，使用诊断菜单或失败页导出运行日志；保留各次 JSONL。
4. 使用生产健康报告核验五段 FPS/delta、CPU/GPU 长尾、输入操作配对、发布/媒体覆盖和 ending/flush 回执。

```sh
python3 scripts/production_trace_health_report.py /absolute/path/runtime-trace-run.jsonl --require-clean-ending --fail-on-gate --output /absolute/path/health-report.json
```

通过实机验收后更新 RFC 和任务状态，并生成最终实施 Report。
