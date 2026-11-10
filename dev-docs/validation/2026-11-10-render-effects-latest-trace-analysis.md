# 2026-11-10 原画、SR、锐化最新日志帧率分析

四个主要配置段的 canvas 提交帧率为 51.60–53.07fps，完整 3 秒最低窗口为 44.92–46.70fps；对应接收、解码与浏览器视频呈现推进接近 60fps。原画段同样存在约 7fps 的持续缺口，共用帧源交付与 WebGL/canvas 路径是优先定位范围。

本次 SR 设置对应 2560×1440 → 2560×1440，`sr.mode=passthrough`、`actualSuperResolutionActive=false`。样本覆盖同尺寸 SR 设置下的直通/锐化行为；FSR 重建性能需要源尺寸小于输出尺寸的独立样本。

## 输入与计算口径

- 原日志：`/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1794275474604-1.jsonl`。
- 分析快照：`/tmp/xbxrc-2026-11-10-render-effects-trace.jsonl`，SHA256 `4f2939390f10309d080d7ec9e49027463abc5f08e19dae0b212827c8ac52e45f`。
- 覆盖 2026-11-10 09:51:14.604–09:57:53.133，Asia/Shanghai；4388 行、10,530,369 字节，完整 JSONL 行解析成功。
- 实际播放约 184.76 秒；733 个原始 RTC 样本、184 个 render telemetry、1 个 renderer generation、7 个实际配置段，全部 schema v3 / dev。构建为 Browser / debug，commit `76a1ea69`、dirty，构建源码指纹 `10c0e6a97f0dd6aff486f1978344c7a298260d05a4b19b34ddbe9ac0e4030bf4`。
- 分段采用 `rendererCommitted` / `rendererUpdated` 资源事件的 `actual*` 字段，配置变化保持同一 renderer generation。canvas FPS 使用同代次累计 `realtimeDrawCount` / 提交数和单调时钟；接收、解码及码率使用完整位于段内的原始 RTC delta 窗口。
- 最低 3 秒值采用完整位于对应实际配置段、renderer coverage 至少 2999ms 的窗口。同 sampleId 的接收/解码/提交比较另外配对；跨配置的边界采样按各自观测时钟归属。
- 最后原画段提交计数的有效覆盖为 40.289 秒，关闭前尾部 212ms 留为计数覆盖边界。下表时长使用实际配置持续时间。

详见 [数值与证据](2026-11-10-render-effects-latest-trace-analysis.metrics.json) 和 [复算脚本](2026-11-10-render-effects-latest-trace-analysis.py)。

## 七个实际配置段

源/输出均为 2560×1440，目标帧率 60；原画与增强配置均使用 WebGL canvas。锐化档位 3 对应 USM。

| 段 | 时段 | 实际设置 | 时长 | 接收 / 解码 fps | canvas 提交 fps | 最低完整 3s fps | 平均码率 |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 09:54:03.285–09:55:05.779 | SR 同尺寸 + USM 锐化 3 | 62.494s | 60.00 / 59.99 | **52.65** | **45.27** | 17.81Mbps |
| 2 | 09:55:05.779–09:55:07.764 | SR 同尺寸，锐度 0 | 1.985s | 59.95 / 59.95 | 51.39 | 短段 | 13.87Mbps |
| 3 | 09:55:07.764–09:55:09.145 | SR 同尺寸 + USM 锐化 3 | 1.381s | 60.13 / 60.13 | 46.34 | 短段 | 13.91Mbps |
| 4 | 09:55:09.145–09:55:44.770 | SR 同尺寸，锐度 0 | 35.625s | 59.94 / 59.94 | **52.86** | **46.22** | 21.80Mbps |
| 5 | 09:55:44.770–09:55:45.786 | 原画，锐度 0 | 1.016s | 60.61 / 60.61 | 58.07 | 短段 | 21.01Mbps |
| 6 | 09:55:45.786–09:56:27.547 | USM 锐化 3，SR 关 | 41.761s | 59.93 / 59.93 | **51.60** | **44.92** | 20.10Mbps |
| 7 | 09:56:27.547–09:57:08.048 | 原画，锐度 0 | 40.501s | 59.96 / 59.96 | **53.07** | **46.70** | 20.86Mbps |

原画主段相对独立锐化主段高 1.46fps；四个主段相对 60fps 的平均缺口约 11.6%–14.0%。第 2、3、5 段属于快速切换窗口，持续性能比较采用第 1、4、6、7 段。各主段平均码率为 17.81–21.80Mbps，画面负载随时段变化；这份样本支持共同缺口的定位，效果开关的精确性能代价继续以同画面配对实验衡量。

## 帧供给缺口与长尾

全部 render telemetry 的 `trackingSource=videoFrameCallback`，源帧率估计返回 `mediaTimeDeltaTooSmall`，RVFC 证据中的 mediaTime 持续为 0。原始 RTC 累计帧数及视频呈现计数提供约 60fps 的源供给证据。

| 主段 | 完整 telemetry 覆盖 | 视频呈现推进 fps | RVFC 回调 fps | callback gap 次数 | presented jump 次数 | 最大回调间隔 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 / SR + 锐化 | 62.248s | 59.95 | 52.69 | 3 | 445 | 120ms |
| 4 / SR 同尺寸 | 34.136s | 59.94 | 52.79 | 1 | 241 | 133ms |
| 6 / 锐化 | 41.164s | 59.96 | 51.55 | 1 | 343 | 216ms |
| 7 / 原画 | 39.157s | 59.94 | 52.99 | 0 | 270 | 67ms |

