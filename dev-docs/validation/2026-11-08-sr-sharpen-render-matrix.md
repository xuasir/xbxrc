# 2026-11-08 SR / 锐化实机渲染对照

## 结论

最新本地样本证实 SR 路径持续提交不足。界面锐度 3 时，两个 SR 窗口提交 49.4 / 40.2 fps；界面锐度 0 时，两个 SR 窗口提交 54.6 / 56.5 fps。四次均自动回退到 video，回退后的完整窗口平均视频推进 59.8 fps。首段排除至少前 1 秒后仍为 48.5 fps，持续掉帧成立。

四组用户设置的实际执行覆盖需要补充：当前有效路径只有 `webgl2_sr` 和 `video`，独立 `webgl2` USM/CAS 的有效样本为 0。SR 两种界面锐度均执行 1440p → 1440p、`passthrough`、`rcasStops=0.88`，标准锐化路径投影均为 `processing=none`。这次数据可以比较实际 SR / video 的供帧表现，对界面锐化 0 / 3 的因果判断仍需要相同执行条件的独立样本。

## 样本范围

- 文件：`/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1794145022876-1.jsonl`
- SHA256：`451e28fbf1737aef6a82291ddd1bafdcdb8727fb394d7b4365e701740faa1df1`
- 3957 行，9,440,655 字节，全部 `traceProfile=dev`。
- 文件时间：2026-11-08 21:37:02.876–21:41:46.599，Asia/Shanghai。
- 成功媒体会话：1 个，目标 2560×1440 / 60fps。首个 SR 提交于 21:38:49.806，用户 21:41:33.833 发起关机退出。
- 原始 WebRTC 统计 652 条，渲染 telemetry 163 条，资源观察 / 快照 95 条，8 个已提交 renderer 代次。
- 数值与计算边界：[派生指标](2026-11-08-sr-sharpen-render-matrix.metrics.json)。派生文件仅保留性能与合同字段。

## 实际路径与帧率

SR 平均值采用同代次提交后的累计实时绘制数差 / 单调时间差；首个准备绘制及其耗时单独记录。video 平均值采用完全落在该代次内的连续 1Hz 视频推进窗口。接收 / 解码采用完全落在代次内的原始约 250ms delta 区间；各列保留自己的观测覆盖边界。

| 已提交路径 / 代次 | 本地时间 | 持续时间 | 实际绘制 / 视频推进 fps | 最低完整 3s fps | 接收 / 解码 fps | 结束原因 |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| SR，界面锐度 3 / 1 | 21:38:49.806–21:38:56.312 | 6.50s | 49.38 | 46.68 | 59.93 / 59.93 | 持续供帧不足回退 |
| video / 2 | 21:38:56.312–21:39:14.411 | 18.10s | 59.94 | 59.76 | 59.97 / 59.91 | 健康后尝试 SR |
| SR，界面锐度 3 / 3 | 21:39:14.411–21:39:18.397 | 3.99s | 40.15 | 38.26 | 46.86 / 46.86 | SR 再试失败回退 |
| video / 4 | 21:39:18.397–21:40:59.763 | 101.37s | 59.76 | 56.54 | 59.75 / 59.76 | 用户再次开启 SR |
| SR，界面锐度 0 / 5 | 21:40:59.763–21:41:05.832 | 6.06s | 54.59 | 51.79 | 59.83 / 60.00 | 持续供帧不足回退 |
| video / 6 | 21:41:05.832–21:41:23.928 | 18.10s | 59.87 | 59.81 | 59.93 / 59.93 | 健康后尝试 SR |
| SR，界面锐度 0 / 7 | 21:41:23.928–21:41:27.915 | 3.99s | 56.46 | 56.37 | 59.93 / 59.66 | SR 再试失败回退 |
| video / 8 | 21:41:27.915–21:41:33.833 | 5.92s | 60.26 | 59.78 | 59.84 / 60.01 | 用户关机退出 |

video 的 138.552 秒完整 telemetry 窗口加权平均为 59.8115 fps；短窗计数相位可以产生略高于 60 的数值。代次 4 的最低 3s 窗口位于 seq2117，接收 / 解码同时降至 56.87 fps，视频推进为 56.54 fps，存在轻微短时波动。

SR 四个窗口相对 60fps 的累计提交缺口分别为 17.69%、33.08%、9.01%、5.90%。这些数值表示目标与实际提交的差额；具体帧的丢弃原因需要各阶段身份记录关联。

排除切换初期后，用后续同代次累计值复算：代次 1 / 3 仍为 48.49 / 48.69 fps，代次 5 / 7 为 55.08 / 55.12 fps。派生指标记录各窗口的起止计数与时间，起点均晚于提交后 1 秒。

## 六段健康证据

### 网络健康

首个 SR 提交至用户退出的完整原始 delta 覆盖 163.979 秒，接收 / 解码均为 59.51 fps，解码 CPU 平均 4.80ms。接收丢包、NACK、FIR 增量均为 0，RTT P95 与 jitter P95 均为 8ms。最后累计 PLI 为 1，发生在启动阶段；最后接收 / 解码均为 9777 帧。

### 恢复链健康

Browser WebRTC 行为 / 稳定性报告为 PASS。成功会话保持同一 peer / video 代次，原始统计记录 1 次冻结、累计 0.3s，位于 SR 代次 3；其余 SR 与 video 窗口冻结增量为 0。该运行时的 Native ReferenceChain / RecoveryLedger / cleanAnchorCommitted / DisplayStable 为 `UNSUPPORTED`，按 Browser 连接、接收、解码与视频推进证据评估恢复。

