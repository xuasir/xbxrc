# SR 显示层、绘制计数与帧交付离线核验

2026-10-29：源码中的主显示层为覆盖 video 的 SR canvas；成功 SR 处理帧均进入统一计数。离线生产代码复现将约 4Hz 交付定位为一种可发生的新帧筛选结果。实际设备输入与 WebView 最终合成归属继续缺证，当前调度参数保持原值。

## 范围

沿用 [16:47–16:51 持续前台日志查验](2026-10-29-browser-standard-output-fresh-trace-validation.md)，原始日志仍为最新文件。本轮采用源码入口审计和离线受控输入，应用操作与串流启动保持暂停，生产源码保持原版本。

复现直接加载当前 `VideoFrameSource`、SR renderer、RenderObservation 和 FramePacer，使用现有 renderer-scheduling 测试中的 DOM/WebGL 替身。每组输入运行 10 秒、300 次原始 RVFC；替身只验证调度与 GPU API 调用数量，实际 shader 执行、GPU 成本和屏幕像素归属另需设备证据。

- [复现脚本](2026-10-29-sr-frame-delivery-offline-audit.cjs)
- [完整输入、状态与结果](2026-10-29-sr-frame-delivery-offline-audit.metrics.json)，包含生产文件 SHA-256、抵扣状态样本及 AST 调用位置。

从仓库根目录执行：

```sh
node dev-docs/validation/2026-10-29-sr-frame-delivery-offline-audit.cjs /tmp/xbxrc-sr-offline-audit.json
```

## 显示层与计数

PlaybackService 创建持续播放的普通流内 video。SR 在 video 后插入 absolute/inset=0 的 canvas，成功绘制设 opacity=1，WebGL 请求 alpha=false。正常 CSS 层叠顺序使 canvas 覆盖 video；video 继续为纹理上传供帧。日志 seq2761 记录当前 renderer 为 generation=1 的 webgl2_sr、一个 canvas、候选 renderer=0，并记录成功绘制503次。最终 computed style、实际 context 属性与 compositor 可见输出在本日志中缺证。

AST 审计定位三个 `gl.drawArrays` 调用点：RCAS helper 一个，single-pass 放大一个，dual-pass 的 EASU 一个；统一 `record()` 位于 drawFrame 的 finally。一次成功处理帧记一次 realtimeDrawCount；双 pass 的两个 GPU draw 记一个处理帧。额外截图重绘单列，截图与实时新帧共享绘制时计入实时处理。

| 受控路径 | 实时处理帧 | 实时 GPU draw | 额外截图后的 GPU draw | 额外截图计数 |
| --- | ---: | ---: | ---: | ---: |
| 同尺寸 RCAS | 30 | 30 | 31 | 1 |
| single-pass 放大 | 30 | 30 | 31 | 1 |
| dual-pass EASU + RCAS | 30 | 60 | 62 | 1 |

截图与一个新实时帧同时到达的用例：GPU draw=1、realtimeDrawCount=1、captureDrawCount=0。全部断言通过。

StatsService 的 rendererSubmitFps 使用同代际实时累计计数增量除以真实采样间隔；资源快照中的503次与该投影同源。video advancement、原始 RVFC、接受的新帧 marker、成功 SR 处理和实际屏幕显示各有观测语义。

## 新帧筛选复现

| 输入组合 | 原始 RVFC | SR 处理 | 处理速率 |
| --- | ---: | ---: | ---: |
| native presentedFrames 与播放计数逐帧推进，mediaTime=0 | 300 | 300 | 30fps |
| native 逐帧、播放计数每250ms合批，mediaTime=0 | 300 | 300 | 30fps |
| 同上，轮询先到 | 300 | 300 | 30fps |
| native 与播放计数同时每250ms合批，mediaTime=0 | 300 | 40 | 4fps |
| 两路计数同时每275ms合批，mediaTime=0 | 300 | 37 | 3.7fps |
| 两路每275ms合批、播放计数在native回调时刷新 | 300 | 37 | 3.7fps |
| native计数固定、播放计数每250ms合批，mediaTime逐帧推进 | 300 | 40 | 4fps |
| native计数固定、播放计数逐帧推进，mediaTime=0 | 300 | 300 | 30fps |
| native计数字段缺失、播放计数固定，mediaTime逐帧推进 | 300 | 300 | 30fps |

全部用例成功，pacingWaitCount、latestOverwriteCount、submitFailureCount均为0。两路275ms合批用例中，累计 advancement=297、SR处理=37，说明约30fps计数推进可以与约3.7fps候选交付同时成立。合批时间为人工输入，用于验证机制；实际设备周期需原始字段确认。

代码的明确边界在 video-frame-source.ts 的 nativeNewFrame 分支：`presentedFrames` 字段存在时，判定依据该字段的增量；字段缺失时采用 mediaTime。固定计数字段伴随持续 mediaTime 的输入复现了4fps筛选结果，证明该分支对存在但失真的计数缺少可靠性判断。逐帧 native 输入的对照组证明抵扣逻辑在本轮正常与合批播放计数场景下可以保持30fps。

## 与真实日志的对应及结论强度

- seq2761：原始RVFC=3366、轮询补足=20、实时处理=503；generation保持1。完整3秒窗 marker/SR 均约3.6fps，接收/解码/video计数推进约30fps。主要排查位置为共享帧源形成新帧候选的过程。
- seq991：一个窗口接受4次marker、累计推进32帧、最大交付间隔284ms；seq2010同为4次/32帧，最大间隔267ms。这与合批输入复现具有相同计数关系。
- 整份trace中逐次原始 presentedFrames/mediaTime、播放计数值、抵扣债务与可见层状态字段均缺证；日志当前只保留投影和累计值。具体设备输入机制保持待证。
- 1440p与720p段同步上传/提交成本中位分别32.68ms、12.28ms，低交付在两段持续存在；成功提交耗时全段最大47ms。SR成本继续作为独立性能项，当前证据优先支持排查新帧筛选。
- 网络归一丢包约0.110%，接收/解码长期约30fps；RTT中位205ms。浏览器内部ReferenceChain、cleanAnchorCommitted、DisplayStable、真实submit-to-present与glass-to-glass均为unavailable，继续沿既有日志报告的证据边界解释。

## 后续决策

当前结论支持统一计数覆盖和正常CSS层叠意图；用户流畅观感与低SR处理计数之间的差异仍需要实际输出归属证据。后续优先为现有帧源补充有界数值观测：原始回调与轮询次数、metadata/counter增量、计数字段有效性、抵扣前后状态、新帧接受/拒绝原因；显示侧记录当前renderer归属、video/canvas实际样式与覆盖区域。保持现有帧率预算，在这些证据对齐后决定新帧判定调整，1440p提交成本单独验收。

2026-10-29 后续[帧身份与显示状态日志补齐](2026-10-29-sr-frame-diagnostics-validation.md)已完成代码与336项自动验证；原始metadata、播放计数、双向抵扣和DOM显示状态进入现有dev trace，调度逻辑继续沿原行为执行。
