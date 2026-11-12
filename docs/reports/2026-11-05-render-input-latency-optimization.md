# 渲染与手柄输入延迟优化

2026-11-05。用户授权的渲染与手柄输入优化已完成代码和自动验收：足帧阶段减少补帧探测导入，缺帧后快速恢复；手柄保持4ms/rAF采样并复用稳定快照；普通输入按浏览器缓冲年龄和排空速率控制积压。59文件600项测试及56组生产模块回放通过。真实GPU执行、物理呈现和输入到游戏画面响应接续Browser stable / S4设备验收。

关联[RFC](../rfcs/2026-11-05-render-input-latency-optimization.md)，执行基于main工作区，主窗口直接完成。既有Tauri/Vue/TypeScript/Rust边界、共享帧源、latest-only、FramePacer、几何导航、输入归属和协议交付继续沿当前合同运行。

## 交付与行为

| 体验点 | 完成内容 | 验证结果 |
| --- | --- | --- |
| 健康帧源成本 | 原始候选1秒窗口达到目标98%后进入healthyLowFrequency；诊断保留低频读取 | 每种canvas renderer的健康10秒窗口提交600张唯一图像，探测读取少于110次，跳过健康读取超过450次 |
| 缺帧恢复 | 候选间隔超过1.5倍目标间隔加1ms即恢复探测；下一候选建立基线 | 健康转45/50Hz回调的后续10秒提交至少599张唯一图像；共享帧源总补供给至少149/99帧，计数推进与GPU像素确认共同交付 |
| 渲染长尾 | CPU绘制、纹理上传、GL命令提交及提交间隔分别记录分位数；每秒最多一个异步GPU耗时样本 | GPU扩展缺失、disjoint、查询过期、驱动失败及关闭清理回归通过；FPS/暂停变化重置cadence |
| 手柄采样成本 | 稳定设备、能力信息与未变状态复用；变化使用新的不可变状态 | 模拟静止设备250Hz API轮询 / 10Hz心跳发布，P95/P99间隔4ms；旧快照保持原值，804ms停顿可观测 |
| 输入积压 | 512B基础余量加50ms有效排空速率预算，软水位最多4KiB；缓冲最旧年龄超过50ms时暂停普通新增状态 | 普通发送字节预算、部分排空年龄、最新摇杆同步、短按/释放顺序、扳机峰值及过期归零通过 |

补帧探测继续持有一个PBO，CPU稳态预算和结果新鲜度各8ms，GPU等待期限50ms。冷导入强制检查GPU错误，稳态检查每秒有界执行；fence状态及调用异常继续逐次处理。准备、尺寸变化、失败冷却/一次重试和退出资源清理沿既有帧源生命周期执行。

普通输入、metadata共同使用软预算，metadata按当前剩余字节切分。释放及释放前的边沿按顺序使用16KiB硬水位和1KiB释放余量，128状态/250ms过期归零与最新态同步继续执行。缓冲低水位事件改为完整排空阈值0，年龄保护导致的小积压可在排空时立即flush；50ms重试保底继续执行。

## 统计与诊断口径

[LatencyWindow](../../src/shared/latency-window.ts)固定保留最近256个数值，快照提供P50/P95/P99、均值、最近最大值，并用maxObservedMs保存本统计期整体最大值。count为累计数量，retained为当前分位数样本数；调用reset时开启新统计期。热路径写数值，消费者请求快照时计算排序并缓存结果。提交间隔在目标FPS或暂停预算变化时重置，其余耗时按renderer生命周期累计。

渲染诊断展示提交间隔、CPU纹理上传及GPU执行的P95/P99。GPU使用EXT_disjoint_timer_query_webgl2，单query在途，仅在QUERY_RESULT_AVAILABLE后读取结果；5秒未完成、错误及disjoint均清理资源或证据。扩展缺失时保留能力状态，取得有效数值后显示GPU耗时。CPU命令提交、异步GPU执行和物理呈现分别取证。

