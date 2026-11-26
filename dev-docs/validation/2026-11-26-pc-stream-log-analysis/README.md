# PC 纹理上传优化后的最新串流日志分析

纹理子上传已经在实机生效。持续耗时集中在 VideoFrame→WebGL 导入；本轮约 3 秒的卡顿与网络尖峰同步，随后恢复持续供帧。后续[历史实机对照](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-pc-upload-historical-comparison/README.md)发现上传开销由约0.30–0.34ms升为8.34ms，优先处理建议更新为恢复texImage2D并保留同帧复用；下文保留本份日志的独立测量。

本轮接续用户的最新日志分析请求，直接核对源码与运行时事实。运行时代码沿用当前实现；新增分析证据与任务记录。

- 冻结输入：[runtime-trace-1795678668580-1.jsonl.gz](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-pc-stream-log-analysis/runtime-trace-1795678668580-1.jsonl.gz)。验收前原应用日志目录已移除；当前应用与默认临时日志目录均无可用的新 PC trace，已将本次接续时取得的最新完整输入压缩归档。
- 日志时间：2026-11-26 15:37:48.580—15:40:43.544，Asia/Shanghai；1838 行、4,056,899 字节，schema v3 / dev。
- 构建：Browser / macOS / debug，`37e4fb43`，`workspaceDirty=true`；哈希见 `manifest.json`。
- 配置：主机串流，源和输出均为 2560×1440，目标 60 FPS，WebGL2 / USM quality / sharpness=3，SR 关闭，decoded worker 供帧，VideoFrame 格式 NV12。

| 维度 | 直接证据 | 判断 |
| --- | --- | --- |
| 网络健康 | 前台健康桶 RTT 中位数 16 ms、P95 33 ms、峰值 105 ms；jitter 中位数 5 ms、峰值 15 ms；累计丢包约 0.2826% | 常态网络开销较低，短窗存在明显尖峰 |
| 恢复链健康 | 首次软件呈现阶段约 854 ms；最终 7 个关键帧、2 次 PLI、34 次 NACK、0 FIR；2 次冻结合计 0.457 秒，末段持续解码/呈现 | Browser 公开统计支持恢复后的供帧健康；原生 ReferenceChain、cleanAnchorCommitted、DisplayStable 的能力标记为 UNSUPPORTED |
| Media supply | 最后 15 秒接收 59.99 FPS、解码 59.92 FPS；最终收到 4005 帧、解码 4004 帧 | 持续解码供给接近目标；卡顿窗口接收和解码同时下降 |
| Steady supply | 最后 15 秒视频呈现 59.92 FPS、RVFC 回调 51.87 FPS、canvas 提交 58.86 FPS；最后 3 秒各媒体阶段约 59.94 FPS | RVFC 存在合批推进；worker 独立交付渲染帧，回调频率应独立解读 |
| Submit/present 长尾 | 末段上传 P95/P99 9/9 ms，CPU submit P95/P99 0/4 ms，read→submit 9/11 ms，receive→submit 9/12 ms；提交间隔 P95/P99 21/30 ms | 本地 API 调用耗时主要在上传；物理屏幕呈现和 glass-to-glass 延迟状态为证据不足 |
| 帧供给 delta | 最终视频呈现 3991 帧、canvas 提交 3881 帧，解码→canvas 差 123 帧；两段 worker 观测共 79 次 latest-slot 覆盖、1 次过期丢帧；隐藏切换桶差 43 帧 | 覆盖与暂停窗口和累计差额吻合；异步采样及 reader 重建的严格逐帧结算需要各 reader 终端快照 |

上传计数截至 `seq=1748`：完整上传 1 次，子上传 3789 次，合计 3790 次；兼容回退原因为空，提交失败和限帧丢帧计数均为 0。累计上传 31,608 ms、绘制 31,944 ms，平均上传 **8.3398 ms/帧**，占绘制耗时 **98.948%**，约占 60 FPS 帧预算的一半。末段 `seq=1779` 的 256 样本窗口平均上传 8.207 ms，P95/P99 均为 9 ms；全程观测最大上传耗时 67 ms 包含启动阶段。

这份日志提供实施后的实际成本。性能提升百分比的证据状态为 INSUFFICIENT_EVIDENCE，下一轮需要相同条件下的 `texImage2D` / `texSubImage2D` 对照。源为 NV12，WebGL 导入参数为 RGBA；内部颜色转换、拷贝和驱动同步的具体占比需要平台剖析。截图次数与同帧复用计数均为 0，本轮覆盖连续新帧上传场景。

卡顿窗口为 **15:39:47.710—15:39:50.789，3.079 秒**：`seq=560` 接收/解码 42.47 FPS、视频呈现 39.58 FPS、canvas 32.82 FPS，jitter buffer 39.25 ms；`seq=584` RTT 105 ms、提交 age 149 ms；`seq=614` 单桶丢包 354 个、11.04%。该窗口两次冻结合计约 0.46 秒，`seq=644` 恢复接收 59.45 FPS、解码/canvas 60.43 FPS。网络尖峰与供给停顿共同解释这次短时卡顿；持续上传约 8 ms 进一步占用本地帧预算。

