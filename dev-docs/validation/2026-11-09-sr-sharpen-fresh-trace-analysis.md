# 2026-11-09 SR / 锐化新日志：持续缺口与上游供给中断

> 2026-11-12 整理：本页保留验证摘要；带 Git 链接的截图、夹具和中间输出按采样时提交读取。

## 结论

增强渲染的持续缺口仍然成立。独立 USM 的晚段可测窗口为 55.03fps，再试整段为 56.67fps；SR 主段为 58.32fps，末尾完整 3 秒降到 53.11fps，再试整段为 48.17fps。video 常规段合计推进 59.945fps，另有一次接收供给中断造成明显冻结。

本次默认像素诊断开关的效果已得到日志确认：140 份媒体诊断全部 `videoPixels.enabled=false`，准备、读取、探测资源均为 0。增强段的有效候选与实际提交逐帧配平。剩余缺口位于浏览器视频呈现代理到有效回调/候选的交付边界，WKWebView 视频纹理导入、canvas 合成与回调派发是下一步定位重点；GPU 执行耗时与物理上屏继续为待证项。

SR 主段平均码率为 0.207Mbps，再试段为 27.648Mbps。码流负载差异构成跨段比较的实际边界，58.32fps 相对旧样本的升幅保留为观测结果，修复收益的因果归因仍需控制画面负载。

## 输入身份与计算口径

- 原文件：`/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1794190273035-1.jsonl`。
- 不可变分析快照：`/tmp/xbxrc-2026-11-09-sr-fresh-trace.jsonl`；SHA256 `e3c88d94856c299dbbc39a14221000f04354b7864fd7ac3f3a242a16232aa1d7`。
- 覆盖 2026-11-09 10:11:13.035–10:17:17.560，Asia/Shanghai；3701 行、8,063,227 字节、解析错误 0，全部 schema v3 / dev。
- 实际构建为 macOS / Browser / debug，commit `20348f4c`、dirty；构建源码指纹 `e387a1d166b326431519dc3f9e4a93784744eb004ccf85dc1e37aec5e4f38ab2`。本报告按该次运行身份归档；复算时工作区正在推进认证与导航修改，当前源码指纹另存于 metrics。
- 596 个原始 RTC stats、140 个 render telemetry、90 个资源事件、8 个 renderer 提交代次。接收与解码采用完整原始 delta 窗口；canvas FPS 采用同代次累计提交量及单调时钟；video FPS 采用完整代次内的呈现计数推进代理。
- 所有原始日志内容按数据解析；分析输出保留数值、固定事件字段及构建身份。

完整计数、窗口覆盖与校验条件见 [metrics](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-09-sr-sharpen-fresh-trace-analysis.metrics.json)，复算入口见 [分析脚本](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-09-sr-sharpen-fresh-trace-analysis.py)。

## 实际路径矩阵

源与输出均为 2560×1440，用户帧率上限为 60。video 列表示浏览器呈现代理；增强列表示 canvas 实际提交。最低 3 秒值只取完整位于当前代次、renderer coverage 至少 2999ms 的原始窗口。

| 代次 / 实际路径 | 时段 | 持续时间 | 接收 / 解码 fps | 提交或 video 推进 fps | 最低完整 3s fps | 实际平均码率 |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 1 / USM，锐度 3 | 10:14:11.628–10:14:55.705 | 44.074s | 59.93 / 59.95 | 全段 32.61；晚段窗口 55.03 | 52.34 | 18.441Mbps |
| 2 / video，增强回退 | 10:14:55.705–10:15:13.778 | 18.075s | 59.94 / 59.94 | 59.94 | 59.46 | 18.123Mbps |
| 3 / USM，锐度 3，再试 | 10:15:13.778–10:15:28.832 | 15.053s | 59.92 / 59.92 | 56.67 | 52.88 | 11.294Mbps |
| 4 / video，增强回退 | 10:15:28.832–10:16:03.341 | 34.511s | 60.00 / 60.00 | 59.94 | 59.12 | 12.961Mbps |
| 5 / SR，锐度 3→0 | 10:16:03.341–10:16:26.071 | 22.721s | 59.94 / 59.94 | 58.32 | 53.11 | 0.207Mbps |
| 6 / video，含接收中断 | 10:16:26.071–10:16:44.175 | 18.101s | 55.33 / 55.28 | 56.13 | 38.19 | 17.289Mbps |
| 7 / SR，锐度 0，再试 | 10:16:44.175–10:16:48.159 | 3.986s | 59.79 / 59.79 | 48.17 | 48.27 | 27.648Mbps |
| 8 / video，增强回退 | 10:16:48.159–10:17:10.218 | 22.062s | 59.93 / 59.97 | 59.96 | 57.86 | 17.551Mbps |

