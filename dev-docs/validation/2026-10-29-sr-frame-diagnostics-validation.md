# SR 帧身份与显示状态日志补齐

2026-10-29：承接[离线核验](2026-10-29-sr-frame-delivery-offline-audit.md)，已补齐共享帧源判定、双向抵扣与当前 renderer 的 DOM 显示状态。字段随现有 dev `renderTelemetryObserved` 进入日志，帧率预算、SR算法和新帧判定继续沿既有行为执行。

## 日志位置与读法

新增路径为 `renderTelemetryObserved.payload.mediaDiagnostics`，包含实际读取时刻 `observedAtMs`、当前 `rendererGeneration/rendererKind/submittedFramesTotal`、帧源 `frameIdentity`、当前显示资源 `surface` 和首帧交接中的 `pendingRenderer`。

`mediaDiagnostics.observedAtMs` 表示诊断快照时刻，外层 `sampledAtMs` 表示对应 getStats 时刻。对比帧率与累计数时按 video/renderer generation、实际读取时间及采样窗口对齐。`frameIdentity.collectionStartedAtMs` 为诊断计数起点；原有 `nativeCallbackCount` 保留视频生命周期内的累计语义。

### 帧身份与去重

- `observed/accepted/rejected` 按 `videoFrameCallback/animationFrame/timeupdate` 分列累计次数；原始回调次数与接受 marker 次数各自取证。
- `reasons` 记录实际判定分支累计数；`nativeUnchangedWithMediaAdvanceCount` 表示 native计数固定、mediaTime推进的输入冲突。
- `evidence` 保留原始及前次 `presentedFrames/mediaTime/rtpTimestamp`、原始与判定使用的native增量、播放计数来源/前值/当前值/增量、counter reset、native/counter有效推进量、最终 `newFrame/localSequence/presentedFramesDelta`。
- `uncountedNativeBefore/After` 与 `counterAheadBefore/After`、`creditedNativeFrames` 展示两种抵扣方向。`presentationTime/expectedDisplayTime` 作为浏览器时间事实保留。
- 数值状态分为missing/invalid/initial/advanced/unchanged/regressed；无效数值显式为null。RTP回退状态需结合32位时钟回绕解释，状态只用于观测。

| reason | 对应分支 |
| --- | --- |
| counterAdvanced | 播放计数抵扣后仍有新推进 |
| nativeAdvanced | 原始RVFC身份抵扣后仍有新推进 |
| initialNativeFrame | 首次原始回调建立帧身份 |
| mediaTimeAdvanced | fallback媒体时间推进 |
| pollAlreadyDelivered | 轮询先交付，晚到native身份被抵扣 |
| nativeAlreadyDelivered | native先交付，后到播放计数被抵扣 |
| nativeCounterUnchanged | native计数字段存在，当前判定沿固定计数路径执行；数值状态单列 |
| counterUnchanged | 轮询播放计数保持原值 |
| mediaTimeUnchanged / identityUnavailable | fallback媒体时间固定或缺证 |

### 显示资源

`surface` 由当前 renderer 自己持有的 video/canvas 生成；候选 renderer 另有 `pendingRenderer.surface`。记录连接状态、计算样式读取状态、opacity/visibility/display、position/zIndex、pointer-events、object-fit、裁剪/transform/filter是否存在、CSS矩形与canvas backing尺寸、video播放状态与当前时间、同父节点关系及DOM顺序。

记录最多四层video祖先的显示样式和矩形，超出部分通过 `ancestorsTruncated` 表示。WebGL实际 `alpha/desynchronized/preserveDrawingBuffer/antialias` 和上下文丢失状态随快照读取。样式按数值/枚举/布尔值白名单输出，filter URL、节点标识与DOM文本保持在日志之外。

`evidence=domLayout` 与 `screenPresentState=unavailable` 明确证据范围。pointer-events会改变命中测试结果，当前实现依赖拥有者绑定、DOM顺序与布局事实。实际compositor输出、屏幕像素变化、GPU耗时和真实submit-to-present继续需要相应设备证据。

## 开销与资源边界

dev trace的惰性payload首次读取时启动帧身份采集。逐次回调累计数字；接受与拒绝各最多采样2Hz，共用最近8条证据，快照返回独立副本。DOM/上下文读取随既有显示监督时钟执行，250ms getStats及基础资源快照继续只读原有数值字段。trace off/production在payload构造前过滤，诊断快照读取为0。帧源继续沿原WeakMap、订阅与取消代际回收。

## 自动验证

- 七文件115项定向测试通过：激活边界、双方抵扣、固定native计数/mediaTime冲突、RTP推进、回退/无效数值、600Hz回调采样上限、快照独立性、DOM祖先限制、上下文读取失败、renderer交接与退出归属。
- 全量45文件336项、`pnpm lint:fix`、`pnpm exec vue-tsc --noEmit`、`pnpm build`通过，生产构建268模块。
- off/production/dev三个模式下媒体决策一致；off/production诊断读取0次，dev能够写出新字段。
- 原9组离线供给复现继续通过：逐帧身份维持30fps，合批身份保持4/3.7fps。GPU采用替身，结果表达当前调度逻辑。
- [构建源码摘要、逐文件SHA-256与复现数值](2026-10-29-sr-frame-diagnostics-validation.metrics.json)绑定本轮源码；历史日志继续使用原构建身份解释。

本轮软件操作保持暂停，代码与离线验证完成。后续使用包含新字段的dev日志判断原始metadata、播放计数与抵扣的实际责任分支，再决定新帧调度调整。
