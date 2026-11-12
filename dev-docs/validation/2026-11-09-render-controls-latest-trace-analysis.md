# 2026-11-09 15:03 新日志：增强持续缺口与自动回退

> 2026-11-12 整理：本页保留验证摘要；带 Git 链接的截图、夹具和中间输出按采样时提交读取。

## 结论

增强路径的持续帧率缺口再次出现。默认渲染在 79.321 秒完整呈现推进窗口中加权为 59.972fps；SR 初次的后段为 53.473fps，自动重试为 48.169fps；锐化“清晰”初次为 49.922fps，自动重试为 50.513fps。四次增强回退后，默认渲染各段均恢复到约 60fps。

同一 6.049 秒锐化窗口接收 362 帧、解码 363 帧、提交 304 帧，冻结增量为 0；同一 3.023 秒 SR 重试窗口接收 182 帧、解码 181 帧、提交 145 帧，冻结增量为 0。该样本支持播放过程中的连续提交缺口。有效候选与实际提交配平，限流、提交失败、重复绘制、overwrite 各代次均为 0，问题继续定位到浏览器视频推进到增强有效回调/候选的交付边界。

此轮运行身份为 **macOS / Browser / debug / dev trace**，场景负载跨段变化明显。结论覆盖该次运行的媒体供给和提交观测；掌机结果、完整超分放大性能、GPU 完成时间及物理上屏列为待证项。

## 输入身份与复算口径

- 原日志：[runtime-trace-1794207479617-1.jsonl](/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1794207479617-1.jsonl)。
- 不可变分析快照：`/tmp/xbxrc-2026-11-09-render-controls-latest-trace.jsonl`，SHA256 `ef1b9e76578eeca063e53036afdd759d928e7f602497a52fcc70cbf4bd14223f`。读取前后源文件大小与修改时间一致，分析结束复查仍为同一文件、5,688,907 字节。
- 时间范围：2026-11-09 14:57:59.617–15:03:14.731，Asia/Shanghai；2720 行，解析错误 0，全部 schema v3 / dev。实际 renderer 播放覆盖约 15:00:56.397–15:02:43.830。
- 构建 commit `20348f4c`，dirty；源码指纹 `0b33ec0386ae4f8373a09c648227f7e8b38628f94b3e62e3053436f3ea458fe5`。按该次构建身份归档，当前工作区另有并行编辑。
- 416 个原始 RTC stats、103 个 render telemetry、54 个资源事件、8 个 renderer 提交代次。source / output 均为 2560×1440，帧率上限 60。
- Canvas 采用同代次累计提交量与单调时钟计算。默认渲染采用完整 telemetry 发布窗口的 presented-frame 推进代理。接收、解码、网络采用完整位于代次边界内的原始 RTC delta。
- 同窗帧差使用匹配的 cached `sampleId`；live diagnostics 的候选/提交差另按其自身 `observedAtMs` 计算。三秒最小值要求相同 renderer identity、完整三秒边界及至少 2999ms renderer coverage。

数据见 [metrics](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-09-render-controls-latest-trace-analysis.metrics.json)，复算入口见 [分析脚本](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-09-render-controls-latest-trace-analysis.py)。原始日志按数据解析，报告保留数值、事件身份与构建证据。

## 实际渲染时间轴

| 代次 / 实际模式 | 时间 | 代次持续 | 接收 / 解码 fps | 提交或 video 推进 fps | 最低完整 3s fps | 实际平均码率 |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 1 / SR | 15:00:56.397–15:01:03.653 | 7.269s | 59.88 / 59.88 | 全段 41.42；后段 53.47 | 32.68 | 27.875Mbps |
| 2 / 默认，持续缺口回退 | 15:01:03.653–15:01:21.753 | 18.100s | 59.90 / 59.90 | 59.93 | 59.26 | 22.968Mbps |
| 3 / SR 自动重试 | 15:01:21.753–15:01:25.741 | 3.987s | 60.04 / 59.77 | 48.17 | 47.97 | 19.490Mbps |
| 4 / 默认，重试回退 | 15:01:25.741–15:02:01.819 | 36.080s | 59.94 / 59.94 | 60.02 | 59.50 | 5.526Mbps |
| 5 / 锐化清晰，USM / strength=3 | 15:02:01.819–15:02:08.916 | 7.094s | 59.91 / 59.91 | 49.92 | 45.99 | 0.081Mbps |
| 6 / 默认，持续缺口回退 | 15:02:08.916–15:02:27.007 | 18.090s | 59.94 / 59.94 | 59.92 | 59.50 | 5.520Mbps |
| 7 / 锐化清晰自动重试 | 15:02:27.007–15:02:31.006 | 4.001s | 59.76 / 59.76 | 50.51 | 50.27 | 24.903Mbps |
| 8 / 默认，重试回退 | 15:02:31.006–15:02:43.830 | 12.828s | 59.93 / 59.85 | 59.95 | 59.53 | 26.083Mbps |

