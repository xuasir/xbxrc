# Browser 逐解码帧供给 RFC

## Status

- Completion: 代码与自动验证完成，设备效果待验
- Current State: validation-pending
- Owner: 主窗口，单窗口执行
- Last Updated: 2026-11-10
- Authorization: 用户明确要求修改供给方案，采用到帧即渲染，并要求覆盖其他 Windows 设备的兼容性。

## Background

11/9 实机样本中接收、解码约 60fps，增强提交约 48–54fps；有效候选与提交配平。独立 1440p H264 发送端在平稳节拍下，现有 SR / 锐化均提交约 60fps；到帧抖动与帧号探测复现解码约 60fps、视频纹理图像变化和增强提交同步下降。逐 rAF 采样中的新增图像均伴随 RVFC，累计 presentedFrames 推进具有合批语义。

当前 macOS WKWebView 的主线程缺省 MediaStreamTrackProcessor，DedicatedWorker 提供该接口与 VideoFrame。Windows WebView2 的 Chromium 主线程实现通过运行时能力检测接入，Worker 实现作为第二入口，旧运行时降级为 video 标签直接呈现。

## Goal

WebRTC 视频轨的每个有效解码帧直接进入现有增强 renderer；纹理上传使用对应 VideoFrame。供给由解码输出驱动，在背压下保留最新一帧，约束队列延迟。默认原画与增强共用独立 VideoFrame 直通 WebGL；接口或渲染故障降级为 video 标签自身呈现，RVFC 继续提供呈现与协议诊断。

## Scope 与合同

- Browser main 的共享 VideoFrameSource、普通 WebGL2 / SR 纹理上传、资源观测与渲染健康判据。
- 复用现有 RTCPeerConnection、接收轨和 WebGL shader；MediaStreamTrackProcessor 消费视频轨的 clone，浏览器 WebRTC 继续持有解码责任。
- 能力检测顺序为主线程 processor、Worker processor。初始化失败或超时、首帧超时、接口缺省、reader 失败及轨结束通过既有 renderer 故障通道直接降级为 video 标签；WebGL 供给只接受独立解码帧。
- processor 的 maxBufferSize 为 1；Worker 最多一帧在途，主线程消费后确认；当前最新 VideoFrame 由帧源持有，替换与取消时释放。
- renderer 订阅解码供给；呈现与输入诊断订阅沿原始 RVFC。两类身份分别记录，渲染统计以实际候选、提交及 VideoFrame 时间戳验收。
- 低帧率显式预算继续由现有 FramePacer 处理；正常 60fps 预算到帧即提交。暂停、恢复、context loss、截图与退出保持代际隔离及资源闭环。

## Plan

1. 在 WKWebView 验证视频轨 transfer、Worker reader、VideoFrame 纹理上传和抖动供给；核对 Chromium 主线程能力。
2. 在共享帧源内实现能力适配与资源所有权；两种增强 renderer 使用同一解码帧入口。
3. 对齐资源诊断、健康判据与回归；构建验证 Worker 打包，归档可复算证据。

## Validation

- [x] WKWebView 主线程/Worker 接口检测。
- [x] WKWebView 实际解码帧 transfer 和抖动下源→候选→提交配对。
- [x] Chromium 主线程接口与旧运行时回退。
- [x] 取消、迟到帧、异常、暂停/恢复、截图及资源释放自动回归。
- [x] 普通锐化/SR 上传对应 VideoFrame，18 次输出帧号与方向校验通过。
- [x] Rust 边界保持；87 文件810项、生产trace2项、lint:fix、typecheck、build通过。
- [ ] Windows 掌机实际60fps、GPU长尾、物理上屏与两菜单体验。

## Risks

- Worker 接口存在与轨 transfer 资格分别验证；初始化采用有限期限并回退。
- VideoFrame 的色彩、翻转与纹理导入性能需要真实 WebGL 对照；完整 Windows 掌机与物理上屏由用户验收。
- 解码、视频呈现推进、增强候选、GPU 提交及物理 scanout 保持各自指标语义。

## Progress

- 2026-11-09：完成既有帧源约束复核、独立发送端与帧号探测，启动逐解码帧原型对照。
- 2026-11-09 17:53日志：新Worker供给已运行，独立VideoFrame的timestamp持续为0，时间戳去重使SR接收510→提交5、锐化接收442→提交1。修正为每次reader交付使用独立序号，时间戳仅作诊断；5项定向回归和lint:fix通过。真实设备效果接续重新进入串流验收。
- 2026-11-09：共享帧源与两种renderer完成逐解码帧适配，WKWebView正常与抖动对照约60fps，SR像素对照原始回调48.26Hz、解码与提交59.78fps；Chromium主线程供给与旧接口回退通过。用户要求按已有验证收口，交付[Report](../reports/2026-11-09-browser-decoded-frame-supply.md)，设备效果接续新构建实测。
- 2026-11-10：用户反馈 ROG 原生 video 掉帧、锐化/SR 流畅，并指定默认直通引擎、兼容降级开关及可组合效果。后续默认原画、配置和界面合同见[直通引擎 RFC](2026-11-10-default-passthrough-render-engine.md)；当前供给实现与已有证据继续复用，实施与局部检查完成，设备验收保持待验。

- 2026-11-10：用户明确默认路径与故障退路，切断初始化及故障期间 RVFC 到 WebGL 的候选分支；首帧期限从实际媒体供给进展开始。供给故障、video 降级、配置保留及显式重试由真实 PlaybackService / renderer 集成回归验收，见[修复报告](../reports/2026-11-10-decoded-frame-startup-deadline-fix.md)。
