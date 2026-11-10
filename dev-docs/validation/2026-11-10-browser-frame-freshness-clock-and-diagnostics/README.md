# 帧时钟、诊断调度和空窗验证

日期：2026-11-10。主窗口直接实施。代码与自动验证完成，真实远端串流和物理上屏接续用户实机验收。

## 实施与自动门

| 项目 | 结果 |
| --- | --- |
| renderer、Worker/reader、时钟、诊断、播放降级、统计、runtime 和生产投影 | 38 文件、472 项通过，[输出](regression.log) |
| Rust/TypeScript 共享日志合同 | Rust 4 项通过，[输出](rust-contract.log)；新增校准、异常元组和完成时序共用 JSON fixture |
| 前端 lint、typecheck、build | 通过；构建产出独立 decoded-frame Worker，[lint](lint.log)、[typecheck](typecheck.log)、[build](build.log) |
| 30 分钟生产日志成本门 | 2 项通过，[输出](production-budget.log)、[指标](production-metrics.json) |
| GPU 完成代理 | 首帧后预热 1 秒，独立任务插入；单 fence、1Hz、零等待查询、最多 8 次轮询；插入 CPU 超过 4ms 后释放并停止采样 |
| 资源退出 | reader/retained frame/RVFC/rAF/像素诊断资源归零；queued fence、watchdog、longtask observer 清理有回归覆盖 |

成本门实际写入 1801 条健康记录，常规 326196.93B/min ≤327680，异常单分钟最大 127687B ≤131072，健康编码 P95 0.1665ms ≤0.5ms，最大健康行 3872B。producer 丢弃、超长、IPC 失败及结束时 pending/inFlight 全部为 0。硬预算保持原值，完整 GPU 时间戳放入异常样本，常规行保留完成状态与上下界。

## WKWebView 夹具

独立前台 WKWebView 使用生产 renderer、reader、Worker、时钟和探针源码，本地 H264 WebRTC 输入 2560×1440；每模式预热 2 秒、测量 8 秒。系统为 macOS 27.0（26A428）。[原始结果](webkit-result.json)、[安全指标](webkit-metrics.json)、[源码 SHA256](source-identity.json)和[夹具](fixture/entry.ts)保存在本目录。

| 指标 | USM 锐化 | 同档 SR |
| --- | ---: | ---: |
| 配对接收 / 解码 / WebGL 提交 | 58.610 / 58.610 / 58.610fps | 58.860 / 58.860 / 58.860fps |
| read / sent / received / delivered / submitted | 563 / 563 / 563 / 563 / 563 | 565 / 565 / 565 / 565 / 565 |
| overwrite / stale drop | 0 / 0 | 0 / 0 |
| invalid / unknown clock | 0 / 0 | 0 / 0 |
| 校准样本 / 误差范围 | 4 / ±2ms | 4 / ±2ms |
| 时钟量化调整次数 | 158 | 64 |
| ACK delivery / roundTrip 有效样本 | 562 / 562 | 564 / 564 |
| read 完成→提交 P95 / 全程最大 | 1 / 10ms | 1 / 9ms |
| receive→提交 P95 | 1ms | 1ms |
| transfer P95 | 1ms | 0ms，处于量化范围 |
| CPU upload P95 | 1ms | 1ms |
| GPU fence 完成上界 P95 / 最大 | 5 / 5ms | 5 / 5ms |
| GPU fence 完成样本 / 超时 | 9 / 0 | 9 / 0 |
| fence 插入 CPU 最大 | 0ms，1ms 计时精度内 | 0ms，1ms 计时精度内 |
| 应用帧源 animationCallbacks / polledFrameCount | 0 / 0 | 0 / 0 |
| pixelProbeGpuReads / pixelProbeCanvases | 0 / 0 | 0 / 0 |

ACK 事实随下一份帧消息回传，最后一帧尚未带回的 ACK 单列为采样边界。夹具自身的 rAF 用于显示刷新对照；应用帧源 rAF 计数独立为 0。SR 输入/输出同为 1440p，`superResolutionActive=false`，本夹具验证同档装配、供给和诊断开销；实际 EASU 放大沿既有像素/采样测试与后续低分辨率实机样本验收。

GPU 数字表达命令队列 fence 的异步完成上下界，包含插入任务派发、查询与轮询误差。队列在 fence 插入前可包含后续提交，逐帧完成归因和物理 scanout 需要对应证据。定向测试保证上界包含查询本身的 CPU 时间。零等待查询使用 `clientWaitSync(sync, 0, 0)`，平台调用的 CPU 成本仍由实际运行记录限定。

前一次夹具暴露了首张 fence 的约 79ms 初始化开销，已通过预热、独立任务插入和 4ms 成本熔断修复；[旧结果](webkit-before-deferred-fence.json)保留作为定位证据。第二次夹具捕获 Worker read 完成 / send 之间 1ms 倒序，旧时钟判断把它记入 invalid；[量化修复前结果](webkit-before-quantization.json)保留。最终代码对同域 1ms 倒序做单调修正，跨域误差单独加入 main/Worker 各 1ms 量化容差，大于误差范围的事件继续分类记录。

## 空窗取证合同

固定四项异常环、每秒最多一个数值元组，超过 33.33ms 的 readWait、receiveGap、transfer、consume，陈旧丢弃及缺失/无效时钟触发取证。元组 `times` 按以下顺序保存原始 epoch 时间：