Canvas 的计数窗口以实际可测计数点为边界。第 1 段后段从提交后 4.013 秒的计数点开始，覆盖 3.254 秒；其整体 41.42fps 含启动过渡。第 5 段跳过启动两秒后，完整累计点窗口仍为 51.38fps，覆盖 5.021 秒。持续缺口判断同时使用下面的同窗接收/解码证据。

四个默认段的完整推进窗口加权为 59.972fps，合计 79.321 秒。其回调频率分别为 51.56 / 54.17 / 53.83 / 52.61Hz，呈现推进分别为 59.93 / 60.02 / 59.92 / 59.95fps。回调稀疏和视频推进保持约 60fps 在默认路径中同时存在；增强路径的提交量跟随已交付有效候选。

此轮模式证据覆盖 SR、锐化清晰及回退默认，锐化柔和样本缺省。SR 的实际计划为 FSR1、`mode=passthrough`、1440p→1440p、`rcasStops=0.88`；该段覆盖同尺寸 RCAS 路径。跨放大倍率的 SR 验证另需对应分辨率样本。

## 同窗帧差与最早异常

| 代次 | 配对 sampleId | 同窗持续 | 接收 / 解码 / 提交 | 解码到提交差额 | 冻结增量 |
| --- | --- | ---: | ---: | ---: | ---: |
| 1 / SR 后段 | `1:15 → 1:27` | 3.032s | 182 / 182 / 164 | 18 帧 | 0 |
| 3 / SR 重试 | `1:103 → 1:115` | 3.023s | 182 / 181 / 145 | 36 帧 | 0 |
| 5 / 清晰 | `1:262 → 1:286` | 6.049s | 362 / 363 / 304 | 59 帧 | 0 |
| 7 / 清晰重试 | `1:362 → 1:374` | 3.023s | 182 / 181 / 152 | 29 帧 | 0 |

- `seq=504 / sampleId=1:15` 已观测到完整三秒内接收/解码约 59.99、video 推进 60.32、增强提交 32.68fps，随后 SR 后段仍存在上表中的 18 帧差额。
- `seq=563 → 564`：完整三秒 SR 提交 54.03、video 推进 60.03、解码 59.99fps，触发 `sustainedFrameDeliveryDeficit`，`seq=569` 提交默认 renderer；后续动作效果记为 `improved`。
- `seq=982 → 983`：SR 重试完整三秒提交 47.97、video 推进 60.18、解码 59.88fps，触发 `enhancementTrialFailed`，`seq=989` 回到默认。
- `seq=1933 → 1934`：清晰已连续积累 3012ms 供帧不足证据，触发持续缺口回退，`seq=1939` 提交默认；该 sampleId 的三秒窗口提交 54.62、video 推进 61.31、解码 61.31fps。
- `seq=2374 → 2375`：清晰重试完整三秒提交 50.27、video 推进 59.88、解码 59.88fps，触发重试失败并回退。

首次 SR 和清晰的回退各约发生在增强提交 7 秒后。默认渲染恢复健康约 18 秒后，策略各自动重试一次，4 秒后再次回退。重试由 `healthyEnhancementTrial` 发起；动作来源为 runtime 策略。增强段自动结束的实际边界为 4–7 秒，当前样本支持这些窗口内的持续缺口及回退效果，长时增强稳定性继续列为待验收。

## 候选、提交与绘制长尾

| 增强代次 | 同时钟 live 诊断覆盖 | RVFC 有效候选增量 | 实际提交增量 | rAF 有效补供给 |
| --- | ---: | ---: | ---: | ---: |
| 1 / SR 后段 | 3.012s | 161 | 161 | 0 |
| 3 / SR 重试 | 3.012s | 144 | 144 | 0 |
| 5 / 清晰 | 7.028s | 349 | 349 | 0 |
| 7 / 清晰重试 | 3.012s | 152 | 152 | 0 |

累计候选与提交配平断言全部通过，四代次限流、提交失败、重复绘制及 overwrite 为 0。103 份像素诊断均为 `enabled=false`，像素探测资源及读回计数全为 0。

各增强代次最终保留的 CPU 绘制/上传 P95 为 1ms，draw interval P95 为 33–34ms；全生命周期最差 draw interval 分别为 143 / 100 / 117 / 67ms。GPU timer query 各代次均 `unavailable`、query count=0，物理 scanout 为 `UNSUPPORTED`。CPU 时间、提交间隔和 GPU 完成时间采用各自证据语义。

最高置信度的定位为：浏览器呈现推进保持约 60fps，已交付候选/回调稀疏，增强逐候选即时提交。WKWebView 的视频纹理导入、canvas 合成与回调派发是后续核查对象；GPU 成本及具体因果边界仍需异步完成证据。

## 六段健康与冻结

