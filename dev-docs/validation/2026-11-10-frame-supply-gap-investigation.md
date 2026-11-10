# 2026-11-10 帧供给缺口：reader 首帧超时早于连接建立

## 排查结论

本次持续缺口存在清晰的两段因果链：逐解码帧 reader 在媒体可用之前触发首帧超时，释放 reader 与 clone、固定回退为 `presentation`；后续原画、SR、锐化都复用该回退帧源，RVFC 合批交付约 52 个有效候选/秒，renderer 按已交付候选逐帧提交。

reader 的启动时序是当前优先修复点。修复后的真实 60fps 供给及画面新鲜度需要新日志验收；实际 GPU/上屏长尾继续保留能力边界。

## 输入身份

- 日志与分段性能：[11/10 最新日志分析](2026-11-10-render-effects-latest-trace-analysis.md)。原日志 `runtime-trace-1794275474604-1.jsonl`，快照 SHA256 `4f2939390f10309d080d7ec9e49027463abc5f08e19dae0b212827c8ac52e45f`。
- 日志构建源码指纹与本次排查工作区指纹均为 `10c0e6a97f0dd6aff486f1978344c7a298260d05a4b19b34ddbe9ac0e4030bf4`；当前源文件与运行证据的身份匹配。
- [时序及复现指标](2026-11-10-frame-supply-gap-investigation.metrics.json)、[加载修复前 reader 的复现脚本](2026-11-10-frame-supply-gap-investigation.repro.mjs)。排查时直接加载当时源码；修复后复算固定使用日志构建 commit `76a1ea69` 的 reader，并校验原 SHA，保留历史缺陷证据。

## 生命周期缺陷

`BaseCanvasVideoProcessor.initialize()` 在 setup 完成后立即调用 `scheduleFrameRequest()`；共享帧源 `requestRender()` 此时创建 `DecodedVideoFrameReader`。视频轨在 remote answer 阶段已经为 live，实际 ICE 连接及解码供给仍在随后建立。

`decoded-video-frame-reader.ts:37` 从构造时开始固定 2000ms 计时。清除计时器的成功入口为 `accept(frame)`；主线程 processor 可成功构造，Worker 也可发送 `ready`，首个 frame 到达前计时器持续运行。`onWorkerMessage()` 对 `ready` 保持缺省处理。

| 单调时钟 | 锚点 | 事实 |
| ---: | --- | --- |
| 160673ms | `seq=345` | 收到视频轨，随后创建 renderer |
| 160694–160697ms | `seq=391/396` 的 attach 生命周期 | setup/attach 完成；依据实际调度代码推定 reader 在此区间启动 |
| 162694–162697ms | 源码 2000ms 截止，推定值 | reader 首帧等待截止；该时刻早于 ICE checking 与 connected |
| 167157ms | `seq=376` | ICE 开始 checking |
| 167454ms | `seq=385` | transport connected，距 reader 启动约 6.76s |
| 168177ms | `seq=391` | 首帧之前，资源已报告 `presentation / initializationTimeout`，decoded 接收计数 0 |
| 168185ms | `seq=393` | 首次解码观测，距 reader 启动约 7.49s |
| 168186ms | `seq=395` | 首次 RVFC 呈现观测 |
| 168193ms | `seq=396` | renderer 首帧提交并 commit，继续使用 presentation 帧源 |

具体超时触发时刻由源码与 attach 时钟推定；日志直接证明首帧前已完成超时回退。资源中的 decoded 接收计数全程为 0，renderer generation 全程为 1。

超时经 `fail()` → `dispose()` 设置 `stopped=true`，取消 reader、停止 clone、关闭 Worker。`VideoFrameSource.requestRender()` 采用 `decodedReader ??=`，回退对象继续留在共享帧源内；持续订阅与同 context 效果更新复用该对象。媒体开始正常推进后，该 reader 仍保持已停止状态。

## 直接源码复现