可见性还有一个局部优化候选：[bindVisibilityGovernor](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/runtime/browser-runtime.ts:840) 注册 `visibilitychange`，当前状态同步依赖后续事件；渲染策略读取缓存的 `visibilityBudgetActive`。日志在隐藏启动后约 **14.321 秒**持续使用 60 FPS worker，后续隐藏切换才降为 0 FPS 并释放 reader。建议绑定时同步当前可见性，覆盖隐藏启动和恢复首帧，减少后台上传成本。

分析器结果需要结合证据范围解读：

- Browser 建连/首帧基本行为门为 PASS，关闭、资源清理及 trace flush 为 PASS，trace 丢失计数为 0。
- 六维健康报告为 FAIL，供给失败对应上述 3.079 秒窗口；network PASS 使用持续窗口门限，单桶网络尖峰应单独查看。
- [owner / normalize](/Users/guo.xu/Documents/code/games/xbxrc/scripts/production_trace_health_report.py:29) 把缺失 `observationEpoch` 的 dev 消费事件归到 0，production 健康桶归到 1。[dev 消费事件](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/runtime/browser-runtime.ts:1471) 的生产端当前省略该字段；两类采样因此重叠纳入报告，自动统计稳态时间 105.059 秒。按可见性 manifest 划分，真实前台窗口为 20.929 秒和 31.356 秒；完整验收要求至少 60 秒的连续前台证据。
- 可见性在健康桶发布时读取，`seq=993` 的跨隐藏切换桶仍标为 visible；其 `generationReset` 与 43 帧提交差额应作为切换窗口解读。
- 自动首包延迟包含桶采样近似；原始 timeline 的 connected→firstInbound 为 580 ms。软件 `firstPresented` 和 Browser `videoPresent` 均属于浏览器观测，物理 scanout 能力标记为 unavailable。
- GPU timer extension 状态为 unavailable；一次 fence 取证后因 5 ms 插入成本超预算关闭，留下单次 0—10 ms 完成上下界。GPU P95/P99 和物理 submit→present 状态为证据不足。
- 关键帧请求的 suppressed/sent/usable-response 与逐请求解码闭环、NACK 有效率、repairability 持久化及恢复综合评分在本份 Browser trace 中为证据不足；最终计数与持续帧供给分别保留。
- 所有 P95/P99 都标注对应的采样窗口；渲染耗时保留最近 256 个样本，报告中的多窗极值属于各窗分位数的最大值。

最小验证采用已有 summary、production health、Browser behavior 三个分析入口，各执行一次；接着核对归档输入与捕获哈希、JSON/seq 完整性、上传计数和最终供给数值。首次原件哈希检查因目录已移除而中断，归档后的核验改用冻结输入。运行时代码沿用上轮通过的冒烟结果。本轮证据为压缩输入、`facts.json`、`evidence-anchors.json`、两类报告摘要及 `checks.json`。

复现入口：

```sh
trace_replay_dir=$(mktemp -d /private/tmp/xbxrc-pc-log-replay-XXXXXX)
gzip -dc dev-docs/validation/2026-11-26-pc-stream-log-analysis/runtime-trace-1795678668580-1.jsonl.gz > "$trace_replay_dir/trace.jsonl"
python3 -B .agents/skills/analyze-runtime-logs/scripts/summarize_runtime_trace.py "$trace_replay_dir/trace.jsonl" --json --exclude-categories log
python3 -B .agents/skills/analyze-runtime-logs/scripts/trace_production_health_report.py "$trace_replay_dir/trace.jsonl"
python3 -B .agents/skills/analyze-runtime-logs/scripts/trace_browser_webrtc_behavior_report.py "$trace_replay_dir/trace.jsonl"
rm -r -- "$trace_replay_dir"
```

历史对照补充（2026-11-26）：旧1440p60 / USM3稳定段texImage2D平均0.3257ms，另一实机样本平均0.2994ms、P95/P99均为1ms；当前恢复后稳定段仍为8.2940ms。详见[历史对照报告](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-pc-upload-historical-comparison/README.md)。建议将新帧导入恢复为texImage2D，保留同帧缓存复用；可见性同步与分析器代际结算作为独立候选继续处理。

临时产物收尾：引用聊天即时状态为 idle，分析与冒烟命令均已完成。已删除 `/private/tmp/xbxrc-pc-log-analysis-20261126-nyccyqj4` 及本轮 7 个临时文件，移除 4,331,533 字节；清理后可用磁盘空间 94,617,600,000 字节。压缩输入和必要报告保留于本证据目录，详见 `checks.json`。