第 1 段详细诊断从提交后约 38.045 秒开始，原始采样间隔同时从约 1000ms 转为 252ms。其全段累计提交 32.61fps 是有效全段事实，前台可见性与原生窗口遮挡状态缺证；55.03fps 对应首个晚段可用累计点开始的 6.142 秒窗口。源码中的 StatsService 周期固定为 250ms，日志中采样节拍变化的具体触发因素继续保留为待证项。

video 第 2、4、8 段的完整推进窗口共 71.282 秒，时间加权平均 59.945fps。第 4 段包含一次 0.260 秒冻结，短时不均匀供给继续按单独冻结窗口解释。

## SR 两种界面锐度设置

`seq=2352`、10:16:10.923、`observedAtMs=297434` 将界面锐度从 3 改为 0，renderer 保持第 5 代。两种设置实际都执行 `1440p → 1440p`、`mode=passthrough`、`rcasStops=0.88`、`processing=none`；该份样本覆盖同尺寸 RCAS 路径。

| 界面设置 | 可测累计提交窗口 | 提交计数 delta | canvas fps | 同 sampleId 配对：解码 / 提交 |
| --- | ---: | ---: | ---: | ---: |
| SR 开、锐度 3 | 7.422s | 439 | 59.15 | 408 / 403 帧，6.808s |
| SR 开、锐度 0 | 15.066s | 872 | 57.88 | 846 / 820 帧，14.117s |

切换前最近计数位于 `297276ms`，切换后首个计数位于 `297508ms`，两侧保留 232ms 的边界缺口。`seq=2357` 同时携带切换前缓存与切换后实时诊断，分段采用各自的观测时钟。两段底层计划相同，帧率差额主要反映后续时段的供给变化。

第 5 段末尾 `seq=2697` 的完整 3 秒窗口中，接收 60.03、解码 60.07、video 推进 60.11、有效回调 53.11、canvas 提交 53.11fps。该窗口同时证明稳定上游供给与持续 canvas 缺口。

## 缺口发生在哪一环

139.558 秒诊断覆盖内，7426 个 RVFC 观测全部被接受；8367 个 rAF 观测提供的额外有效候选为 0。播放质量计数共更新 532 次，其中 RVFC 入口 528 次、rAF 入口 4 次，合批最大推进 25 帧。rAF 的 4 次计数变化全部抵扣已由 RVFC 交付的额度，rAF 补供给保持 0。

| 增强代次 | 同时钟诊断覆盖 | 有效 RVFC 候选 | 实际提交 | rAF 有效补供给 |
| --- | ---: | ---: | ---: | ---: |
| 1 / USM 晚段 | 6.025s | 335 | 335 | 0 |
| 3 / USM | 14.058s | 796 | 796 | 0 |
| 5 / SR | 22.093s | 1288 | 1288 | 0 |
| 7 / SR | 3.014s | 147 | 147 | 0 |

所有增强代次的限流、提交失败、重复绘制与 overwrite 累计均为 0。第 7 段冻结前，`sampleId=1:507 → 1:515`、`331555–333573ms` 的同一 2.018 秒窗口接收 121、解码 122、提交 102 帧，对应接收 59.96、解码 60.46、canvas 50.55fps，冻结增量为 0。增强缺口在后续 0.185 秒冻结之前已经出现。

记录到的 RVFC 入口 clock lag 最大 1ms，rAF 间隔最大 87ms；有效回调的长期稀疏与浏览器合批推进继续成立。共享帧源资格来自 RVFC / 播放计数，已交付候选由 renderer 即时承接；当前优化方向沿这条链继续定位回调派发、视频纹理导入与合成成本。

## 接收中断与恢复

从首个 renderer 提交到退出操作，完整原始 RTC delta 覆盖 177.439 秒，接收 59.525、解码 59.530fps，平均解码成本 4.274ms；窗内丢包增量 126、NACK 增量 95、收到重传包 353，RTT P95 45ms、jitter P95 11ms。最终累计丢包 232、NACK 96；较早启动边界的 106 个丢包与 1 次 NACK 位于完整播放窗之外，两种计数口径各有边界。

| 冻结观测锚点 | 代次 | 冻结增量 | 已确认的关联证据 |
| --- | --- | ---: | --- |
| `seq=557` / 10:14:42.555 | 1 / USM | 0.304s | 原始 1Hz 窗；当前窗接收/解码各 60，内部停顿与追赶被合并 |
| `seq=1449` / 10:15:30.663 | 4 / video | 0.260s | 253ms 内接收/解码各 5、丢包/NACK 各增 1 |
| `seq=3022` / 10:16:41.018 | 6 / video | 1.236s | 视频包接收中断，随后 PLI 与关键帧解码接续 |
| `seq=3176` / 10:16:48.075 | 7 / SR | 0.185s | 相邻 252ms 窗接收 7→23、解码 8→22，停顿后追赶 |