复现脚本读取并转译日志构建 commit `76a1ea69` 的 `DecodedVideoFrameReader`，只模拟时钟、track、processor/Worker 和输入 frame；Worker URL 使用实际源文件路径。

| 场景 | 结果 |
| --- | --- |
| reader 启动，2s 内媒体尚未生产帧，7.5s 才交付首帧 | 2s 触发 `initializationTimeout`；clone stop 与 reader cancel 各一次；迟到 frame 关闭，consume/received 均为 0 |
| 500ms 内交付首帧，随后时间推进至 7.5s | 模式为 main-thread，首帧接受并消费，初始化计时器成功清除 |
| Worker 先完成 ready，首个媒体 frame 晚于 2s | 继续触发 `initializationTimeout`；Worker 收到 start、stop |

三个断言场景通过，确认当前代码的超时与永久停止机制。

```sh
node dev-docs/validation/2026-11-10-frame-supply-gap-investigation.repro.mjs
```

## 回退后的持续缺口

完整播放窗接收与解码均为 59.961fps。四个主段视频呈现计数约 59.94–59.96fps，有效回调约 51.55–52.99fps，canvas 提交约 51.60–53.07fps。原画配对窗 39.088 秒解码 2343 帧、提交 2070 帧，冻结增量为 0。

最终数字诊断中，9646 个 RVFC 观测全部接受；11020 个 rAF 观测的额外有效候选为 0。播放质量计数最多一次推进 31 帧、更新间隔最大 316ms。呈现计数的批量推进记录历史累计量，当前 video 纹理只提供当时可读取的画面；一次 RVFC 对应一个最新候选。主段已交付候选与实际提交逐帧配平，缺口集中于原始呈现通知到候选的交付边界。

当前像素诊断为关闭状态，数值日志证明上述资格与计数关系。既有独立 WKWebView 对照已经复现 video 纹理图像变化与 RVFC 合批同步下降，并验证独立解码供给约 60fps；参见 [逐解码帧供给 Report](../../docs/reports/2026-11-09-browser-decoded-frame-supply.md)。这份日志把持续回退与首帧等待时序明确关联起来。

## 六项健康与新鲜度边界

- 网络健康：RTT P95 26ms、jitter P95 8ms；完整播放新增丢包 176、NACK 112、收到重传包 267。
- 恢复链健康：一次 227ms 媒体组帧/缓冲停顿后解码追赶恢复；Native ReferenceChain / cleanAnchorCommitted / DisplayStable 为 Browser 能力限定项。
- Media supply：接收/解码稳定约 60fps；reader 兼容回退后，renderer 消费呈现回调。
- Steady supply：各效果主段约 52fps，最低完整 3s 窗 44.92–46.70fps；原画同样存在持续缺口。
- Submit/present 长尾：同步 draw CPU P95 1ms，绘制间隔 P95 33–34ms、全代次最大 218ms。最近提交年龄只是距上一提交的时间；GPU 完成时间与物理上屏继续缺观测。
- 帧供给 delta：已交付候选与提交配平；逐解码帧 reader 接收为 0，原画配对窗仍有 273 帧差额。

## 优先修复方向

沿现有共享帧源修正 reader 生命周期：分开接口/Worker 初始化截止与等待媒体首帧，Worker ready 形成明确初始化事实；媒体首帧等待以视频轨 unmute 或实际媒体供给进展作为锚点，并沿既有会话生命周期约束终止与资源释放。

该修复已在本次用户授权后完成，行为与验证见 [启动期限修复报告](../../docs/reports/2026-11-10-decoded-frame-startup-deadline-fix.md)。本报告数值保留为修复前基线。

恢复后的供给继续保持 maxBufferSize=1、Worker 在途一帧、最新已解码可显示帧即时提交、独立 localSequence 和迟到帧关闭。验收增加“轨已创建、连接晚于 2s、首帧晚于 2s”的启动场景，并以 decoded 接收量、候选量、提交量、帧年龄及 GPU 完成长尾共同判断设备收益。
