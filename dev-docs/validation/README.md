# 专项验证证据

本目录保留浏览器精简与串流启动验收中的报告、metrics 和生产模块回放。当前待验范围以 [浏览器 RFC](../../docs/rfcs/2026-10-28-browser-runtime-simplification-and-stability.md) 与 [公共可靠性 RFC](../../docs/rfcs/2026-10-28-streaming-common-reliability-simplification.md) 为准。

| 当前阅读入口 | 证据与待验项 |
| --- | --- |
| [SR冷准备与运行态补供帧修复](../../docs/reports/2026-11-03-sr-cold-preparation-and-runtime-supply-fix.md) | 前置准备、一次有界重试与56组回放通过；171ms/25ms冷成本之后运行态600张唯一图像/10秒，实机持续60fps待验 |
| [SR GPU补供帧最新设备日志](2026-11-03-sr-gpu-supply-fresh-trace-analysis.md) | 首次CPU196ms超冷预算停用、补帧0；SR同窗51.39/46.97fps，冷启动与恢复资格待修 |
| [GPU异步补供给实施验证](2026-11-02-gpu-drawable-supply-validation.md) | 40组受控回放通过；设备warm成本、有效像素配对及实际补帧待验 |
| [像素读取成本分段](2026-11-02-video-pixel-read-cost-validation.md) | 当前代码验证；cold/warm 读取开销和有效 drawable 配对待新日志 |
| [SR60fps 瓶颈续查](2026-11-02-sr-60fps-bottleneck-analysis.md) | 同代际 RVFC/rAF、候选交付和回放；SR60fps/实际呈现待验 |
| [最新网络与渲染分析](2026-11-02-network-render-fresh-trace-analysis.md) | 同窗网络、恢复、供帧、处理成本与长尾 |
| [公共启动修复报告](../../docs/reports/2026-10-30-common-startup-fixes.md) | 当前正式回归入口和五项问题的修复行为 |
| [home 首帧与退出新日志](2026-10-30-home-startup-fresh-trace.md) | 空 ICE 轮询后成功启动、首帧和资源清零 |
| [浏览器代码验证](2026-10-29-browser-policy-code-validation.md) | 精简初始基线、共享样本和稳定性工具 |

日期化 `.py` 和 `.cjs` 文件绑定对应样本或历史源码，用于复算该份报告；命令、输入身份与结果边界见同名 Markdown。最新分析仍依赖早期复算模块和回放，依赖链随本轮证据保留。历史 metrics 中的源码/脚本 SHA 表示采样时身份。

`browser-runtime-optimization-baseline.json` 是 [复杂度测量工具](../../scripts/measure-browser-runtime.mjs) 的固定基线；`browser-runtime-optimization-current.json` 是当时的测量结果。常规日志分析入口使用 [本地验收手册](../streaming-reliability-local-validation.md)中的 `.agents/skills/analyze-runtime-logs/scripts/` 工具，仓库级回归统一见 [tests/](../../tests/README.md)。
