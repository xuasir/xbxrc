# 视频像素探测分项成本验证

2026-11-02，沿已授权S4取证路径完成现有dev像素探测的分段计时。冷读初始化、drawImage与getImageData分别累计次数、总耗时和最大耗时；首末读取记录时钟、来源、候选序号及结果。8ms总读取停止阈值、2组/秒配对预算和8–50ms资格继续执行。8项新增回归、117项定向、445项全量、lint、类型检查、生产构建以及24/36组生产回放通过。

## 改动与语义

[像素观测模块](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/video-frame-pixel-observation.ts:159)沿现有共享帧源dev诊断入口计时，字段经原 `frameIdentity.videoPixels` 快照发布。新增 `readTiming`：

| 字段 | 表达的事实 |
| --- | --- |
| `initialization` | canvas创建、尺寸设置与Canvas2D context获取的次数/总耗时/最大耗时 |
| `drawImage` | 从video绘制16×9缩略图的同步调用成本 |
| `readback` | getImageData及取得data的同步调用成本 |
| `coldReads` / `warmReads` | 本次需要创建canvas与复用已有canvas时的完整读取成本，含失败和超预算尝试 |
| `firstRead` / `lastRead` | 读取开始/结束单调时间、总耗时、冷读标记、三个实际执行分项、trackingSource、localSequence和outcome |

每个聚合结构为 `{ count, totalMs, maxMs }`。首末读取的分项名为 `initializationMs`、`drawImageMs`、`readbackMs`；尚未执行的分项保持undefined，JSON省略。`outcome`为success、contextUnavailable、readFailed或readbackBudgetExceeded，沿用原停止原因优先级。失败调用也在finally记录已执行阶段耗时；快照深拷贝聚合和首末记录。

原 `readDurationTotalMs/max`继续覆盖整个读取函数，包含初始化、图像绘制、回读及计时记账的区间成本。分项相加与完整总值分别保留各自测量边界和时钟精度。单次完整读取超过8ms时，当前video的后续像素读取停止，离屏canvas与配对释放。三个分项各3ms的回归得到9ms总读取，正确停止；某一分项24ms的三个回归分别保存该分项，并继续停止后续探测。

cold表示本次需要创建离屏canvas；隐藏/恢复及消费者中断会释放canvas，后续创建计入新的cold读取。firstRead保留该video观测器首次尝试，lastRead跟随最新尝试；字段保存数值事实，原始像素继续留在内部配对基线。额外分项时钟与读取都由已有dev像素资格触发；从off/production开始的观察路径仍有0次额外canvas/像素读取。

这批交付提供成本归因。drawImage/getImageData的同步时长可包含浏览器图像转换、拷贝和GPU同步等待；实际GPU执行时间及物理呈现继续需要直接设备证据。当前帧源交付、native抵扣、latest候选、用户限帧及renderer沿原合同执行。

## 验证结果

[帧源回归](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/video-frame-source.test.ts:131)新增8项：完整成本分解与有效配对、快照隔离，三个分项慢调用、合计超预算及三个分项异常。实施前8失败/39通过，实施后该文件47通过。定向帧源、renderer调度和PlaybackService共117通过；全量48文件445通过。

有效配对回归在native入口执行初始化1ms、drawImage2ms、readback3ms，总冷读6ms；16ms时的rAF复用context，读取5ms，配对间隔10ms。得到1个有效配对、1个原生新候选，cold/warm总耗时6/5ms。snapshot修改和重复读取快照均保持内部计数与采样时钟事实稳定。

重跑既有24组像素/准备回放和36组帧计数回放，均直接加载当前生产模块。24组覆盖两种renderer、off/dev、运动/静态、0.25ms读取、poll先到、隐藏和用户30fps上限；off/dev候选、成功处理和唯一图像一致，资源关闭归零。

| SR，dev，10秒输入 | 成功处理fps | 像素读取 | 有效配对 | 变化配对 |
| --- | ---: | ---: | ---: | ---: |
| 250ms合批、native50Hz、运动 | 50 | 20 | 2 | 2 |
| 相同输入、静态 | 50 | 20 | 2 | 0 |
| 相同运动输入、drawImage成本0.25ms | 50 | 20 | 2 | 2 |
| 250ms合批、poll先到 | 52 | 34 | 16 | 16 |
| 合批输入、隐藏/恢复 | 50 | 21 | 4 | 4 |
| 逐帧60Hz、用户上限30fps | 30 | 18 | 0 | 0 |

0.25ms成本组的20次读取在drawImage分项累计5ms，coldReads为1次/0.25ms，warmReads为19次/4.75ms。36组计数回放保留逐帧计数补满60fps、合批时序约束及off/dev一致性。上述成本和图像来自受控DOM/WebGL及时钟，实际设备表现由新日志验收。

`pnpm lint:fix`、`pnpm exec vue-tsc --noEmit`、`pnpm build`和`git diff --check`通过。最终737输入构建指纹为 `bd14d98f856a7432f0767a1d01b3ae5efe70544e951f5af466a2376a71258120`。[metrics](2026-11-02-video-pixel-read-cost-validation.metrics.json)保存回放、源码指纹、验证数量和命令输出SHA256。

## 新日志验收

[17:43旧日志与SR续查](2026-11-02-sr-60fps-bottleneck-analysis.md)保留历史基线：接收/解码约59.94/59.95fps，正丢失率0.039889%、RTT P9517ms、freeze增量0；SR完整3秒提交51.50/54.25fps，实时rAF58.80/60.09Hz、RVFC52.62/54.78Hz，rAF补帧0。旧日志的首次24ms读取只有总值，这批分项字段由新构建产生。

用户手动使用新构建、同主机1440p/60fps运动场景、持续前台和dev诊断，分析时按以下顺序核对：

1. `readTiming.firstRead`的实际时间、三个分项及outcome；后续warmReads、停止原因和有效/变化配对。总读取超过8ms时，字段继续保留超限阶段与资源释放事实；有效变化配对支持判断RVFC缺席时drawable图像推进。
2. 同代际3s/15s接收、解码、video推进、候选与提交delta；将分项读取时间对齐rAF/RVFC间隔、诊断/窗口状态和提交年龄，评估诊断成本及30→53fps边界。
3. 浏览器恢复计数、media/steady supply及提交长尾联合判断；ReferenceChain、cleanAnchorCommitted、DisplayStable、NACK逐请求有效率、GPU执行和submit→present继续独立标明可观测边界。

S4维持In Progress。下一步从新构建firstRead分项确认24ms来源，再依据低成本有效像素配对决定共享帧源补供给策略；实际采集继续由用户手动执行。

复算生产模块：

```sh
node dev-docs/validation/2026-10-30-video-pixel-pair-and-preparation-audit.cjs /tmp/pixel-read-cost-replay.json
node dev-docs/validation/2026-10-30-browser-frame-counter-pairing-audit.cjs /tmp/pixel-counter-replay.json
```