回调间隔跨阈值共 5 次，呈现计数多帧推进达上千次。`droppedFramesSinceLastSample` 属于回调/呈现推进的 dropped-like 并集；浏览器解码 `framesDropped` 全程为 0。持续差额集中在视频呈现计数到回调候选的交付边界。

有效候选与提交量在各可测配置段逐帧配平，主段分别为 3280/3280、1802/1802、2122/2122、2075/2075。同 sampleId 的解码到提交差额分别为 456、246、342、273 帧；其中原画主段的配对窗口为 39.088 秒、解码 2343 帧、提交 2070 帧、冻结增量 0。renderer 限流、提交失败、重复绘制和 overwrite 累计均为 0。

最终帧源诊断：11020 次 rAF 观测，额外有效候选为 0；9646 次 RVFC 观测全部接受。播放质量计数最大一次推进 31 帧，计数更新周期最大 316ms。RVFC 入口 clock lag 最大 1ms，支持回调进入 JS 后迅速承接的判断。`renderFrameSource=presentation`，`renderFrameFallbackReason=initializationTimeout`，decodedFrame reader/候选为 0；本次运行使用呈现回调兼容帧源。

各主段末尾保留的 256 帧 timing 中，同步 draw/upload CPU P95 均为 1ms，提交 CPU P95 为 0–1ms，绘制帧间隔 P95 为 33–34ms；全代次最大绘制间隔为 218ms。主段采样的最近提交年龄 P95 为 32、29、31、16ms，锐化冻结窗最大 177ms。GPU query 为 `unavailable`，物理 scanout 为 `UNSUPPORTED`，GPU 执行及实际显示延迟属于待补证据。

最差完整 3 秒窗口锚点：第 1 段 `seq=842`、第 4 段 `seq=2141`、第 6 段 `seq=2744`、第 7 段 `seq=3561`。原画 `seq=3561` 同窗接收 59.70、解码/视频呈现推进 60.03、回调/提交 46.70fps，直接支持上游正常供给与 canvas 交付不足同时成立。

## 网络、恢复与六项健康

| 维度 | 本次证据 |
| --- | --- |
| 网络健康 | 完整播放 raw delta 覆盖 184.569s，新增丢包 176、NACK 112、收到重传包 267；RTT P95 26ms、jitter P95 8ms，平均码率 19.69Mbps。启动前已累计丢包 172、NACK 1，最终累计分别为 348、113 |
| 恢复链健康 | 09:56:06.081 / `seq=3027` 记录一次 227ms 冻结；其后 252ms 窗解码 30 帧并恢复推进。完整播放窗新增 PLI/FIR 为 0，解码关键帧 9 个，peer generation 保持 1。Browser Native ReferenceChain / cleanAnchorCommitted / DisplayStable 为能力限定项 |
| Media supply | 全窗接收/解码均 59.961fps，平均单帧解码耗时 4.377ms。各主要配置段完整 raw 窗接收/解码均约 60fps |
| Steady supply | 四个主段 canvas 51.60–53.07fps；最差完整 3 秒 44.92–46.70fps，同时视频呈现代理继续接近 60fps |
| Submit / present 长尾 | CPU P95 1ms，绘制间隔 P95 33–34ms；冻结期间最大回调/绘制间隔 216/218ms。GPU 完成时间与物理上屏保留待证状态 |
| 帧供给 delta | 有效 RVFC 候选与提交量配平，rAF 额外供给 0；原画配对窗仍少提交 273 帧，缺口集中于共用呈现回调交付链 |

冻结窗的 raw delta 同时记录 252ms 内接收 7 帧、解码 3 帧、组帧累计 269.177ms、jitter-buffer 累计 293.669ms；下一窗接收 27、解码 30 帧。该窗包接收继续推进，冻结定位为媒体组帧/缓冲到解码供给的短时停顿。具体远端发送间隔与接收队列原因仍需更细证据。持续低帧率在冻结前后的正常媒体窗口均成立。

## 证据完整性与下一步

逐帧候选/提交配平、代次累计数单调性及 raw sampleId 唯一性断言通过。producer/writer 各层丢失累计为 0，`traceBudgetNotice=0`。像素诊断 184 份均 `enabled=false`、read count 0。

Browser 行为脚本在 `--require-stability` 下返回 `FAIL`，子状态为 `MISSING`，缺项为 `rendererEffectEvents`。日志保留 13 个 `sustainedFrameDeliveryDeficit` 计划事件，效果评估事件计数为 0；这项门禁反映动作效果证据缺失，帧率结论采用上述完整计数窗口。生产合同门禁为 `INSUFFICIENT_EVIDENCE`，当前文件为 dev profile，生产终态记录覆盖为 partial。

优先定位共用 WKWebView 呈现回调、video 纹理导入与 canvas 合成链；增加低频 GPU 完成观测，并核验 decoded-frame reader 在目标 WebView 的初始化结果。采用同一持续运动画面、相同码率的原画/SR/锐化各 60 秒配对样本；真实 SR 验收采用源尺寸小于目标尺寸的窗口。

复算命令：

```sh
python3 -B dev-docs/validation/2026-11-10-render-effects-latest-trace-analysis.py /tmp/xbxrc-2026-11-10-render-effects-trace.jsonl --output /tmp/xbxrc-2026-11-10-render-effects-replay.metrics.json
```
