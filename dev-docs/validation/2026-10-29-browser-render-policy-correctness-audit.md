# 2026-10-29 浏览器显示、SR 与观测策略正确性复核

## 审计结论

策略确实会影响画面清晰度与帧节奏。已定位自动输出降采样、SR 时间限流、瞬时接收帧率与内容帧率合同冲突、源视频观察与实际 canvas 绘制错位、热路径 trace 过密等问题。资源归零与共享统计验证仍有效，策略正确性和运行效果继续待修复验收。

本次按用户要求聚焦代码策略；使用既有 Mac 日志、生产代码和生产纯函数离线复算。自动实机运行保持暂停。掌机具体系统缩放/刷新率与对应日志尚待绑定；1440p→1080p 路径已在生产函数复现，并在 Mac 日志观察到真实触发。

2026-10-29 后续源码复核补充整体连锁：`evaluateProfileBandwidthState` 把 `presentAgeMs` 纳入 warning/congested；`classifyRecoveryCause` 的实际码率占请求上限比可单项判网络拥塞；`highRtt` overlay 收紧 packet/present/feedback 边界；`resolveAdaptiveRenderProfile` 与 RCAS 消费这些网络标签修改本地显示。新的[整体策略重设计](../../docs/isu/2026-10-29-browser-policy-stability-and-native-quality.md)采用源尺寸硬下限、按故障域控制和稳定新帧供给。本文逐项修复建议按该最新方向实施，生产源码修复继续待完成。

## 1. 解码之后仍有明显帧供给损失

在同一 peer、video、SR renderer generation 内对齐资源快照与 getStats 累计计数，排除长后台段：

| 同一前台窗口 | 接收 | 解码 | 帧回调序号推进 | SR drawCount 推进 |
| --- | ---: | ---: | ---: | ---: |
| 72.152s，资源 seq 1035–3823，stats seq 1029–3818 | 2159 / 29.92fps | 2047 / 28.37fps | 1174 / 16.27次每秒 | 1025 / 14.21次每秒 |
| 22.047s，资源 seq 4941–5684，stats seq 4943–5681 | 613 / 27.80fps | 365 / 16.56fps | 142 / 6.44次每秒 | 130 / 5.90次每秒 |

资源时刻使用前端单调时钟与 trace 的映射，stats 取最近样本，存在一个采样周期内的边界误差。两端 renderer generation 均为 1；drawCount 表达 canvas 提交绘制，系统实际扫描上屏仍需另外测量。最后窗口含截图读回，其额外绘制最多为日志记录的两次。

帧链至少包含三个不同环节：接收→解码、解码→浏览器回调交付、回调交付→SR 提交。回调合批与 `presentedFramesDelta>1` 会让底层 video 推进量高于 callback 数量，latest-only 每次回调只能处理当前最新纹理。SR 额外预算又继续减少绘制机会。各环节具体损失帧的身份与原因需要下一批修复形成计数闭环。

## 2. 呈现统计与显示监督绑定的是底层 video

- [PlaybackService.ts](../../src/player/app/media/PlaybackService.ts:257) 在 video 的 RVFC 回调发送 `media.videoFramePresented`。
- [StatsService.ts](../../src/player/app/stats/StatsService.ts:239) 按该事件的 `presentedFramesDelta` 计算 `presentFps`。
- SR 的实际绘制记录由 [render-observation.ts](../../src/player/infra/render/render-observation.ts:24) 更新 `drawCount/lastDrawSequence`，当前只进入资源诊断。
- [browser-runtime.ts](../../src/streaming/runtime/browser-runtime.ts:1395) 用 video 回调刷新 `lastMediaActivityAt`；显示原因判断读取 video 的 `presentAgeMs` 和 callback gap。

因此当前面板 presentFps 支持“video/compositor 推进”解释，canvas 提交 FPS 需要独立事实。底层 video 继续推进且 SR 停绘/少绘时，当前显示监督可能继续收到健康活动。将渲染器配置投影为实际 `processingMode=performance` 也只证明配置字段，SR Processor 当前更新入口没有消费 processingMode 来切换算法成本。

修复合同：保留 browser received / decoded / video presented / callback / renderer submitted 五项独立事实；显示健康度消费当前 renderer 的提交进展，协议 metadata 继续按其自身阶段语义核验。

## 3. SR 的高刷新率限流与 30fps 边界问题

