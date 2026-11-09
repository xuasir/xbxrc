# 2026-11-09 SR 持续供帧与诊断干扰续查

## 当前结论

已修复一项明确的诊断干扰：常规 dev trace 首次读取媒体诊断时会自动准备像素探测器；原实机记录同步 shader 准备 155ms、回调长尾 160ms。本机 WKWebView 对照复现了 149ms 准备，其中 shader 为 146ms。现在默认采集数值帧身份，像素取证通过显式选项开启。

[原实机矩阵](2026-11-08-sr-sharpen-render-matrix.md)的持续缺口继续成立：SR 提交 49.38 / 40.15 / 54.59 / 56.46fps，video 完整窗口平均推进 59.81fps；排除切换初期后依然存在差额。[11月9日新日志](2026-11-09-sr-sharpen-fresh-trace-analysis.md)确认140份诊断的像素准备/读取均为0，SR主段58.32fps、末尾3秒53.11fps，再试48.17fps；持续满帧状态保持待修复。

本轮最终的 1440p H.264 WebRTC 隔离对照中，RGB / RGBA 两段均接收、解码、提交 380 帧，约 58.45fps，解码到提交承接率 100%。当前 1:1 RCAS 在这个受控环境里能够承接源供给。原实机的额外差额更集中于完整应用运行负载、诊断开关、WebView 调度及视频纹理导入/合成的组合；各因素的贡献需要新实机证据确认。

## 修改与边界

- `MediaDiagnosticsOptions.videoPixels` 经 PlayerClient → MediaService → PlaybackService → 共享 VideoFrameSource 透传。缺省调用保留帧元数据取证，显式 `true` 开启像素探测，显式 `false` 清理探测资源；最后消费者退出时恢复缺省状态。
- 已保留现有 Browser VideoFrameSource、帧身份去重、即时提交、显式 FPS 预算与截图合同。像素探测继续承担低频诊断，供帧资格继续来自原始回调及播放计数。
- SR 继续采用原 RGB 视频上传。格式对照的稳定收益证据有限，当前修改集中于诊断开关。

```ts
client.snapshotMediaDiagnostics() // 常规数值诊断
client.snapshotMediaDiagnostics({ videoPixels: true }) // 显式像素取证
client.snapshotMediaDiagnostics({ videoPixels: false }) // 结束像素取证并释放资源
```

## 受控对照

完整累计值、统计口径、原始结果 SHA 和生产源码指纹见[指标](2026-11-09-sr-frame-supply-investigation.metrics.json)。所有实验使用生产 renderer 类，临时夹具留在 `/tmp/xbxrc-sr-render-probe/`，测试窗口与服务完成后关闭。

### 文件视频与格式

源为同源 HTTP 的 H.264 Constrained Baseline MP4，2560×1440、60/1、720 帧/12 秒。输出固定 2560×1440，SR 使用 1:1 RCAS、stops 0.88。

| 引擎 / 路径 | 窗口 | 回调 fps | 实际 canvas 提交 fps | 保留上传 P95 | 保留绘制间隔 P95 |
| --- | --- | ---: | ---: | ---: | ---: |
| Chromium / RGB，正序 | 3.5s | 59.94 | 59.94 | 约 0.4ms | 约 17ms |
| Chromium / RGBA，两次 | 各 3.5s | 59.67 / 59.69 | 59.67 / 59.69 | 约 0.4ms | 约 17ms |
| Chromium / RGB，反序 | 3.5s | 57.67 | 58.25 | 约 0.4ms | 约 17ms |
| WKWebView / 原始 RVFC，直接订阅 | 6.5s | 55.69 | — | — | — |
| WKWebView / video，共享帧源 | 各 6.5s | 57.22 / 56.45 | — | — | — |
| WKWebView / RGB | 6.5s | 55.98 | 55.98 | 2ms | 33ms |
| WKWebView / RGBA | 6.5s | 55.68 | 55.68 | 2ms | 33ms |

Chromium 使用真实 ANGLE Metal / Apple M2 Pro，初步对照预热 1.2 秒。WKWebView 扩展对照预热 2 秒，持续可见；88 份原生窗口抽样均可见，rAF 约 60Hz。原始 RVFC 直接订阅也存在稀疏回调，文件视频的回调行为需要按 AVFoundation 文件路径解释。格式切换的收益未形成稳定差额。

WebKit 文件播放计数约 2 秒合批，超出共享帧源的 1 秒 native credit 窗口；该实验的统一 `derivedAdvanceFps` 出现 107–127 的重复累计。文件对照以上表回调/提交以及原始 metadata 为依据，统一 advance 数值标为源 FPS 无效。WebRTC 对照中的播放计数约 4Hz 更新，按自己的累计值与原始身份对齐。

