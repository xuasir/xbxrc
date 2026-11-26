# PC 最新日志分析：上传恢复与视频供给停顿

`texImage2D(VideoFrame)` 已在实机生效，累计平均上传 **0.439 ms/帧**，末段上传 P95/P99 均为 **1 ms**。两个连续前台健康窗口的六维门均为 PASS。主要卡顿表现为视频接收供给间歇停顿：最长冻结 **4.477 秒**，同期 RTT 为 4–5 ms、丢包增量为 0、音频持续推进，之后视频恢复约 60 FPS。

分析沿用当前运行时代码；本轮产物为冻结日志、分析报告和关键行证据。

## 输入与观测范围

- 原件：`/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1795692428673-1.jsonl`。
- 冻结时间：2026-11-26 19:40:47，Asia/Shanghai；冻结前后文件长度一致。
- 日志覆盖：**19:27:08.673—19:31:32.955**，264.282 秒，5483 行，15,303,638 字节；实际串流会话 232.116 秒。
- Schema v3 / dev；essential 66、key 3727、debug 1690；`traceBudgetNotice=0`，生产端和写入端丢失计数均为 0。
- 构建：Browser / macOS / debug，`37e4fb43`，`workspaceDirty=true`，source fingerprint `1463f257ab59eac2a3aabd415b573bf6d82f6414f83d8d12af97bdfdc7f745eb`。
- 主机串流，源 2560×1440 / 60 FPS，WebGL2 / USM quality / sharpness=3，SR 关闭，decoded worker 供帧，VideoFrame 格式 NV12。
- 输出画布开始为 2560×1440；19:29:56.774 随窗口尺寸改为 2994×1684，源尺寸继续保持 2560×1440。
- 输入压缩归档见 [runtime-trace-1795692428673-1.jsonl.gz](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-pc-latest-log-1795692428673/runtime-trace-1795692428673-1.jsonl.gz)，捕获哈希和命令见 [manifest.json](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-pc-latest-log-1795692428673/manifest.json)。

## 六维健康结果

| 维度 | 实际证据 | 判断 |
| --- | --- | --- |
| 网络健康 | 197 个前台有效、完整健康桶：RTT 中位数 4 ms、P95 13 ms、峰值 189 ms；jitter 中位数 5 ms、P95 9 ms、峰值 14 ms；视频 jitter buffer 中位数 5.76 ms、P95 10.18 ms。最终丢包 452 / (539840 + 452)，约 0.08366%。 | 常态延迟低；19:28:37—19:28:46 有短时丢包及 RTT 尖峰，局部网络事件单独保留。 |
| 恢复链健康 | 建连到首帧软件呈现 708 ms；最终解码关键帧 14、PLI 2、NACK 4、FIR 0；冻结 5 次、共 7.618 秒。视频停供后，关键帧和持续帧推进恢复；两次 `decideRecovery.reason=null`，peer/runtime generation 保持 1。 | Browser 公共统计证明后续供给恢复；Native ReferenceChain、cleanAnchorCommitted、DisplayStable 的能力状态为 UNSUPPORTED。 |
| Media supply | 最终收到 12764 帧、解码 12763 帧、浏览器 framesDropped=0；最后 89.080 秒健康窗口接收、解码均为 59.342 FPS。 | 稳态接收与解码保持接近目标；两段局部窗口有源供给不足。 |
| Steady supply | 最后 89.080 秒视频呈现 59.466 FPS、RVFC 53.904 FPS、canvas 提交 59.330 FPS；该段视频推进 5292 帧、callback 4797 次。 | RVFC 有合批推进，worker 继续独立提交接近 60 FPS。 |
| Submit/present 长尾 | 末段最近 256 帧：CPU draw P95/P99 1/1 ms、upload 1/1 ms、CPU submit 0/1 ms、read→submit 2/3 ms、receive→submit 1/1 ms。GPU fence 完成边界取证 194 次，完成上界 P95/P99 6/8 ms，timeout=0。 | 本地上传和交付开销已回落；物理屏幕呈现与 glass-to-glass 为证据不足。 |
| 帧供给 delta | 最终解码 12763、视频呈现 12766、canvas 提交 11340；解码→提交差 1423。两段 worker 共 read 11362、覆盖 21、过期丢弃 1、delivered 11340，delivered 与 canvas 提交一致。末段解码 5281、canvas 提交 5280。 | worker 的局部计数闭环为 `11362 = 11340 + 21 + 1`；解码到 reader 的差额跨后台暂停和 reader 重建窗口。 |

