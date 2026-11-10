# 11:17 最新日志：独立解码供给与 SR / 锐化复验

独立 Worker VideoFrame → WebGL 路径已生效，主要效果段提交 59.84–60.01fps。此前 RVFC 驱动下的持续 51–53fps 缺口明显收敛；本次原画、稳定 USM 段与第二次播放的配对窗口，解码→提交新增缺口均为 0。首段仍有一次新增 4 帧差额及短时供给突发，原画段出现一次 175ms 冻结；继续优化应围绕帧供给时序与画面新鲜度。

## 样本与身份

- 源日志：`/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1794280647769-1.jsonl`。
- 稳定快照：`/tmp/xbxrc-2026-11-10-latest-after-sampling.jsonl`。
- 时间：2026-11-10 11:17:27.769–11:22:56.128，Asia/Shanghai；328.359s。
- 3954 行、8,793,323 字节；schema v3/dev；JSON 全部有效，seq 1–3954 连续。
- SHA256：`66be2ef3c70c17cc61f7464ccf81b0c0803e68bdd69ecf398cf7705b53f13af3`。
- Browser/macOS/debug，commit `76a1ea69`，dirty=true，构建时间 `1794280632326`。
- 构建源码指纹：`6be9987d2344adba8fdbc95a0ab849dc6ce1a5869f2d205ff211f655f4072383`。首次按 `src-tauri/build.rs` 对 555 个输入复算一致；后续工作区整体指纹随其他编辑变化，日志结论绑定本次构建。
- 采样相关五个源文件与[像素验证身份](2026-11-10-sr-sharpen-sampling-optimization/source-identity.json)一致，记录见[源码核验](2026-11-10-decoded-supply-latest-trace-analysis.source-verification.json)。

两次会话分别为 `F97470F1-B849-4C55-8248-78064674DBEB`、`BB06D0CB-79C6-4477-9835-9B3F405F8114`。两会话各自只有一个已提交 renderer generation，第一会话在同一实例内更新实际效果。

## 实际效果与稳定供给

| 会话 / 实际段 | 时间 | 持续秒数 | 提交 FPS | 最低完整 3s 提交 FPS | 配对解码→提交新增缺口 |
| --- | --- | ---: | ---: | ---: | ---: |
| 1 / SR 开启、USM 3；同尺寸跳过重建 | 11:19:22.127–11:20:19.532 | 57.404 | 59.84 | 58.45 | 4 / 2285 帧 |
| 1 / 原画 | 11:20:20.446–11:20:49.758 | 29.310 | 60.01 | 56.56 | 0 / 1695 帧 |
| 1 / USM 3 | 11:20:49.758–11:21:20.547 | 30.790 | 59.95 | 58.86 | 0 / 1800 帧 |
| 2 / SR 开启、USM 3；同尺寸跳过重建 | 11:21:53.580–11:22:42.628 | 49.049 | 59.95 | 58.86 | 0 / 2840 帧 |

11:20:19.532–11:20:20.446 另有 0.916s 的 USM 过渡段，提交 57.86fps；完整 3s 覆盖为 0，单列为切换观察。

输入、输出均为 2560×1440，已提交状态 `actualSuperResolutionActive=false`。本次覆盖同尺寸直通与 USM 3 的设备吞吐；EASU 放大路径的硬件收益接续真实输入小于输出的样本。SR 源码 48→16 次兼容 RGB fetch 的数值一致性由此前软件 GPU 像素验证覆盖。

第二次会话的用户选择与启动请求为 **1080p**：`sessionExecutionStarted seq 2806` 的 runtime 目标为 1920×1080；本地 offer seq 2821 / 2828 已写入 `max-fs=8160;max-fr=60`。远端 answer seq 2838 的视频 `imageattr` 声明发送范围上限 2560×1440 / 60fps，首个有效 inbound stats seq 2889 已为 2560×1440，随后全部 194 个有效尺寸样本均为 1440p；seq 2878 也记录 `configuredTier=1080p / actualSourceTier=1440p / sourceAboveRequested=true`。本次观察到远端实际发送尺寸超出请求，具体约束落点接续会话参数与协商排查。

完整 3s 窗的条件为：decode 与 renderer coverage ≥2999ms，窗口起止均位于同一个实际效果段及 renderer generation。第一段有 55 窗，原画 40 窗，稳定 USM 42 窗，第二会话 52 窗。

## 帧供给 delta

| 同样本配对窗口 | 覆盖秒数 | 解码帧数 | 提交帧数 | 新增差额 | 解码 / 提交 FPS |
| --- | ---: | ---: | ---: | ---: | --- |
| 会话 1 整体已观测窗，`1:31→1:425` | 99.373 | 5959 | 5955 | 4 | 59.97 / 59.93 |
| 会话 1 原画，`1:190→1:302` | 28.264 | 1695 | 1695 | 0 | 59.97 / 59.97 |
| 会话 1 稳定 USM，`1:306→1:425` | 30.008 | 1800 | 1800 | 0 | 59.98 / 59.98 |
| 会话 2，`3:24→3:212` | 47.387 | 2840 | 2840 | 0 | 59.93 / 59.93 |