新增每秒gamepadSamplingObserved记录API轮询Hz、发布Hz、采样间隔、读取/分发耗时；手柄页显示实际API轮询Hz。浏览器轮询频率对应getGamepads调用，手柄硬件报告频率由固件与系统驱动决定。输入源最新发布快照的时间由observedAt提供；状态变化即时发布，稳定状态按100ms心跳发布。

新增每秒gamepadInputDeliveryObserved记录bufferedAmount最旧年龄、浏览器排空速率、queueWait、observationToSend及physicalSampleAge的分位数。physicalSampleAge表示最近物理输入快照距发送的年龄；该字段与浏览器采样间隔配合解释稳定状态的心跳。bufferedAmount排空由有界512项字节账本观测，超过容量时合并尾部并保留较早年龄。远端收包、游戏处理和输入到画面响应通过设备另行取证。

## 自动验证

- pnpm lint:fix：通过。
- pnpm test：59文件600项通过。
- pnpm typecheck：通过。
- pnpm build：通过，313模块转换完成。
- git diff --check：通过。
- [生产模块回放](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-03-sr-cold-preparation-audit.cjs)：56组PASS，复用真实帧源与renderer模块，视频图像/GPU/时钟采用受控替身。

[本轮指标及源码SHA-256](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-05-render-input-latency.metrics.json)保留完整生产回放结果与新增回归断言边界。历史冷准备指标保存原采样身份。

| 持续10秒输入，每种canvas renderer | 成功提交 / 唯一图像 | GPU补交付 |
| --- | ---: | ---: |
| 60fps源，RVFC50Hz，计数250ms合批 | 600 / 600 | 100 |
| 60fps源，RVFC45Hz，计数250ms合批 | 600 / 600 | 150 |
| 171ms准备、25ms首导入，RVFC45/50Hz，GPU等待0/3ms | 600 / 600 | 150 / 100 |
| 30fps源或用户限制30fps | 300 / 300 | 遵循帧源和目标预算 |
| 静态缩略图或12ms迟到取回，RVFC50Hz | 500 / 500 | 0 |

冷成本回放的运行态窗口从预热后开始。持续稀疏回调保持足帧；新增健康切换回归单独检查599帧下限、唯一图像、总补供给及探测降频。自动回归同时覆盖准备取消/清理、尺寸切换、GPU异常、静态画面、30fps限制、输入短按/短释放、扳机峰值、断开/交接归零、拥塞恢复和会话替换。

复算供帧命令，独立输出路径保留历史证据：

```sh
node dev-docs/validation/2026-11-03-sr-cold-preparation-audit.cjs /tmp/xbxrc-render-input-supply-replay.json
```

## 设备验收与后续

保持同一设备、同一网络、1440p源和60fps目标，用持续运动场景分别运行video、普通锐化和SR各60秒；前10秒观察准备，其余时间按完整3秒/15秒窗口结算。记录实际源/输出尺寸、shaderPath与upscale/passthrough模式。GPU timer的平台支持及稀疏1Hz采样范围随state、count、sampledAtMs保存。

设备trace验收同时覆盖六项：网络RTT/jitter/loss；Browser恢复milestone与关键帧结果；received/decode/callback/candidate媒体供给；steady窗口帧率；submit间隔、上传/GPU长尾及设备present证据；decode→candidate→submit的帧供给delta。Native专属ReferenceChain、RecoveryLedger、cleanAnchorCommitted与DisplayStable继续按rust开发线既有trace合同验收。Browser的物理present与scanout由设备或外部测量补充。

每种渲染模式分别进行连续摇杆、快速点按/释放及扳机轻点，比较sampling gap/read/dispatch与queueWait/observationToSend/physicalSampleAge。缓冲持续老化时核对普通新增输入暂停、释放顺序、排空flush和过期归零。端到端响应以高帧率拍摄或设备测量记录按下到游戏画面变化的P50/P95/P99。

继续验收SR开关、30/60fps切换、窗口/源尺寸变化、前后台切换、断开重连、Windows/ROG及30分钟长测。代码和自动验收已收口；真实呈现、响应改善与平台长测接续Browser stable / S4，实机运行由用户执行。