### 最终 1440p H.264 WebRTC

本机 Canvas → H.264 RTCPeerConnection → HTMLVideoElement → 生产 renderer。发送上限 60fps / 12Mbps，显式 2560×1440 轨道约束、`scaleResolutionDownBy=1`、`maintain-resolution`；每个窗口前后都验证真实入站尺寸。输出始终为 2560×1440，实际执行 1:1 RCAS。

| 路径 | 接收 / 解码 fps | 回调 / 候选 fps | 实际提交 fps | 解码到提交差额 | 保留绘制间隔 P95 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 原始 RVFC 直接订阅 | 58.14 / 58.14 | 58.30 / 58.30 | — | — | — |
| video，共享帧源 | 58.30 / 58.30 | 58.30 / 58.30 | — | — | — |
| SR RGB | 58.45 / 58.45 | 58.45 / 58.45 | 58.45 | 0 帧 | 18ms |
| SR RGBA | 58.45 / 58.45 | 58.45 / 58.45 | 58.45 | 0 帧 | 18ms |
| video，共享帧源，再次对照 | 58.30 / 58.30 | 58.30 / 58.30 | — | — | — |

全部窗口前台有效，90 份原生窗口抽样均可见；各段预热 2 秒、测量约 6.5 秒，rAF 约 59.99Hz。回环源自身接收/解码约 58.3–58.5fps；对照确认的是 SR 承接率。真实 60fps 源、游戏运动画面和持续前台长测继续按实机合同验收。

后台文件视频样本已剔除。首次回环的可用视频准备超时由测试夹具补齐 ICE 候选交接和显式 play 后接续；另一次 640×360 → 1440p 放大实验保留为独立低分辨率证据。最终表格只采用真实 1440p 1:1 窗口。

## 六段健康证据

| 维度 | 当前证据 | 实机待确认项 |
| --- | --- | --- |
| 网络健康 | 原实机丢包/NACK 增量 0，RTT/jitter P95 8ms；本机最终回环丢包/NACK 增量 0 | 新构建原实机会话的连续网络窗 |
| 恢复链健康 | 原实机 SR 再试曾有 0.3s 冻结；本机最终回环冻结、解码丢帧增量 0。Browser 的 Native ReferenceChain / RecoveryLedger 为能力限定项 | 长测冻结、恢复及回退边界；Native 门禁按支持能力验收 |
| Media supply | 原实机多个 SR 窗口接收/解码接近 60、提交落后；最终隔离回环接收/解码/提交均约 58.45 | 满 60fps 接收时的同窗 candidate / submit |
| Steady supply | 最终 WebRTC 的候选与提交配平、rAF 约 60Hz；文件视频 RVFC 约 55–57Hz。WebRTC `mediaTime=0`，源率使用 getStats 累计 delta | 应用内 RVFC 合批、rAF 计数更新节拍与主线程回调滞后 |
| Submit / present 长尾 | 默认像素准备长尾已改为显式诊断；最终 SR CPU/上传 P95 1ms，绘制间隔 P95 18ms。WebKit GPU 查询 unavailable，物理 scanout unsupported | 新实机 CPU/GPU/合成与物理呈现；预热长尾和测量窗长尾分列 |
| 帧供给 delta | 最终两段 SR 各解码/提交 380 帧，限流、失败、重复、overwrite 累计 0；原实机持续额外缺口仍以旧矩阵成立 | 修改后同窗接收 → 解码 → video → candidate → submit 与自动回退效果 |

这些短窗提供定位证据。单组 60 秒前台覆盖、生产健康 gate 与整机呈现的验收状态继续为 `INSUFFICIENT_EVIDENCE / PENDING`。

## 自动验证与下一步

定向 171 项通过，全量 77 文件/711 项通过；`pnpm lint:fix`、`pnpm typecheck`、`pnpm build` 通过。新增回归验证：常规诊断经过 180 帧保持像素 context/准备/读取为 0，显式开启后采集像素，显式关闭后清理资源，182 个有效候选完整交付。

新构建已取得[SR / 锐化续查日志](2026-11-09-sr-sharpen-fresh-trace-analysis.md)：像素取证全量关闭，独立USM约55–56.7fps，video常规段合计59.945fps；SR两种界面锐度设置均执行同尺寸RCAS0.88。再试冻结前同窗解码122帧、提交102帧，增强段有效候选与提交均配平；另有1.236秒上游供给中断。下一步定位实际WKWebView的视频纹理导入、canvas合成与回调派发，并保留0.207/27.648Mbps码流负载差异的比较边界。
