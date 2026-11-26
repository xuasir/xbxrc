# 历史 texImage2D 与当前 texSubImage2D 实机耗时对照

历史实机归档已提供上传基线：1440p60 / USM 3 的 `texImage2D(VideoFrame)` 平均约 0.30–0.34 ms，当前 `texSubImage2D(VideoFrame)` 平均 8.34 ms，稳定段仍为 8.29 ms。当前开销较旧基线约 **25–28 倍**，应优先按性能回退处理新增子上传路径。

| 样本 | 上传入口 | CPU 上传样本数 | 平均上传 ms/帧 | 保留窗上传 P95 ms |
| --- | --- | ---: | ---: | ---: |
| 11月10日 11:20:49—11:21:20，稳定 WebGL2 / USM 3 | texImage2D | 1799 | 0.3257 | 1 |
| 11月10日 12:31:10—12:31:43，USM 3 / 同尺寸直通 | texImage2D | 1954 | 0.2994 | 1 |
| 11月26日，累计计数至 seq1748 | texSubImage2D | 3790 | 8.3398 | 末窗 9 |
| 11月26日，恢复后的稳定增量 seq1033—1748 | texSubImage2D | 1646 | 8.2940 | 末窗 9 |

11月10日 12:30 日志的最后 256 个上传样本：均值 0.3281 ms，P95/P99 均为 1 ms；当前末窗均值 8.2070 ms，P95/P99 均为 9 ms。独立前台 WKWebView / 本地 H264 / 2560×1440 夹具也记录 `videoUploadMethod=texImage2D`，平均上传 0.3783 ms，P95/P99 均为 1 ms；该夹具作为补充平台证据单列。

比较采用实际媒体输入和实际输出，均为 **2560×1440**，目标 60 FPS、USM quality、锐度 3、实际 SR 重建关闭，Worker VideoFrame 供给。首份旧日志的稳定 USM 段与当前均为 `actualPipeline=webgl2`。第二份旧日志记录 SR 开关开启、`actualPipeline=webgl2_sr`，实际同尺寸直通、`actualSuperResolutionActive=false`，其实际源尺寸由 132 个 inbound 样本确认。

两边同步计时合同一致：`performance.now()` 包围纹理绑定及 VideoFrame 导入，随后记录 `uploadFinishedAtMs - uploadStartedAtMs`；平均值取累计上传耗时除以样本数，或使用稳定段内两项计数的增量。该计时使用主线程本地时钟，Worker 时钟校准属于独立分段。旧值受毫秒计时精度影响，当前与历史的差距仍达到多个计时档位。

原始来源与复核依据：

- [11:17 实机归档指标](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-10-decoded-supply-latest-trace-analysis.metrics.json)：`sessions[0].segments[3]`，seq2067—2715；上传 586 ms / 1799 样本。原 trace SHA256 为 `66be2ef3c70c17cc61f7464ccf81b0c0803e68bdd69ecf398cf7705b53f13af3`。
- [12:30 实机归档指标](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-10-worker-freshness-latest-trace-analysis.metrics.json)：`sessions[0].segments[0]`，上传 585 ms / 1954 样本；`worker.final.rendererTiming.upload` 为最后保留窗。原 trace SHA256 为 `9f21810d88716cbdc19de0fc8a3fd722c73fcfa125f9b98bb3dfae9c084062ae`。
- [旧实机报告的上传入口记录](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-10-worker-freshness-latest-trace-analysis.md:112)及[采样源码核验](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-10-decoded-supply-latest-trace-analysis.source-verification.json)。旧日志为 dirty 构建，源码身份使用归档核验；提交 `76a1ea69` 的源码摘录用于解释计时合同，保存于 `source-contract.json`。
- [WKWebView 补充结果](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-10-browser-frame-freshness-clock-and-diagnostics/webkit-result.json)：`runs[0].resources`。
- [当前冻结输入与指标](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-pc-stream-log-analysis/README.md)：当前稳定增量上传 13,652 ms / 1646 样本，直接从已归档 JSONL 解压到内存复算。历史原件和旧临时快照已移出现有路径，旧指标保持归档层级；当前输入保留可重算原件。

网络、恢复与供给分别解读：旧稳定 USM 段接收/解码 59.94 FPS、提交约 59.95 FPS、RTT P95 16 ms，冻结增量 0；当前最后 15 秒接收/解码/视频呈现约 60 FPS、canvas 58.86 FPS，另有 105 ms RTT / 11.04% 单桶丢包尖峰和 0.457 秒冻结。旧稳定配对窗口解码/提交均为 1800 帧；当前全程解码→canvas 差 123 帧，包含 latest-only 覆盖与前后台暂停窗口，计数范围分别保留。CPU 上传 P95 的 1→9 ms 回退独立于这些计数范围；物理 scanout、GPU 执行分位数和 Native ReferenceChain 能力仍为证据不足或 UNSUPPORTED。

旧样本与当前相隔 16 天，构建、协商设置与系统运行条件存在差异。历史证据支持同步导入开销回退的判断，新增子上传路径是首要排查项；API 单变量因果及 WebKit 内部转换、拷贝、驱动等待占比保持待证。

本轮仅更新分析证据和任务记录。最小验收复算三个均值、稳定段样本数、分位数及源码计时合同，各检查一次；临时文件创建数为 0，已有输入归档保留。详见 `checks.json`。

建议恢复 `texImage2D(VideoFrame)` 作为新帧导入入口，保留 `videoGeneration + localSequence` 同帧复用，以及尺寸/context 缓存失效合同。
