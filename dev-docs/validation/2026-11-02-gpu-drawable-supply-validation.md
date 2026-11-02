# SR GPU 异步补供给实施验证

2026-11-02。已按用户“找到解决方案”授权完成共享帧源补供给代码。方案沿同一 HTMLVideoElement、VideoFrameSource 和现有 WebGL/SR renderer 执行。关联 [RFC](../../docs/rfcs/2026-10-28-browser-runtime-simplification-and-stability.md)。

## 已交付行为

canvas renderer 通过一次性帧请求声明动态目标 FPS。有效 native 观察建立 GPU 缩略基线；rAF 缺少计数新帧且具有显示槽位资格时，再采样当前 video。16×9 缩略图通过 `TexImage2D(video) → shader/FBO → PIXEL_PACK_BUFFER/readPixels → fenceSync` 生成，timer 使用 `clientWaitSync(..., 0, 0)` 等待完成，再执行 `getBufferSubData`。RGB 像素变化且身份仍有效时，立即沿原帧源通知 SR 绘制。

同一 video 最多一个在途读回、一个基线、八组差异证据。GPU 原始像素保持内部；一个 full-source video texture 承接视频导入，FBO/读回保持 16×9/576 字节。默认统计及原生 video 订阅保持原供帧路径，dev 独立像素取证保持每组至少 500ms 的间隔。

补供给同时增加 native 抵扣和晚到播放计数抵扣额度；`localSequence` 只在新候选交付时递增。后续原始 RVFC metadata 继续交给消费者，去重观察携带 `newFrame=false`；GPU 补供给 metadata 使用 `trackingSource=animationFrame`，RTP/native 计数字段保持缺省。

完全抵扣已交付帧的计数批次可在下一次 GPU 读取前重对齐基线资格，消除 45Hz 回调与 250ms 合批相位交叉处每十秒漏十张的边界。`baselineCounterFrames` 保留实际采样时的原计数；本次读回期间的 native 数量、计数、尺寸、sequence 和 epoch 继续严格验证。

## 预算与失效

| 约束 | 实现 |
| --- | --- |
| CPU 提交与取回 | 稳态两阶段合计 ≤8ms；初始化和首次导入允许一次 ≤50ms，随后使用稳态预算，取消首个 fence 也消耗冷提交资格 |
| GPU 等待 | 最大 50ms，非阻塞 timer 推进 |
| 补供给新鲜度 | 读取开始到实际消费 ≤8ms，超过期限结束当前资格 |
| 帧资格 | 原生回调、计数、尺寸、sequence、epoch 或前后台状态变化使异步结果失效 |
| 错误 | context/shader/FBO/readback/fence 错误或预算超限释放探测资源，继续原供帧与已有 renderer 回退 |
| 清理 | 取消最后 canvas 订阅释放 GPU/PBO/timer，剩余统计订阅继续运行；全部消费者关闭后回调和资源归零 |

## 生产模块回放

[回放脚本](2026-11-02-gpu-drawable-supply-audit.cjs) 直接加载生产帧源、FramePacer 和两种 renderer；WebGL/PBO/fence、视频图像及时间采用受控替身。探测器 GL 和 renderer GL 各自独立，实际 video 导入逐张记录源图像编号并检查唯一性。运行命令：

```sh
node dev-docs/validation/2026-11-02-gpu-drawable-supply-audit.cjs dev-docs/validation/2026-11-02-gpu-drawable-supply.metrics.json
```

40 组通过。两种 renderer 的诊断 off/dev 结果一致，所有组的图像导入与候选序列均具有唯一身份；退出资源全部归零。[完整指标与源码 SHA-256](2026-11-02-gpu-drawable-supply.metrics.json)。

| 10 秒输入，每种 renderer | GPU 不可用时提交 | GPU 功能启用后提交 | 补交付 |
| --- | ---: | ---: | ---: |
| 60fps 源、250ms 合批、RVFC50Hz | 500 | 600 | 100 |
| 60fps 源、250ms 合批、RVFC45Hz | 450 | 600 | 150 |
| 同上45Hz，rAF 先到 | — | 600 | 150 |
| 30fps 源 | — | 300 | 0 |
| 用户上限30fps | — | 300 | 0 |
| 静态缩略图、RVFC50Hz | — | 500 | 0 |
| 3ms GPU 完成、RVFC45Hz | — | 600 | 150 |
| 12ms 迟到 GPU 结果 | — | 500 | 0 |
| 逐帧播放计数、RVFC45Hz | — | 600 | 0 |

GPU 不可用列使用当前代码的能力失败回退，用相同受控输入隔离补供给功能的效果。此结果证明共享供给和生产绘制调度可以处理六百张不同的源图像；实际设备的转换、GPU 执行与上屏通过新日志验收。

## 回归检查

- `pnpm test`：48 文件、460 项通过，含 119 项帧源/渲染调度定向用例。
- `pnpm lint:fix`、`pnpm exec vue-tsc --noEmit`、`pnpm build`、`git diff --check` 通过；270 个生产模块构建完成。
- 既有帧供给 9 组与绘制路径 4 组、播放计数配对 36 组、抵扣窗口/节拍前后 20 组、冗余绘制路径前后 8 组回放通过。抵扣回放复用已保存历史基线，其他旧指标保留原构建边界。
- 原 Canvas2D 专项作为历史记录保留，本轮使用 GPU 状态机用例和 40 组回放验证替换后的路径。

## 新日志读取与设备步骤

`frameIdentity.videoPixels.evidence=webgl2AsyncVideoPixels` 表示新机制。`deliveredFrameCount` 是 GPU 补交付累计，`polledFrameCount` 包含计数轮询及 GPU 补交付；`frameIdentity.accepted` 和 `callbackTiming` 继续描述原回调入口观察，GPU 完成采用独立像素计数。按同一 video/renderer 代际结算增量，避免跨代际相减。

`readTiming.upload/readback/initialization` 记录 CPU 分项，`gpuWait` 记录异步等待；`readDurationTotalMs` 为 CPU 分项合计，首末读的墙钟间隔还包含 GPU 等待。`readCount = completedReadCount + cancelledReadCount + pixelProbeGpuReads`；`counterRebaseCount` 记录已配对计数批次重对齐。`pixelProbeGpuReads/pixelProbeTimers` 随 PlaybackService 资源快照透传。

用新构建保持前台、开启 SR 与 60fps 目标，连续运动画面至少 30 秒；确认补交付增量、同代际候选→提交、完整 3s/15s 稳态窗口、CPU/GPU 等待预算和退出资源。再运行一次30fps配置及SR关闭→开启，确认限帧与资源恢复。

[改动前10:59日志](2026-11-02-pixel-cost-fresh-trace-analysis.md)中，网络接收59.945/解码59.458fps、丢失0.159683%、RTT P95 28ms；SR完整3s窗48.087/58.444fps，g2/g4候选→提交296→294/176→176。一次原生video冻结1.11s伴随756ms接收停顿和恢复时drop62。本轮代码改善本地候选供给；该接收停顿保留独立网络事件归属。

内部 ReferenceChain、cleanAnchorCommitted、DisplayStable 和逐NACK有效率延续原取证边界；实际 media/steady supply、帧供给delta、submit/present 长尾继续按同窗新日志验收。既有上传0.393ms、提交年龄P95/max15ms属于改动前数据。设备 GPU 成本、缩略图与当前导入图像的精确对应、真实 submit-to-present、Windows/ROG与长测仍归属 S4。自动实机运行保持暂停。