前台健康桶的 RTT/jitter 分位数采用样本最近秩口径；渲染 P95/P99 来自对应最近 256 帧窗口。GPU fence 上界表示完成时间范围；GPU timer query 状态为 unavailable，物理 scanout 状态为 unavailable。

## 关键时间线

| 时间（Asia/Shanghai） | seq | 事件与后果 |
| --- | --- | --- |
| 19:27:39.409 → 19:27:40.116 | 175 → 192–194 | Peer connected，软件解码/呈现首帧阶段完成；首帧阶段计时为 708 ms。 |
| 19:28:37.031—19:28:39.805 | 1586、1603、1631、1658 | 丢包先增加 127，RTT 峰值 189 ms；两次冻结共 0.400 秒，随后帧供给继续推进。 |
| 19:28:45.100—19:29:11.205 | 1805 → 2032 | 切到后台 26.105 秒，渲染预算降为 0，恢复前台后 reader 重建。隐藏健康桶 canvas 提交增量为 0。 |
| 19:28:45.101—19:28:50.963 | 1807、1817、1859 | 另一批丢包增加 325，累计变为 452；后台两次冻结合计 2.741 秒。 |
| 19:29:12.968—19:29:22.795 | 2103 → 2338 | 9.827 秒供给不足窗口，接收/解码/提交逐步由 44/24/25 FPS 降到 0，再恢复。 |
| 19:29:17.757—19:29:21.789 | 2227、2250、2280、2308 | 四个完整桶，视频收包、解码、视频推进、canvas 提交增量均为 0；packet age 从 1009 增至 4032 ms；RTT 4–5 ms，丢包增量 0，音频约 47–49k samples/s。 |
| 19:29:19.231—19:29:21.248 | 2237、2265–2266、2292–2293 | 本地仲裁一次 suppression；两次媒体健康恢复提案收到 `reason=null`，继续保持当前连接。 |
| 19:29:20.789 → 19:29:23.808 | 2281 → 2317、2338、2362 | PLI 累计 1→2；记录 4.477 秒冻结结束，后续关键帧 +1，接收/解码/canvas 返回约 60 FPS。 |
| 19:29:48.756—19:29:51.780 | 3004 → 3053 | 3.024 秒局部供给不足，最低单桶解码/提交 28.77 FPS，RTT 5–7 ms、丢包增量 0；之后恢复。 |
| 19:29:56.774 → 19:31:25.853 | 3172 → 5442 | 输出尺寸调整，后续 89.080 秒健康窗口六维门 PASS，冻结增量 0。 |
| 19:31:25.848—19:31:26.303 | 5437、5458、5472、5473 | 菜单退出、资源清理完成，startup/input/remote/runtime 清理结果完整，producer drain 与 BufWriter flush 成功。 |

## 上传优化的实机结果

`seq=5384` 资源快照记录 `videoUploadMethod=texImage2D`、完整上传 11249、子上传 0、同帧复用 0。累计上传 4938 ms、CPU draw 5245 ms，对应平均上传 **0.43897 ms**、平均 draw **0.46626 ms**。

此前 [15:37—15:40 实机样本](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-pc-stream-log-analysis/README.md) 的子上传累计均值为 8.3398 ms，末段 P95/P99 为 9/9 ms。当前完整上传累计均值为 0.439 ms，末段 P95/P99 为 1/1 ms；这些独立实机数据支持保留完整上传策略。

全程上传最大观测 37 ms，首次进入该最大值的 render 样本为 `seq=3194`，紧随输出画布尺寸变化。末段最近 256 帧最大上传为 1 ms。截图与同帧复用计数均为 0，本份输入覆盖连续新帧导入。

## 卡顿归因与音频