配对以 `sessionId + sampleId + sampledAtMs` 为准。RTC 与 renderer 的同步观察存在很小的边界误差，第二会话的累计 gap 为 -1～0 帧；该级别可由同步采样边界解释。

第一会话累计 decode−submit gap 在早期为 7～8 帧，`seq=1333 / sample=1:170 / sampledAtMs=166205` 起为 12 帧，此后保持至结束。新增 4 帧集中在该局部窗口，占首段配对解码帧约 0.18%；其余主要段 gap 增量均为 0。初始 7～8 帧包含启动与首次已观测窗口之前的差额，进一步定位需要处理器读取与 Worker 在途证据。

reader→renderer 采用同一个 live resource snapshot 配对：第一会话 54 个快照、第二会话 24 个快照全部收帧=提交。末次分别为 `7047=7047`（seq 2689）和 `2885=2885`（seq 3876）；`rateLimitDropCount / submitFailureCount / duplicateDrawCount / latestOverwriteCount` 均为 0。此证据覆盖 reader 已交付后的路径；处理器队列内部的覆盖与 Worker 读取等待仍缺独立计数。

RVFC 保持诊断用途：三个第一会话主要段约 50.22 / 50.43 / 50.73Hz，第二会话 51.56Hz；video 呈现计数代理约 59.93–60.10fps。各段 RVFC 多帧推进合并为 364 / 267 / 273 / 390 次。`frameIdentity.accepted.decodedFrame=0` 属于呈现身份观测口径，独立帧交付应读取 `decodedFramesReceived` 与实际提交计数。

与[09:54 基线](2026-11-10-render-effects-latest-trace-analysis.md)比较：旧原画配对 39.088s 为解码 2343 / 提交 2070，缺 273 帧（11.65%）；新原画配对 28.264s 为 1695 / 1695，缺 0。两份日志的网络和操作不同，比较证明持续供给缺口的收敛趋势，shader GPU 加速幅度另行验证。

## 本地提交节拍与长尾

| 实际段 | CPU 绘制均值 ms | 绘制 P95 ms | 段内保留窗提交间隔 P95 范围 ms | 段内捕获最大提交间隔 ms | 最近提交年龄 P95 / 最大 ms |
| --- | ---: | ---: | --- | ---: | --- |
| 会话 1 / SR 开关＋USM 3 | 0.366 | 1 | 26–72 | 154 | 48 / 56 |
| 会话 1 / 原画 | 0.368 | 1 | 24–32 | 176 | 24 / 26 |
| 会话 1 / USM 3 | 0.345 | 1 | 27–36 | 84 | 30 / 36 |
| 会话 2 / SR 开关＋USM 3 | 0.349 | 1 | 26–32 | 149 | 20 / 46 |

CPU 绘制包含同步上传与 WebGL 命令提交；CPU upload 均值约 0.326–0.341ms，保留窗 P95 为 1ms；submit P95 为 0ms，受当前毫秒时钟精度影响。初始化首帧绘制最大 6ms。

分位数来自 256 项环形窗口，筛选时要求当前效果段已提交至少 256 帧；各段合格快照 59 / 39 / 40 / 66。多个保留窗相互重叠，表内范围表达已捕获窗口的分位数，整段全帧分位数需要逐帧记录。最大间隔同样是已保留观察范围内的最大值。

GPU timer：`EXT_disjoint_timer_query_webgl2 / state=unavailable / queryCount=0`；148 个 surface observation 的 `screenPresentState=unavailable`，contextLost=false。当前可确认 CPU 成本与本地提交间隔；submit→GPU 完成→物理上屏长尾属于缺证项。`rendererSubmitAgeMs` 表示采样时距离上次本地提交的时间；WKWebView 的 VideoFrame timestamp 在 78 个供给快照中均为 0，解码帧年龄和 glass-to-glass 同样需要额外证据。

## 网络与恢复健康

| 完整播放观察区间 | 接收 / 解码 FPS | RTC dropped 增量 | 网络丢包终值 / 收包终值 | RTT P95 / 最大 ms | jitter P95 / 最大 ms | 冻结 |
| --- | --- | ---: | --- | --- | --- | --- |
| 会话 1，117.470s 完整 RTC delta | 59.94 / 59.94 | 0 | 168 / 274939，约 0.061% | 54 / 118 | 11 / 18 | 1 次，175ms |
| 会话 2，48.655s 完整 RTC delta | 59.95 / 59.93 | 0 | 23 / 94533，约 0.024% | 87 / 87 | 8 / 13 | 0 |