第 6 段 `seq=3003/3009/3013` 的连续三个窗口共 757ms，视频包、接收帧、解码帧增量均为 0，getStats 采集耗时仍为 1–2ms。`seq=3019` 收到 995 包，丢包增量 21、jitter 70ms、PLI 增 1，接收/解码帧仍为 0；`seq=3022` 再增 1 次 PLI，解码 1 个关键帧及 14 帧，随后帧供给恢复。完整 3 秒最低窗 `seq=3045` 的接收/解码为 35.86/34.86fps，video 推进为 38.19fps，接收链路/上游供给中断的证据完整。

本次共 4 次冻结、累计 1.985 秒，peer generation 保持 1。Browser 原始 stats 支持上述接收、反馈、解码与恢复观测；Native ReferenceChain / RecoveryLedger 的验证状态保持 `UNSUPPORTED`。

## 六段健康、长尾与门禁

| 维度 | 本次结论 |
| --- | --- |
| 网络健康 | 新增丢包、NACK 与 757ms 视频包接收空窗，RTT / jitter P95 45 / 11ms；码流负载按各代次单列 |
| 恢复链健康 | 4 次冻结共 1.985s；1.236s 中断后 PLI、关键帧解码和持续供给接续；Native 恢复合同为能力限定项 |
| Media supply | 大部分原始窗口接收/解码约 60；第 6 段降到 55.33/55.28fps，异常完整 3 秒窗更低 |
| Steady supply | SR 第 5 段末尾原始接收/解码/video 约 60，有效候选与提交 53.11fps；第 7 段冻结前已存在 20 帧同窗差额 |
| Submit / present 长尾 | 增强上传/CPU 保留 P95 1ms；晚段绘制间隔 P95：USM 再试 34ms、SR 主段 33ms、SR 再试 34ms，后者最大 183ms。GPU query unavailable，物理 scanout unsupported |
| 帧供给 delta | 各增强代次有效候选与提交配平；RVFC 合批与 rAF 补供给 0 构成浏览器交付边界，限流/失败/重复/overwrite 均为 0 |

Browser 行为与严格稳定性门禁均 `PASS`，验证事件合同、观测配对、动作及资源闭环。帧率验收采用上方累计量与完整窗口，持续缺口状态保持待修复。

生产健康门禁为 `INSUFFICIENT_EVIDENCE`：该份文件采用 dev profile，生产 health records / manifest 为缺省；消费投影覆盖约 41.9 秒，完整原始 RTC delta 提供更长的性能复算。producer / writer 各层丢失计数均为 0，`traceBudgetNotice=0`。

`seq=3662`、10:17:10.240，clientClosed 后媒体资源清零；当前捕获结尾为 partial，生产终态 flush 验收继续为 `INSUFFICIENT_EVIDENCE`。菜单鼠标操作有 toggle completed 证据，退出动作在关闭 owner 后记为 cancelled；触屏与手柄操作的结论以对应输入来源样本验收。

## 优化方向与复算

优先定位实际 WKWebView 视频纹理导入与 canvas 合成对回调派发的影响，并按实际码率/画面负载分组验证。保留共享帧源、帧身份去重与最新候选即时提交，默认像素诊断继续关闭。当前可见的具体证据缺口为 GPU 完成时间及首段原生窗口遮挡/活动状态；下一轮取证采用有界、低频 GPU fence 完成观测与窗口状态，分别计算 upload/submit CPU、异步完成长尾及回调供给，避免将异步 GPU 工作只按同步 CPU 时间归因。

本轮现有样本已经支持持续缺口与上游中断的定位。稳定播放配置推荐 SR 关闭、锐度 0，异常上游供给按独立网络窗口处理。

仓库根目录复算命令如下，输入必须匹配上述不可变快照 SHA。分析脚本自带候选/提交配平、像素全量关闭、同 sampleId 时间配对、单调计数及提交代次断言。

```sh
python3 -B .agents/skills/analyze-runtime-logs/scripts/trace_browser_webrtc_behavior_report.py /tmp/xbxrc-2026-11-09-sr-fresh-trace.jsonl --require-stability > /tmp/xbxrc-2026-11-09-sr-browser-replay.json
python3 -B .agents/skills/analyze-runtime-logs/scripts/trace_production_health_report.py /tmp/xbxrc-2026-11-09-sr-fresh-trace.jsonl > /tmp/xbxrc-2026-11-09-sr-production-replay.json
python3 -B dev-docs/validation/2026-11-09-sr-sharpen-fresh-trace-analysis.py /tmp/xbxrc-2026-11-09-sr-fresh-trace.jsonl --browser-audit /tmp/xbxrc-2026-11-09-sr-browser-replay.json --production-audit /tmp/xbxrc-2026-11-09-sr-production-replay.json --output /tmp/xbxrc-2026-11-09-sr-fresh-replay.metrics.json
```

本轮变更为日志分析、复算脚本与验证文档。先前像素开关修改的定向 171 项、全量 77 文件/711 项及 lint/typecheck/build 结果接续 [上一轮报告](2026-11-09-sr-frame-supply-investigation.md)。
