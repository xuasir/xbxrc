# SR 冷准备与运行态补供帧修复

2026-11-03。已完成用户授权的两项代码修复：GPU资源准备前置到renderer接入阶段；失败后的重新启用受3秒冷却和每video最多一次重试控制。受控171ms初始化/25ms首导入条件下，两种canvas renderer在45/50Hz回调、250ms计数合批的运行态窗口均提交600张唯一图像/10秒。设备持续60fps、实际drawable与物理呈现继续归属S4手动验收。

## 问题与结果

[最新日志](../../dev-docs/validation/2026-11-03-sr-gpu-supply-fresh-trace-analysis.md)中，GPU探测器将171ms初始化和25ms导入累计为196ms，超过50ms冷预算后停用；同video后续renderer重建继续保留停用状态。两段SR完整3秒提交中位51.388/46.972fps，解码60.033/60.142fps，补帧0。该证据把本次执行边界限定在既有共享帧源和GPU补供帧资源生命周期。

| 修复点 | 当前行为 |
| --- | --- |
| GPU冷准备 | renderer注册帧请求前先准备context、shader及FBO/PBO资源；初始化与逐次video导入/取回分别计时 |
| 待首帧与尺寸变化 | 待首帧、暂停或暂时不可绘制时结束当前采样资格并保留已准备资源；源尺寸变化复用context及固定16×9取回资源，重置冷导入与帧资格 |
| 失败后重新接入 | 保留失败诊断，3秒冷却后显式新renderer接入最多允许一次重试；逐帧观察继续按当前供帧资格运行 |
| 准备取消与清理 | 准备资格由VideoFrameSource持有，多个接入共享同一准备；最后资格取消后释放资源，已取消的排队准备在分配前结束 |

[帧源](../../src/player/infra/render/video-frame-source.ts)提供短期准备资格。普通[WebGL renderer](../../src/player/infra/render/Renderers.ts)和[SR renderer](../../src/player/infra/render/SuperResolutionWebGL2Renderer.ts)在接入时等待准备，注册原帧消费者后交还资格；destroy和接入异常均释放资格。GPU能力失败仍沿现有供帧/renderer回退处理。

[像素观测器](../../src/player/infra/render/video-frame-pixel-observation.ts)把资源初始化从read移入独立prepare。准备采用2秒耗时阈值，分项为context、shader、resources；逐次CPU读取采用一次50ms冷导入和8ms稳态预算，GPU等待上限50ms、补帧消费新鲜度8ms。准备中仍有同步WebGL调用，实际设备准备长尾通过新分项持续取证。

PBO、fence、timer、图像身份比较、native/播放计数抵扣、latest-only和FramePacer继续共用原实现。GPU补交付仍需像素变化及当前native数量、计数、尺寸、sequence、epoch一致；用户30fps限制、静态图像和迟到结果沿原合同执行。

## 诊断口径

`videoPixels.preparationTiming`独立记录准备次数、CPU总值、最大值及context/shader/resources分项，last记录开始/结束/outcome。`readDurationTotalMs`与coldReads/warmReads仅结算逐次导入及取回；兼容字段`readTiming.initialization`投射独立准备累计，逐read的initializationMs为0。准备CPU与read CPU分别复算。

新增preparationState、retryCount/maximumRetryCount/retryCooldownMs/retryAfterMs与eligibility。eligibility保留可见性、paused、readyState、源尺寸、FPS和观察状态；原始像素继续保持内部。资源快照新增pixelProbePreparations和framePreparationRequests，并经[PlaybackService](../../src/player/app/media/PlaybackService.ts)透出，退出资源一起归零。

## 自动验证

[生产模块回放](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-03-sr-cold-preparation-audit.cjs)复用既有生产模块加载器，检查真实renderer调用所导入的图像编号与唯一性；视频图像、GPU和时钟使用受控替身。[56组指标及源码指纹](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-03-sr-cold-preparation.metrics.json)保留具体输入、准备和首处理成本。

| 每种renderer，10秒运行态 | native回调 | GPU补交付 | 成功提交/唯一图像 |
| --- | ---: | ---: | ---: |
| 171ms初始化、25ms首导入、RVFC50Hz | 500 | 100 | 600/600 |
| 171ms初始化、25ms首导入、RVFC45Hz | 450 | 150 | 600/600 |
| 同上，GPU等待3ms | 500/450 | 100/150 | 600/600 |
| 30fps源或30fps用户限制 | 随输入 | 遵循预算 | 300/300 |
| 静态缩略图、RVFC50Hz | 500 | 0 | 500/500 |
| 12ms迟到结果、RVFC50Hz | 500 | 0 | 500/500 |

冷成本场景分别记录171ms准备与25ms首导入，运行态窗口从两帧预热之后开始，两个诊断模式的候选/提交/图像/补帧计数一致。GPU不可用的同代码回退场景保持450/500张原生供给。重复提交、身份重复、关闭时资源残留均为0。

- 定向：帧源、renderer调度及PlaybackService，3文件144项通过；覆盖冷成本分离、3秒冷却/一次重试、准备共享/取消、待首帧、尺寸复用、实际运行态唯一图像和30fps等既有合同。
- 全量：`pnpm test`，47文件485项通过。
- `pnpm lint:fix`、`pnpm exec vue-tsc --noEmit`、`pnpm build`及`git diff --check`通过。
- 既有GPU回放共享入口已同步准备时序；历史metrics保存原采样身份，本次证据保存于新metrics。

复算命令：

```sh
node dev-docs/validation/2026-11-03-sr-cold-preparation-audit.cjs
```

## 实机验收

用新构建保持前台和持续运动画面，开启SR及60fps目标至少30秒。先检查preparationState=ready、独立准备分项、warm读取、pairCount/changedPairCount及deliveredFrameCount增量；再按同renderer代际核对完整3s/15s的decode→候选→提交和实际呈现。

重新开关SR、30fps限制和尺寸切换分别验证资源与预算恢复；失败场景核对3秒冷却、每video一次重试及退出清零。网络、ReferenceChain、media/steady supply、submit/present长尾继续按日志证据逐项结算。本轮实现与自动验证完成，S4整体保持In Progress，自动实机运行继续保持暂停。