两会话 ICE/peer 连接完成后持续收到媒体，完整 RTC delta 中接收 0 帧窗口为 0，pause 增量为 0；帧内短暂停顿仍由下述冻结与提交间隔捕获。平均 decode 为 4.38 / 4.23ms，实际 jitterBuffer dwell 平均 6.04 / 4.10ms。

冻结证据位于 `seq=1732 / 11:20:33.848 / sample=1:241 / sampledAtMs=184133`：上一 250ms 窗只收到/解码 6 帧（seq 1724），随后 252ms 窗收到/解码 21 帧；RTT 从 15ms 升至 107ms，jitter 为 14ms，freezeCount +1、totalFreezesDuration +0.175s。相邻提交观测 seq 1739 的最大间隔为 176ms，同期绘制 P95 维持 1ms、该段 decode→submit 新增缺口为 0。高置信结论为短时上游供给波动及集中到帧；网络瞬时波动与冻结同窗，确切触发点还需包到达及解码输出时序。

首段新增 4 帧窗口 seq 1333 有 `renderCause=frameDeliveryLimited`，最近提交年龄 56ms；下一样本 seq 1354 年龄 39ms。相邻 RTC 只有少量丢包/NACK，RTT 约 14–15ms；本地 CPU 保留窗 P95 1ms，实际原因仍需处理器/Worker 时序分解。该局部差额随后保持固定，持续供给缺口已经退出。

Browser 恢复能力：关键帧终值 6 / 5，启动 PLI 终值 2 / 1，FIR 0 / 0；完整播放窗 PLI、FIR 增量均为 0，NACK 增量 64 / 116，重传包增量 249 / 121。数据支持两次媒体持续解码。Native `ReferenceChain / cleanAnchorCommitted / DisplayStable`、逐次 NACK 有效/迟到/过期/跳过、关键帧请求逐次闭环均为能力限定项；恢复有效率与链构建成功率保留为缺证。148 次 `recoveryCauseClassified=unknown` 属于 Browser 当前分类覆盖。

## 启动、退出与证据门

- 会话 1：peer connected seq 455，renderer committed seq 467，间隔 799ms。
- 会话 2：peer connected seq 2866，renderer committed seq 2884，间隔 331ms。
- 两次 committed 状态为 `renderFrameSource=worker / decodedFrameReaders=1 / decodedFramesReceived=1`，后续所有 telemetry 均为 worker，故障回退证据为 0。
- 退出 seq 2715 / 3906，renderer、reader、保留帧、回调、视频、音频及准备 timer 均归零。
- traceBudgetNotice 0，所有 telemetry 的 producer dropped/queueFull/IPC failure/timeout/essential loss 最大值均为 0。
- 分会话执行 `trace_browser_webrtc_behavior_report.py --require-stability`：会话 2 PASS；会话 1 stability=MISSING，唯一缺项 `sharedSamples=0`。149 个 observation 与 100 个 telemetry 均保留，panel 与 supervision 的采样 ID 在此会话错开，门禁要求两消费者共享同一 ID；该结果限定为共享采样证明缺口。

## 建议

1. 保持到帧即提交、latest-only 与单帧在途，继续将新鲜度作为首要目标。
2. 下一次定位优先补齐 processor read 等待、Worker 发送→主线程接收→ACK，以及同步帧序号/丢弃计数；分解局部 4 帧差额与 100ms 以上空窗。
3. 使用真实放大输入/输出的 SR＋USM 配对样本，验证 EASU 采样合并的硬件成本；同时采集可用的 GPU 完成与外部上屏时序，评估 submit→present 长尾。

## 复算

[脚本](2026-11-10-decoded-supply-latest-trace-analysis.py)与[指标 JSON](2026-11-10-decoded-supply-latest-trace-analysis.metrics.json)按 session/代际/实际效果分段；全部累计提交单调、原始采样 ID 唯一、live reader/submit 配平检查通过。源码与业务行为保持当前版本。

```sh
python3 -B dev-docs/validation/2026-11-10-decoded-supply-latest-trace-analysis.py \
  /tmp/xbxrc-2026-11-10-latest-after-sampling.jsonl \
  --source /Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1794280647769-1.jsonl \
  --output dev-docs/validation/2026-11-10-decoded-supply-latest-trace-analysis.metrics.json \
  --source-root /Users/guo.xu/Documents/code/games/xbxrc \
  --source-verification dev-docs/validation/2026-11-10-decoded-supply-latest-trace-analysis.source-verification.json \
  --browser-report /tmp/xbxrc-latest-session-1.behavior.json \
  --browser-report /tmp/xbxrc-latest-session-2.behavior.json
```

分会话 Browser gate 的临时输入保存在 `/tmp/xbxrc-latest-session-{1,2}.jsonl`，可从源日志按上述 sessionId 重建。基础供给复算可省略两个 `--browser-report`，共享采样缺项与 gate 摘要已保存于指标 JSON。
