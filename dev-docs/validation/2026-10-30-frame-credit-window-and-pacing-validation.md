# 帧抵扣窗口与毫秒节拍修正验证

2026-10-30，用户授权继续下一步。共享帧源现采用1秒native抵扣有效窗口，现有FramePacer采用最多1ms截止点容差。普通WebGL与SR在受控后台计数分叉及毫秒取整输入下恢复60fps，用户主动30fps上限继续保持30fps。设备SR满帧和真实呈现继续由新构建日志验收。

## 故障依据与实施边界

[15:26新日志](2026-10-30-video-texture-import-fresh-trace-validation.md)证明上传调用14.665→0.436ms；SR完整窗口输出47.41/52.26fps。共享video持续约6100帧native抵扣余额、5317次轮询观察接受0；首次SR即时300候选→280提交，存在20帧latest覆盖损失。两段SR解码约60fps、freeze增量0。

本轮修改共享帧源、现有累计FramePacer、既有PlaybackService资源快照及回归。两种renderer继续沿现有HTMLVideoElement→TexImage2D→shader路径工作，帧身份/截图/context/暂停/回退和媒体生命周期沿既有所有者处理。网络与浏览器恢复事实按上份设备报告保留，自动实机操作保持暂停。

## 抵扣资格与调度合同

- native抵扣承担短时晚到播放计数的同帧去重。播放计数超过1000ms未推进时清理余额，并在计数停滞期间停止追加资格；播放计数重新推进时建立新窗口。持续未结清的余额超过1000ms时清理，覆盖计数仍推进但起点分叉的情况。
- 余额全部消费时重建资格时间；播放/native计数回退、来源改变和最后消费者取消时按原epoch清理。250/275ms批量更新与新增800ms晚到批次仍正确抵扣，native/rAF先后、48帧偏移、暂时计数缺证及原始RTP透传继续通过。
- FramePacer保留累计deadline、余数和迟到后最新帧处理，提前容差为 `min(1ms, interval×0.06)`。WebView毫秒时钟的floor/round误差得到覆盖，高速源继续受用户累计预算限制。
- 帧源资源快照与PlaybackService低频资源事件透传 `uncountedNativeFrames`、`expiredNativeFramesTotal`、`nativeCreditWindowExpirationCount`、`nativeCounterCreditWindowMs`。身份环保留schema2兼容扩展字段 `expiredNativeFrames`、`nativeCreditWindowExpired`、`counterProgressAgeMs`，仍使用惰性诊断、接受/拒绝各2Hz及最近8条证据；新计时逻辑使用现有回调时钟。

## 失败复现与生产模块回放

新增10项回归，修复前定向3文件8项失败/83项通过：

- 两种renderer在600个60fps新候选、毫秒round时钟下仅处理400次、latestOverwrite199。
- 两种renderer在统计消费者后台继续订阅、102秒native推进/播放计数冻结后，仅处理450/600次，轮询补帧0。
- 帧源后台恢复与native起点突然领先6120帧时持续吞掉轮询；FramePacer的floor/round测试失败。800ms晚到批次和高速源预算作为保留合同同时验证。

修复后定向91项通过。[生产模块回放](2026-10-30-frame-credit-window-and-pacing-audit.cjs)对同一受控输入运行修复前后各10组，直接加载生产帧源、FramePacer及两种renderer，使用既有DOM/WebGL替身：

| 每种renderer，10秒输入 | 修复前提交 | 修复后提交 | 修复后第2–10秒 |
| --- | ---: | ---: | ---: |
| 后台计数分叉后，60fps源/RVFC45Hz | 450 | 600 | 60fps |
| 后台计数分叉后，60fps源/RVFC50Hz | 500 | 600 | 60fps |
| 前台native起点突然领先6120，RVFC45Hz | 450 | 585 | 60fps |
| 60fps源，毫秒round回调/刷新 | 400 | 600 | 60fps |
| 120fps源，用户30fps上限 | 300 | 300 | 30fps |

后台分叉45/50Hz两组分别恢复150/100次轮询交付；后台余额从6120→0，期内首先过期60帧资格，之后停滞窗口持续阻止追加。前台起点跳变在1秒有效窗口后释放6120帧余额，最初一秒保留欠供给，后续稳态恢复60fps。毫秒round输入latestOverwrite199→0。主动30fps限帧下latestOverwrite899保持其正常latest-only含义。

20组均无提交失败或重复身份绘制；TexImage2D(video)导入次数与成功处理数量对应，结束后原生回调/rAF资源归零。[metrics](2026-10-30-frame-credit-window-and-pacing-validation.metrics.json)保存前后计数、稳态分母、源码哈希、检查结果和精确基线模块内容；后续Git索引变化时可使用该文件作为第三个参数复算历史基线。

原9组帧供给回放、3组48帧偏移和4组冗余绘制路径回放通过。正常供给与偏移维持30fps；只有批量身份可用的4/3.7fps用例继续按真实候选语义处理，截图/单双pass记账维持。

## 检查与设备验收

`pnpm test`：48文件423项通过。`pnpm lint:fix`、`pnpm exec vue-tsc --noEmit`、`pnpm build`和diff检查通过。资源透传补齐后全量与类型/构建再次通过；本轮检查范围覆盖统计/监督消费者、renderer、旧配置及公共播放生命周期。

上一份设备全媒体丢失0.05090%、RTT中位14ms/P9564ms；前台两次freeze/1.319秒，其中约1秒收包观测停顿后丢弃63帧。此类网络/接收停顿与本次确定性补帧缺陷分别取证；浏览器内部ReferenceChain、cleanAnchorCommitted、DisplayStable与逐请求NACK有效率继续缺证。当前离线回放只证明帧供给/调度和资源记账，GPU执行、像素更新、真正screen present及submit-to-present长尾需要设备证据。

1秒资格过期后，首批播放计数与native同帧关系会重新推断；latest-only最多交付当前一帧，重叠批次的像素身份及呈现需要实机核对。修正目标为阻止历史余额持续压低新帧供给，并保留短期晚到去重。实机播放计数若只能批量更新，候选供给继续受该浏览器事实约束。

新构建验收使用同机1440p/60fps持续前台SR至少30秒，并包含一次后台→前台恢复。核对当前源码fingerprint、TexImage2D上传成本、抵扣过期/余额、native与轮询接受增量、同代际候选→提交98%、完整3s/15s输出、回退/试探、画面与截图；网络冻结按接收增量、RTT/jitter及缓冲长尾同时解释。S4继续保持SR效果60fps、零拷贝/GPU、Windows/ROG和2h/8h验收。

复算：`node dev-docs/validation/2026-10-30-frame-credit-window-and-pacing-audit.cjs <output.json> [baseline.metrics.json]`。