| 维度 | 结论 |
| --- | --- |
| 网络健康 | 完整播放 RTC 窗新增丢包 43、NACK 38、收到重传包 44；接收 162492 包，RTT / jitter P95 29 / 8ms。增强同窗接收保持约 60fps。 |
| 恢复链健康 | 累计一次 0.180s 冻结，peer generation 保持 1；播放窗新增 PLI/FIR 均为 0。Browser Native ReferenceChain / RecoveryLedger 为 `UNSUPPORTED`。 |
| Media supply | 106.962s 完整原始 RTC 窗接收 59.937、解码 59.928fps，平均解码 CPU 4.147ms；完整采样窗全部有解码增量。 |
| Steady supply | 同窗解码约 60fps，SR/清晰提交持续约 48–54fps；四次回退后的默认推进恢复约 60fps。 |
| Submit / present 长尾 | 增强绘制间隔 P95 33–34ms，存在 67–143ms 长尾；GPU 完成时间与物理上屏保持能力限定。 |
| 帧供给 delta | 四增强代次同窗解码到提交差额为 18 / 36 / 59 / 29 帧；有效候选与提交配平，rAF 补供给 0，renderer drop counters 全 0。 |

短冻结锚点为 `seq=733`、15:01:12.599，位于默认代次 2。相邻 RTC 窗接收/解码先为 11/11，随后 19/17，再为 8/10 和 21/20；这些窗口丢包、NACK、PLI 增量均为 0，停顿后有供给追赶。约 250ms 采样支持这段供给波动与短冻结的关联，具体停顿环节继续列为待证项。增强配对窗口的冻结增量全为 0，其持续帧差另有独立证据。

NACK 具备发送计数和收到重传包证据；逐请求 recovered/late/skipped/expired disposition、有效率、ReferenceChain 建链成功率、repairability 持久化及 recovery effectiveness 评分在 Browser 样本中为能力限定项。

## 样本有效性、菜单与结束清理

Browser 行为合同和 runtime stability 门禁均 `PASS`，验证观测配对、动作资格与资源闭环。帧率结论由累计量和同窗供给证据分别给出。

生产健康门禁为 `INSUFFICIENT_EVIDENCE`：文件采用 dev profile，production health records / manifest 为缺省；生产消费投影有效媒体覆盖约 27.725 秒。原始 RTC 窗覆盖 106.962 秒，上方性能复算使用原始证据。producer / backend 各层丢失、IO failure 与 `traceBudgetNotice` 全为 0。

菜单样本有 12 个 mouse 来源交互，`seq=1271` 的诊断开关和 `seq=1662` 的显示面板操作完成，input health failure count=0。触屏和手柄导航操作来源样本缺省。`seq=1271` 开启详细诊断，清晰初段的部分采样期间 display 弹层仍为 top scope；这一条件和各段码率差异共同构成跨模式性能比较的边界。

`seq=2655`、15:02:43.830，clientClosed 后 renderer 资源清零。`seq=2669` 的退出清理结果为 completed，inputReleased / runtimeStopped / remoteClosed / startupCancelled 全为 true，耗时 358ms；`seq=2670` 确认 flush 至 2669。`seq=2682 → 2683` 继续确认页面卸载清理及对应 flush。生产 ending gate 因终态 summary 证据缺省保持 `INSUFFICIENT_EVIDENCE`，实际资源释放与后续写入已有独立证据。

## 后续定位

优先检查共享视频帧源的 RVFC 派发与帧身份观测、视频纹理上传以及 WKWebView canvas 合成。保留默认回退作为当前稳定播放路径，并单独评估增强失败后自动重试引入的再次低帧窗口。后续增强长测固定场景运动、关闭采样期间的详细诊断和设置弹层；生产日志用于完整摘要验收，GPU 异步完成与窗口活动状态用于继续拆分因果。

复算命令在仓库根目录运行，输入 SHA 必须匹配上述快照：

```sh
python3 -B .agents/skills/analyze-runtime-logs/scripts/trace_browser_webrtc_behavior_report.py /tmp/xbxrc-2026-11-09-render-controls-latest-trace.jsonl --require-stability > /tmp/xbxrc-2026-11-09-render-controls-latest-browser.json
python3 -B .agents/skills/analyze-runtime-logs/scripts/trace_production_health_report.py /tmp/xbxrc-2026-11-09-render-controls-latest-trace.jsonl > /tmp/xbxrc-2026-11-09-render-controls-latest-production.json
python3 -B dev-docs/validation/2026-11-09-render-controls-latest-trace-analysis.py /tmp/xbxrc-2026-11-09-render-controls-latest-trace.jsonl --expected-sha ef1b9e76578eeca063e53036afdd759d928e7f602497a52fcc70cbf4bd14223f --browser-audit /tmp/xbxrc-2026-11-09-render-controls-latest-browser.json --production-audit /tmp/xbxrc-2026-11-09-render-controls-latest-production.json --output /tmp/xbxrc-2026-11-09-render-controls-latest-replay.metrics.json
```

本轮交付为日志分析、可复算指标和分析脚本，应用实现保持本轮分析前的工作区状态。