1. Worker reader.read 发起。
2. Worker 读取完成。
3. Worker send。
4. main receive。
5. main consume 完成。
6. main 发送同序号 ACK。
7. Worker 收到同序号 ACK；下一帧消息补入，尚未回传时保留 0。

`offsetMs` 为 Worker→main 偏移；计算跨域差值前应用偏移并保留 `uncertaintyMs`。`startup` 区分首帧供给等待。longtask 在支持平台使用固定八项缓冲，在快照时关联；本机 WKWebView `taskState=unavailable`，readWait 仍覆盖上游供给与 Worker 调度。最终异常主要是 34–35ms 的 readWait/receiveGap，同序号 send→receive/consume/ACK 较短，支持将该夹具空窗定位到处理器可读帧供给侧。具体 WebRTC 解码输出与 Worker 内部调度的进一步拆分，需要引擎调度事实。

## 六维证据

| 维度 | 结论与边界 |
| --- | --- |
| 网络健康 | 本地配对窗口 lost/NACK/freeze/dropped 增量均为 0。远端基线沿用 12:30 trace：RTT P95 18ms、jitter P95 7ms、loss 约 0.024%；远端 1440p 已按用户确认收口 |
| 恢复链 | 夹具持续解码与绘制；Browser ReferenceChain/cleanAnchorCommitted/DisplayStable 按能力单列。12:30 基线完整播放窗 PLI/FIR 增量为 0，NACK/重传持续可用 |
| media supply | 本地收帧/解码约 58.6–58.9fps，提交跟随实际输入；独立处理器供给保持默认主线 |
| steady supply | 8 秒配对提交与解码相等；短测结果用于验证实现，远端长测的完整 3s 窗口接续日志验收 |
| submit/present 长尾 | 读取后提交 P95 1ms，GPU fence 完成上界 P95 5ms；video RVFC 和 canvas 物理上屏分别保留各自语义 |
| 帧供给 delta | 本地 read→submit 563/565 帧逐层配平，覆盖/陈旧丢弃为 0；远端空窗按同序号时间元组复盘，并继续优先提交新鲜帧 |

远端基线见[12:30 分析](../2026-11-10-worker-freshness-latest-trace-analysis.md)。这次独立夹具提供本地实现证据，远端网络与实际用户窗口负载通过 fresh trace 验证。

## CPU 像素拷贝审计

应用实时路径为 `track.clone → MediaStreamTrackProcessor → VideoFrame transfer list → texImage2D(VideoFrame) → GPU shader`。Reader/Worker、帧源、WebGL、EASU 重建与新 GPU 探针的守卫禁止 `copyTo`、CPU ImageData、2D canvas 中转、drawImage、readPixels 和 getBufferSubData；renderer 测试确认上传对象保持同一 VideoFrame，并让 copyTo 一旦执行即抛错。最终夹具帧格式为 NV12，默认像素诊断关闭。

`fsr1-cpu.ts` 只计算尺寸/锐度 uniform 常量；像素重建与锐化由 GLSL 完成。用户主动截图或显式启用像素诊断有独立的读回路径，常规实时供给与本次诊断调度保持关闭。

公开 WebKit main 快照的相关实现已核验，下载日期与 SHA256 在 source-identity.json 中：

| 位置 | 源码事实 |
| --- | --- |
| WebGLRenderingContextBase.cpp:3346 | Cocoa VideoFrame 快速导入候选格式为 I420/NV12 |
| WebGLRenderingContextBase.cpp:3592 | 快速导入合同要求 TexImage2D、完整源矩形、RGBA/RGB 与 UNSIGNED_BYTE；现有调用符合 |
| WebGLRenderingContextBase.cpp:3758 | GPU→GPU fast path 优先；导入失败或格式门未通过时进入 videoFrameToNativeImage 软件路径 |
| GraphicsContextGLCocoa.mm:1196 | NV12 pixelBuffer 使用 IOSurface；其他格式可经 PixelBufferConformerCV 转换 |
| GraphicsContextGLCocoa.mm:1244 | IOSurface 作为 EGL pbuffer 绑定成源纹理 |
| GraphicsContextGLCocoa.mm:1271 | copyTextureFromVideoFrame 使用 GL_CopyTextureCHROMIUM 做纹理导入 |
| VideoFrameCV.mm:693 | clone 复用 pixelBuffer 引用 |

本次保留 texImage2D，公开快路径对 TexSubImage2D 有 GateFunction 限制。应用显式 CPU 像素拷贝为 0；当前 NV12 与上传参数满足公开引擎 GPU 导入条件。已安装 WebKit 内部实际路径、系统软件回退及完整端到端零拷贝保证需要引擎级追踪，公开 WebGL API 当前提供参数和帧格式证据。

## 复验

夹具原始 root 为 `/tmp/xbxrc-2026-11-10-clock-diagnostics-probe`，原始 runner 使用该固定目录和已有 probe-webkit；归档保存源码、打包产物、输入清单与 runner 身份，原生夹具二进制 SHA 留在 source-identity.json。复用原生夹具后可执行：

```sh
node /tmp/xbxrc-2026-11-10-clock-diagnostics-probe/run-webkit.mjs --modes=sharp,sr --tag=freshness-clock-final --duration-ms=8000
pnpm test:trace-production
```

真实串流验收按默认、SR、锐化各留稳定段，优先核对校准/ACK 有效覆盖、原始异常时序、分层帧供给 delta、readToSubmit 与 GPU 完成上界；结合网络、恢复和完整 steady 窗口评估画面新鲜度。