### Media supply

同一 cached stats sample 对齐后的比较进一步确认供给差额：

| SR 代次 | 同 sample 覆盖 | 接收 fps | 解码 fps | 绘制 fps |
| --- | ---: | ---: | ---: | ---: |
| 1，sample 1:20 → 1:44 | 6.171s | 59.80 | 59.96 | 48.94 |
| 3，sample 1:119 → 1:131 | 3.028s | 50.20 | 49.87 | 42.93 |
| 5，sample 1:534 → 1:557 | 5.800s | 59.83 | 60.00 | 54.66 |
| 7，sample 1:633 → 1:645 | 3.032s | 59.70 | 59.70 | 56.40 |

代次 3 同时存在接收 / 解码供给下降和绘制缺口。代次 1 / 5 / 7 的源供给接近 60fps，绘制持续落后。

### Steady supply

所有渲染 telemetry 使用 `trackingSource=videoFrameCallback`。直接源节拍估算为 `null`，原因为 `mediaTimeDeltaTooSmall`；源供给以上述连续累计接收 / 解码为依据。

完全落在代次内的 1Hz telemetry 窗口，SR 代次 1 / 3 / 5 / 7 的 callback 为 48.68 / 45.85 / 54.60 / 55.78 fps，视频推进代理为 59.81 / 53.49 / 60.07 / 59.76 fps。SR 实际绘制基本承接这些稀疏回调。video 平均视频推进接近 60fps，同时 callback 可以只有约 54fps；两个计数承担独立的事实语义。

SR 代次 1 / 3 的内部窗口 callback gap 分别为 1 / 2 次，最大 callback 间隔为 160 / 285ms；代次 5 / 7 的 gap 为 0，最大间隔 35 / 34ms。各 SR 窗口的 presented-frame jump 均持续增长，表明多帧推进被合入一次回调。`droppedLike` 记录回调 gap 与推进 jump 的并集，具体实际绘制缺口以上表累计值为依据。

### Submit / present 长尾

首个 SR 准备绘制的 CPU duration 为 219ms、上传为 218ms，已从提交后平均帧率分母中分离。后续四个 SR 窗口的保留 CPU duration / upload P95 均为 1ms；提交 CPU P95 均为 0ms。SR 代次 3 的准备绘制为 22ms，代次 5 / 7 为 19 / 23ms。

SR 代次 1 / 3 / 5 / 7 的绘制间隔 P95 为 34 / 35 / 33 / 33ms，超过 60fps 的 16.67ms 节拍；历史最大绘制间隔为 160 / 288 / 36 / 34ms。CPU 调用耗时与实际供帧节拍分别成立。

GPU 查询全部 `state=unavailable`、`queryCount=0`，Browser 物理 scanout 为 `UNSUPPORTED`。本报告的 SR 数值表示 canvas 提交，video 数值表示视频推进代理；GPU 耗时、合成与物理上屏结论需要相应能力证据。

### 帧供给 delta 与回退效果

四个 SR 代次的 renderer overwrite、rate-limit drop、submit failure、duplicate draw 累计均为 0。解码 `framesDropped` 累计为 0。核心异常是源推进与 canvas 实际提交之间的持续差额。

seq547 / 3275 的 `sustainedFrameDeliveryDeficit` 明确持续 3009 / 3013ms；seq1029 / 3758 的 `enhancementTrialFailed` 结束再次 SR 试运行。seq638 / 1114 / 3364 / 3864 分别记录回退后的完整 3s 验收 `result=improved`。

## 结论边界与后续建议

- 四段 SR 的源 / 输出均为 2560×1440，实际执行 RCAS 1:1 路径，`rcasStops=0.88`。SR 界面锐度 0 / 3 均由同一 SR 参数驱动；标准 USM/CAS 只属于独立 `webgl2` 路径。`resources.actualProcessing=cas` 是运行配置字段，实际 shader 以 `renderPolicyApplied.processing` 和 renderer kind 为准。
- 当前 video 长段混有自动回退和用户设置变化。输入 action 白名单把部分操作投影成 `other`，完整“四种设置意图”边界缺少显式值。测试组有效性应以设置意图、有效 shader、当前 renderer 代次与回退状态共同确定。
- 自动稳定性 gate 的 PASS 表示该合同的行为验收通过。生产健康脚本对本 dev 文件给出 `INSUFFICIENT_EVIDENCE`：稀疏 consumed 投影覆盖不足，且缺生产 manifest。帧率复算使用完整原始 stats 与累计绘制证据。
- 优先定位 SR 路径的 video callback → 当前帧交接 → canvas 提交节拍，以及 WebKit 视频纹理上传 / 合成执行情况。独立 USM/CAS 锐化接续有效路径采样，每组保留至少 60 秒前台运行与回退边界。

## 验证方式

运行 `analyze-runtime-logs` 的汇总与 Browser / 生产健康报告，并逐个检查回退及资源累计值的原始 seq。派生脚本验证了原始 sampleId 唯一、8 个提交代次，以及每个 SR 代次内累计绘制数的单调性；缓存时间与现场诊断时间分别对齐，跨代次的累计值分别处理。产品源码保持当前版本。
