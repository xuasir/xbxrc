# Browser 逐解码帧供给交付

日期：2026-11-09；主窗口直接执行。任务：`2026-11-09-browser-decoded-frame-supply`。

2026-11-10 当前合同：默认原画及增强由独立 VideoFrame 驱动 WebGL；供给接口或渲染故障直接降级为 video 标签呈现。初始化期间等待解码供给，RVFC 继续承担呈现诊断。启动期限与降级修正见[修复报告](2026-11-10-decoded-frame-startup-deadline-fix.md)，下文记录 11/9 交付及历史验证。

## 交付

锐化与 SR 已改为从现有 WebRTC 视频轨读取独立 `VideoFrame`，到帧后立即上传并沿原 WebGL shader 提交。共享帧源选择主线程 MediaStreamTrackProcessor 或 Worker 接口；接口缺省、初始化失败/超时、reader 异常及轨结束回退到既有 RVFC/播放计数。默认 video 和呈现/输入诊断继续使用浏览器播放及原始 RVFC。

reader 的 maxBufferSize 为 1，Worker 最多一帧在途，消费完成后确认；帧源保留当前最新帧供绘制与截图，在替换、暂停、退出和异常时释放。原视频轨由 WebRTC 持有，processor 使用 clone。低帧率显式预算继续使用现有 FramePacer。

渲染健康判断按解码供给模式使用实际提交验收；生产 manifest 和 dev render telemetry 保留 rendererFrameSource，资源记录来源、回退原因、解码帧接收计数、保留帧数量和时间戳。

## 已完成验证

| 场景 | 解码 fps | 解码供给 fps | 提交 fps | 原始视频回调 Hz |
| --- | ---: | ---: | ---: | ---: |
| WKWebView / SR / 到帧抖动 | 59.96 | 59.96 | 59.96 | 59.81 |
| WKWebView / 锐化 / 到帧抖动 | 59.96 | 59.96 | 59.96 | 59.96 |
| WKWebView / SR / 输出像素采样 | 59.78 | 59.78 | 59.78 | 48.26 |
| WKWebView / 锐化 / 输出像素采样 | 59.94 | 60.10 | 60.10 | 59.94 |
| Chromium / 主线程 SR | 43.94 | 43.79 | 43.79 | 7.37 |
| Chromium / 主线程锐化 | 42.27 | 41.96 | 41.96 | 2.92 |

每段有效窗口约 6.5 秒。RTC 采样和 JS 资源读取的端点存在少量时间差；一帧差额使用端点口径解释。Chromium 夹具发送端约 42–44fps，该组验证主线程接口与解码供给到提交的配平。Windows 设备的实际 60fps、GPU 性能和物理上屏接续实机验收。

WKWebView 两种 renderer 共 18 次输出帧号采样全部通过，校验标记 `0xd3`、帧号推进及画面方向一致；CPU 上传 P95 为 1ms，绘制间隔 P95 约 19–20ms。异步 fence 观测包含 GPU 队列与轮询派发，GPU 实际执行时间和物理 scanout 使用能力限定口径。

Chromium 接口缺省模拟选择 `presentation / unsupported`，既有渲染继续工作。全部生产对照退出后，decodedFrameReaders、retainedDecodedFrames、frameRequests、nativeFrameCallbacks、animationCallbacks 归零。

自动回归：87 文件 / 810 项通过；生产 trace 合同 2 项通过；lint:fix、typecheck 与 build 通过，构建产出独立 Worker 资源。新增 5 项覆盖主线程到帧供给、原始 RVFC 分流、帧身份/异常、兼容回退、Worker 确认与迟到帧关闭；2 项覆盖低频原始回调下的健康判据。最后一次测试夹具修正为可构造的 class mock，保留上述通过结果。

## 17:53 实机日志回归与修正

最新日志 `runtime-trace-1794217868106-1.jsonl` 已运行新 Worker 供给。`seq=1101→1229` 的接收计数从82增至442，当前 VideoFrame 的 timestamp 始终为0，锐化提交累计保持1；`seq=700` 的SR已接收510帧，提交累计保持5。两段随后以 `sustainedRendererPressure` 触发保护，帧供给和提交之间出现确定的本地过滤。

本轮实现曾将相等的 timestamp 当成重复帧；WebKit实机为独立VideoFrame提供持续为0的时间戳，使后续read交付被错误过滤。已改为每次reader交付都生成独立localSequence，timestamp只作诊断；最新5项定向回归和lint:fix通过，回归覆盖两个timestamp=0的独立帧均交付与正确关闭。

日志完整RTC窗61.492秒接收59.943、解码59.959fps；丢包26、NACK24、收到重传包26，PLI/FIR增量0；短冻结1次、0.214秒。增强的持续提交停滞与零时间戳过滤有独立资源证据。修正后的设备持续帧率、GPU及物理上屏接续新一轮串流验收。[安全复算指标](../../dev-docs/validation/2026-11-09-browser-decoded-frame-supply/fresh-regression.metrics.json)。

## 六段证据与边界

- 网络与恢复：本地 H264 对照的增强窗口解码持续推进、冻结增量为零；真实设备网络/NACK/PLI 与恢复效果沿实机 trace 复算。Browser 的 Native ReferenceChain、RecoveryLedger 保持能力限定。
- Media supply 与 steady supply：WKWebView 的独立解码供给、实际提交均约 60fps；原始 RVFC 为 48.26Hz 的 SR 段仍交付约 59.78fps。
- Submit/present 长尾与帧供给 delta：上传、提交和帧号有独立观测；显式低帧预算、截图与恢复沿现有回归。物理 present、长时温控和 Windows GPU 尾延迟由设备验收。

## 证据与后续

[RFC](../rfcs/2026-11-09-browser-decoded-frame-supply.md)、[指标与原始浏览器结果](../../dev-docs/validation/2026-11-09-browser-decoded-frame-supply/metrics.json)、[最终源码身份](../../dev-docs/validation/2026-11-09-browser-decoded-frame-supply/source-identity.json)、[原菜单修复](2026-11-08-handheld-stream-menu-and-frame-trace.md)。夹具文本位于对应验证目录的 `fixture/`，其运行器依赖本地 Chrome、WKWebView probe 与原临时目录；原始结果已归档到仓库。

用户要求按已有证据收口，额外一分钟对照留待后续实机验收。当前交付状态为代码与自动验证完成；下一步使用包含本轮实现的 Windows 构建验证 SR、锐化和两个菜单的真实掌机效果。
