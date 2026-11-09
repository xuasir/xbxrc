# Browser 逐解码帧供给 RFC

## Status

- Completion: 未完成
- Current State: in-progress
- Owner: 主窗口，单窗口执行
- Last Updated: 2026-11-09
- Authorization: 用户明确要求修改供给方案，采用到帧即渲染，并要求覆盖其他 Windows 设备的兼容性。

## Background

11/9 实机样本中接收、解码约 60fps，增强提交约 48–54fps；有效候选与提交配平。独立 1440p H264 发送端在平稳节拍下，现有 SR / 锐化均提交约 60fps；到帧抖动与帧号探测复现解码约 60fps、视频纹理图像变化和增强提交同步下降。逐 rAF 采样中的新增图像均伴随 RVFC，累计 presentedFrames 推进具有合批语义。

当前 macOS WKWebView 的主线程缺省 MediaStreamTrackProcessor，DedicatedWorker 提供该接口与 VideoFrame。Windows WebView2 的 Chromium 主线程实现通过运行时能力检测接入，Worker 实现作为第二入口，旧运行时继续使用现有视频帧回调。

## Goal

WebRTC 视频轨的每个有效解码帧直接进入现有增强 renderer；纹理上传使用对应 VideoFrame。供给由解码输出驱动，在背压下保留最新一帧，约束队列延迟。默认 video 保持浏览器自身播放，RVFC 继续提供呈现与协议诊断。

## Scope 与合同

- Browser main 的共享 VideoFrameSource、普通 WebGL2 / SR 纹理上传、资源观测与渲染健康判据。
- 复用现有 RTCPeerConnection、接收轨和 WebGL shader；MediaStreamTrackProcessor 消费视频轨的 clone，浏览器 WebRTC 继续持有解码责任。
- 能力检测顺序为主线程 processor、Worker processor、现有 RVFC/播放计数。初始化失败、接口缺省、轨结束使用有原因的回退。
- processor 的 maxBufferSize 为 1；Worker 最多一帧在途，主线程消费后确认；当前最新 VideoFrame 由帧源持有，替换与取消时释放。
- renderer 订阅解码供给；呈现与输入诊断订阅沿原始 RVFC。两类身份分别记录，渲染统计以实际候选、提交及 VideoFrame 时间戳验收。
- 低帧率显式预算继续由现有 FramePacer 处理；正常 60fps 预算到帧即提交。暂停、恢复、context loss、截图与退出保持代际隔离及资源闭环。

## Plan

1. 在 WKWebView 验证视频轨 transfer、Worker reader、VideoFrame 纹理上传和抖动供给；核对 Chromium 主线程能力。
2. 在共享帧源内实现能力适配与资源所有权；两种增强 renderer 使用同一解码帧入口。
3. 对齐资源诊断、健康判据与回归；构建验证 Worker 打包，归档可复算证据。

## Validation

- [x] WKWebView 主线程/Worker 接口检测。
- [ ] WKWebView 实际解码帧 transfer 和抖动下源→候选→提交配对。
- [ ] Chromium 主线程接口与旧运行时回退。
- [ ] 取消、迟到帧、异常、暂停/恢复、截图及资源释放。
- [ ] 普通锐化/SR 上传对应 VideoFrame，颜色、翻转和尺寸一致。
- [ ] cargo/Rust 边界保持；前端定向回归、lint:fix、typecheck、build、diff check。

## Risks

- Worker 接口存在与轨 transfer 资格分别验证；初始化采用有限期限并回退。
- VideoFrame 的色彩、翻转与纹理导入性能需要真实 WebGL 对照；完整 Windows 掌机与物理上屏由用户验收。
- 解码、视频呈现推进、增强候选、GPU 提交及物理 scanout 保持各自指标语义。

## Progress

- 2026-11-09：完成既有帧源约束复核、独立发送端与帧号探测，启动逐解码帧原型对照。