[SuperResolutionWebGL2Renderer.ts](../../src/player/infra/render/SuperResolutionWebGL2Renderer.ts:873) 使用 `floor(1000/targetFps)` 与 `now-lastFrameTime` 比较，每次成功绘制将 lastFrameTime 重置为 now。SR 新门槛在有新帧且时间预算通过后绘制；本轮 diff 把既有“新帧可以通过时间门槛”的行为改为同时满足两项，造成新的节拍风险。

提取生产 shouldDraw 方法，按固定 rAF 节拍离线复算，假定每个 tick 均有新帧，仅验证限流上界：

| 显示刷新率 | 目标 FPS | 当前限流稳定上界 |
| --- | ---: | ---: |
| 60Hz | 60 | 60 |
| 90Hz | 60 | 45 |
| 120Hz | 60 | 60 |
| 144Hz | 60 | 48 |
| 165Hz | 60 | 55 |

144Hz 的相邻 tick 约 6.944ms，两个 tick 约 13.889ms，三个 tick 约 20.833ms；固定 16ms 门槛导致每三个 tick 才画一次。这是生产方法可直接复算的策略问题，实际 GPU 成本会进一步影响结果。

普通 [Renderers.ts](../../src/player/infra/render/Renderers.ts:106) 在目标 >=60 时按新帧绘制，目标 30 时采用同类 lastFrameTime 限流。交替 32/34ms 的约 30.3fps 视频回调在 300 次回调中只通过 150 次绘制。视频已接近目标时，边界抖动仍造成周期性跳帧。

修复合同：同帧至多处理一次，累计截止时间保持平均节拍；late/latest-only 与主动限流分别记录。SR 等待新帧时保留一次待调度机会，后台 targetFps=0 暂停常驻 draw rAF，截图由有界请求唤起。

## 4. 1440p→1080p 的清晰度损失是可复现的策略路径

[browser-render-policy.ts](../../src/streaming/runtime/browser-render-policy.ts:233) 的 L2 同时设置 `maxOutputPixels=1920*1080` 与 `maxOutputDevicePixelRatio=1`。这个性能预算进入 SR 的 D，后续 clamp 与 SR renderer 再按 present target 裁剪，CSS viewport 继续铺满窗口。

用生产 `resolveSuperResolutionDisplayBudget`、`resolveSuperResolutionTierPlan`、`resolveBrowserRendererPlan` 复算 2560×1440 源和显示：

| 系统缩放示例 | 档位 | SR backing 输出 | CSS 最终显示 | 输出语义 |
| --- | --- | --- | --- | --- |
| DPR 1 | L0 / L1 | 2560×1440 | 2560×1440 | 同尺寸 |
| DPR 1 | L2 | 1920×1080 | 2560×1440 | 先降采样，再由合成器放大 |
| DPR 1.5，约 1707×960 CSS | L1 | 2133×1200 | 约 2560×1440物理像素 | 输出低于源，plan仍为1440p/passthrough |
| DPR 1.5，约 1707×960 CSS | L2 | 1707×960 | 约 2560×1440物理像素 | 输出约960p，档位标签为1080p |

“屏幕像素需求”与“临时性能预算”进入同一个 D。性能预算压低后，SR 的“显示本身小于源”例外允许缩小真实1440p源。下游裁剪只改宽高，mode/outputTier 保留上游值；面板又优先采用 frozen 输出标签，实际 backing 尺寸继续缺失。

Mac 日志 seq 1055、1226、1415 均明确记录 `actualSourceTier=1440p / displayTier=1080p / mode=present-downscale / outputTier=1080p`；后续又回到1440p。33次SR重新冻结形成多次输出变化，包含1080/1440及720p直通。

修复合同：独立确认真实显示需求与性能限制；1440p 源在1440p显示区域保持源信息量。SR负责实际放大场景，已有足够分辨率的源进入等尺寸呈现/后处理。临时画质预算变化必须具备渲染器成本证据及稳定驻留。诊断直接读取当前 processor 的 outW/outH 与实际 pass/mode。

## 5. 瞬时接收 FPS 被当成远端显式 FPS

本轮 [browser-stats.ts](../../src/player/app/stats/browser-stats.ts:144) 把 `stats.fps` 写成250ms累计差值产生的 inboundVideoFps。[browser-runtime-profile.ts](../../src/streaming/runtime/browser-runtime-profile.ts:180) 将它作为“远端显式帧率”，优先级高于近窗稳定上沿。Mac 的 mediaTime=0 使独立源节拍分支持续缺证。