最高置信度的事实为：19:29:17—19:29:22 的视频 RTP 字节/包推进停住，Browser 接收、解码和本地提交一起停住；同期音频和采样继续工作，RTT 与丢包计数保持低值。该现象指向视频输入供给间歇，主机游戏场景、主机编码/发送、视频流专属传输与浏览器接收内部状态的具体分工需要远端证据。

音频为 Opus / 48 kHz / 2 声道，direct 路由，末段 ready，累计 concealment 23226 / 10841760 samples，约 0.2142%，recoveryAttempts=0。前台音频 jitter buffer 中位数 38.46 ms、P95 193.33 ms、峰值 326.15 ms；19:28 的网络尖峰后缓冲逐渐回落，末段为 36.92 ms。音频缓冲长尾是本轮另一项实际观察，耳机/扬声器物理播放时间为证据不足。

## 分析器与证据边界

- Summary、生产健康报告、Browser 行为报告各执行一次。Browser `--require-stability` 返回 2，对应中间供给不足与 submit age 长尾；建连、首帧和最终退出证据完整。
- 真实 production 健康桶的两个连续窗口分别为 **65.045 秒**、**89.080 秒**，六维门均 PASS；中间 44.858 秒窗口的 media/steady/delta/submit gate 为 FAIL。
- [owner](/Users/guo.xu/Documents/code/games/xbxrc/scripts/production_trace_health_report.py:33) 对缺失 `observationEpoch` 使用 0。全部 291 条 `browserObservationConsumed` 省略该字段；真实健康桶 epoch 为 1。全局报告叠加两种采样，得到 424.145 秒稳态时间，超过实际会话 232.116 秒；全局 network/recovery 的 INSUFFICIENT_EVIDENCE 也包含 dev 消费窗口的配置缺口。分段结论按真实健康桶核对。
- `seq=2048` 是跨恢复前台的部分桶，最大 submit age 26404 ms 包含后台暂停；持续前台卡顿使用 `seq=2227–2308` 的完整桶。
- 最终累计计数与逐桶 delta 的覆盖不同：逐桶 delta 因启动/重建边界排除部分 counter 区间；视频呈现计数还包含 RVFC/公共计数合并。逐帧结算采用 worker reader 自身的计数。
- Native recovery 事件族计数为 0，其 chainBuildSuccessRate / NACK effectiveRate / recoveryEffectiveness 在 summary 中的 0 属于空证据默认值；本份 Browser trace 的逐请求 suppression、usable response、decode→cleanAnchor→DisplayStable、NACK timely/late/expired、repairability 持久化与综合评分状态为 INSUFFICIENT_EVIDENCE。
- 首包等 Browser getStats 阶段包含采样延迟；原始 connected→firstInbound 锚点约 649 ms，自动报告引用事件内 elapsed 得到 502 ms。软件首帧阶段为 708 ms，两种时钟/采样口径分别保留。
- 报告 freshness 读取冻结副本 mtime；捕获时间与原始日志起止时间独立记录于 manifest。

详细事实见 [facts.json](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-pc-latest-log-1795692428673/facts.json)，38 个关键行见 [evidence-anchors.json](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-pc-latest-log-1795692428673/evidence-anchors.json)，原始分段门结果见 [production-health.json](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-pc-latest-log-1795692428673/production-health.json)。

## 最小验证与收尾

验收采用冻结输入的 JSON/连续 seq/哈希核对、归档 gzip 哈希复核及 worker read→delivered→canvas 计数闭环检查。运行时代码验收沿用本轮实机数据；本任务新增证据文档。

建议保留 `texImage2D` 与同帧复用；优先对照 19:29:13—19:29:23 的主机游戏场景和视频发包，再校准 dev 消费事件的 epoch 与分段计时。

清理核验：三个分析命令均已结束；已删除本轮临时目录 /private/tmp/xbxrc-pc-latest-20261126-n9g2kxfj，共 9 个文件、15,844,651 字节。原应用日志与本证据目录保留；清理后可用磁盘空间 92,160,000,000 字节。详见 [checks.json](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-pc-latest-log-1795692428673/checks.json)。