生产函数复算：即使稳定 ceiling=60，fps=24/30/36均返回 content30；fps=40/56/72/0返回content60。间歇供给和合批接收会切换内容分类，进一步切换 renderer 30/60预算。本次日志目标FPS变化25次。

修复合同：远端配置/协商的帧率提示、源媒体节拍、实际接收吞吐分别表达。内容分类使用有效源证据与稳定窗口，缺证保留unknown；采样抖动保持为观察事实。

## 6. 回调异常、显示降级与锐化变化形成过强耦合

[browser-runtime.ts](../../src/streaming/runtime/browser-runtime.ts:1356) 把 RVFC callback gap 直接记为 renderBackpressure；`resolveRenderCause` 按 gap或video presentAge 判 renderStarvation，连续三个监督观察升级 L2。网络/源供给稀疏、浏览器 callback合批和 JS绘制阻塞会共享这个入口。

L2降低输出分辨率，L1/L2改变用户画面 fit，SR冻结重算触发canvas大小/FBO重建。下一次观察继续读相同video事实，实际处理成本改善缺少配对效果验证。显示档位55次、FPS预算25次、SR冻结33次支持策略反复改变实际输出的结论。

动态RCAS还以实际接收码率除以用户请求上限判断锐化强度。[browser-render-policy.ts](../../src/streaming/runtime/browser-render-policy.ts:160) 在请求100Mbps、实际2Mbps时，即使网络置信度high/状态stable/画质L0/renderStable，仍把基线stops 0.88提高为1.00；L1升至1.06。stops增加使锐化变弱，静态/低运动内容会被该比值一并影响。

修复合同：回调稀疏、源断供、解码压力、renderer提交压力分别判断；本地输出质量调整消费renderer成本与效果，连续性按时间覆盖验证。RCAS与源分辨率和用户画质保持稳定基线，实测编码压力与有效置信度决定有限调整。

## 7. 单采集有效，观测热路径继续需要收敛

- StatsService 当前一个getStats入口、250ms时钟、同peer single-flight，日志中调用耗时0–25ms，时间区间保持顺序。这部分提供正确的采集合同。
- 1119条renderFrameDropped、627条renderBackpressureChanged逐次从帧回调发送RPC，分别约0.87/0.48MB序列化行；480条完整stats约2.13MB。高频事件在本次trace中进入key lane。
- 前端 `recordRuntimeTraceEvent` 直接进入 `runtimeTrace.recordEvent` 和Tauri invoke；profile过滤发生在Rust recorder，关闭落盘后前端构造/IPC仍有执行成本。
- 相同档位L2→L2评估13次；相同级别超过驻留期后仍写displayDegradeLevelChanged，重复事件需要表达为周期summary或实际changed。
- SR每帧执行getError/部分分支checkFramebufferStatus，画质输出改变时重建FBO/texture。这些查询和分配进入GPU相关热路径，日志只有累计drawDurationMax=204ms，具体分项占比继续待测。

修复合同：帧热路径做有界计数，按已有1s样本聚合；真实边界/错误事件即时发送，重复事件限频；trace在前端入口判定启用/级别并保留诊断功能。FBO完整性核验绑定创建/resize，运行期错误观察采用有界策略并保留硬故障回退。采集、窗口、策略、上传、draw与GPU查询分别记录有界耗时摘要。

## 验证证据与下一批顺序

离线复算使用生产TS函数与提取的生产限流方法，结果见[源函数复算](2026-10-29-browser-render-policy-correctness-audit.replay.json)。复算覆盖像素预算、内容FPS语义和理想限流上界；日志覆盖真实运行结果。掌机的具体触发时间、GPU成本与实际scanout长尾继续待绑定设备日志。

1. P0修正SR像素保留、mode/尺寸诊断、内容FPS合同和高刷新率限流；增加跨源FPS/刷新率/缩放与32/34ms时序验收。
2. P1以当前renderer提交事实监督显示，按帧链对齐计数和delta，验证接收/解码健康时canvas供给。
3. P1收敛自动显示降级、RCAS变化、SR重建、相同意图和trace热路径；保留能力、截图、用户开关及故障回退。
4. 真实日志按相同源、显示、前台窗口验证开/关SR的输出像素、帧节奏、draw成本和画质；网络统计作为归因上下文。

本轮新增审计证据并修订优化重点。生产实现和实机运行保持当前状态；任务收口以像素保真、renderer供给、策略稳定与完整功能验收为准。
