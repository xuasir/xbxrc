# 标准 SR 修正后的 Mac/cloud 前台日志查验

2026-10-29：标准输出尺寸与源下限通过本次日志核验；共享帧源的新帧交付与 SR 绘制计数存在持续缺口，实际可见画面的帧率与层归属继续取证。用户确认测试期间持续在前台，并补充实际观感明显好于 3fps。

## 范围与构建身份

- 日志：[runtime-trace-1793263671826-1.jsonl](/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793263671826-1.jsonl)。北京时间 16:47:51.827–16:50:58.927，共 187.100s、2,832 行；媒体采样覆盖 128.905s。
- 环境：macOS debug、cloud、schema v3 / dev；H264 constrained-baseline，answer `profile-level-id=42e02a`。
- Git revision `bec3acfb`、workspaceDirty=true；构建源码 SHA-256 为 `4f1e7668766be3f1e6329f0b732dfa0d07a0de78f2fd4a5cdff13626bca38800`。按 `src-tauri/build.rs` 同一算法复算当前 726 个输入路径，完全匹配本轮标准 SR 与最新帧修正版本。
- 采样区间本地 renderer 预算保持 60fps；一次初始 pipeline 选择、两次 SR 档位冻结、四次 plan 应用。窗口焦点条件采用用户提供的持续前台事实。
- 可复算数据：[机器可读记录](2026-10-29-browser-standard-output-fresh-trace-validation.metrics.json)。既有 [14:24 日志](2026-10-29-browser-policy-fresh-trace-validation.md)包含频繁前后台切换，两批结果按各自测试条件解释。

## 时间线与尺寸核验

| 锚点 | 北京时间 / 阶段 | 事实 |
| --- | --- | --- |
| seq 158 | 16:48:32.234 | 远端 answer 设置成功，选中 constrained-baseline |
| seq 185 / 204 / 207 | 首包 / video 首次推进 / 首次解码观测 | 报告给出连接后 252ms 首包、1034ms 首次解码、1171ms video 首次推进；getStats 阶段存在采样与事件到达误差 |
| seq 200–201 | 16:48:37.070 | 请求 1440p、真实源 2560×1440、显示需求归 1080p；SR 输出仍为 2560×1440，`passthrough`、RCAS stops 0.88 |
| seq 1577–1582 | 16:49:47.757–47.926 | 真实源切为 1280×720；SR 重算为标准 1920×1080，`upscale`、RCAS stops 0.72；原始 inbound 尺寸随后确认源变化 |
| seq 2789–2797 | 16:50:46.421–46.437 | 用户从菜单退出，runtime stop 完成 |
| seq 2796 | 退出资源快照 | video/audio、当前/候选 renderer、帧请求、rAF/RVFC、音轨及相关资源计数归零 |

165 个有效尺寸样本均保持源下限，实际 renderer backing 与 SR plan 一致。1440p 阶段覆盖同尺寸 RCAS 路径，720p 阶段覆盖放大到标准 1080p 路径。真实源变化的上游触发原因需远端证据进一步说明；本次应用层质量调整与重协商动作计数为 0。

## 五段帧供给与稳定供给

以下使用同一 supervision 样本的完整 3s 窗口，均值按真实 elapsedMs 聚合，再取窗口间中位数。1440p 稳定段为 runtime 单调时钟 60.000–112.580s，52 个完整窗口；720p 稳定段为源变化后至少 3s 至媒体采样结束，55 个完整窗口。

| 同窗指标 | 1440p 源 → 1440p 输出 | 720p 源 → 1080p 输出 |
| --- | ---: | ---: |
| received | 30.01fps | 30.03fps |
| decoded | 30.01fps | 30.04fps |
| video advancement | 30.10fps | 29.57fps |
| 新帧 marker 交付 | 3.57fps | 3.63fps |
| 当前 SR renderer 实时提交 | 3.57fps | 3.63fps |
| JS/WebGL 同步提交耗时 | 32.68ms | 12.28ms |

98 个完整 15s 长窗进一步确认：接收、解码、video 推进中位数分别为 29.98、29.98、29.97fps，新帧交付与 renderer 提交均为 3.62fps。持续缺口集中于 video 推进到新帧 marker 交付这一段；renderer 完成了已交付 marker 的处理。

两段都有约 26fps 的 video→marker 吞吐差。callback→submit 差为 0，latest overwrite、pacing wait、实时重复绘制、submit failure 的累计值均为 0。真实源 FPS 估算缺证：126 个 telemetry 样本标记 `mediaTimeDeltaTooSmall`，2 个标记 `mediaTimeMissing`；上述约 30fps 结论表示实际供给速率。

末次资源快照 seq 2761 记录原始 RVFC 3,366 次、轮询补足 20 次、实时绘制 503 次，计数来源为 `playbackQuality`。全部 telemetry 汇总的新帧 marker 为 505 次，video 帧推进量为 3,839；两组累计值覆盖各自首尾采样边界。

当前最强定位是共享帧源的新帧判定与交付链：大量原始 RVFC 未形成可绘制的新帧 marker，轮询补足数量也较低。原始逐次 `presentedFrames / mediaTime`、playback counter、抵扣债务值和 rAF tick 次数尚缺，具体输入失真或抵扣分支继续待复现确认。[帧身份与抵扣实现](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/video-frame-source.ts:132)

## 提交长尾、显示监督与处理成本

- 全段采样时距最近提交的年龄中位 233ms、P95 450.2ms、最大 585ms。1440p / 720p 稳定段的该年龄 P95 分别为 427.8 / 448.9ms。这是提交停顿年龄，真实 `submit-to-present` 延迟需要实际上屏事实取证。
- 每个 telemetry 窗口的最大新帧交付间隔，中位 284ms、P95 428.05ms、最大 570ms；callback gap 459 次、video 多帧推进跳变 457 次。当前 callback 字段表达经帧源筛选后的 marker，原始 RVFC 次数另由资源快照记录。
- 应用 dropped-like 计数累加 480，浏览器 `framesDropped` 增量为 3；两者分别表达交付节拍异常与浏览器丢帧事实。浏览器累计 freeze 41 次 / 10.532s，冻结原因涉及浏览器媒体链和渲染环境，单凭计数尚无法进一步归责。
- 1440p 同尺寸 RCAS 提交成本超过 60fps 对应的 16.67ms 预算。720p→1080p 路径成本降到 12.28ms，新帧交付仍约 3.6fps，因此共享帧源问题拥有优先整改地位；1440p 的处理成本作为另一项性能验收继续保留。
- 129 个归因事件中 127 个为 `frameDeliveryLimited`，2 个为启动附近 `renderStable`。本次软 renderer 回退、增强试探、质量调整和应用层恢复动作均为 0。当前监督依据已接受 marker 的供给与提交关系，video→marker 的持续缺口需要独立闭环。[归因条件](/Users/guo.xu/Documents/code/games/xbxrc/src/streaming/runtime/browser-render-health.ts:10)

## 用户观感补充与结论边界

用户补充实际观感明显好于 3fps。前一轮将约 3.6fps 的 SR 提交计数扩展为体验验收阻断，推断强度过高；当前确认项收敛为“SR 记录的实时绘制约 3.6 次/秒，video 推进约 30fps，二者与用户可见体验存在待解释差异”。

统计使用累计 `realtimeDrawCount` 增量除以实际 elapsedMs；同一 renderer 代际的资源快照也记录约每 2 秒增加 7 次绘制。低计数同时存在于原始资源与 FPS 投影，采样频率和实际帧率各有语义。

源码保留持续播放的 video，并在其后插入 SR canvas；两者共存，canvas 使用绝对定位且绘制后设 opacity=1。`actualPipeline=webgl2_sr` 来自当前 renderer 对象，表达资源选择。[绘制计数位置](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/SuperResolutionWebGL2Renderer.ts:994)与[管线观测投影](/Users/guo.xu/Documents/code/games/xbxrc/src/player/app/media/PlaybackService.ts:169)都尚未记录 WebView 最终可见层及屏幕像素更新。

优先核验实际可见输出由哪一层提供，以及 GPU draw 调用、绘制记账与合成结果是否对应。原生 video 层可见、计数归属偏差等解释作为待验证假设保留；具体判断需 DOM/compositor 可见性与输出变化证据支持。源下限和计数观测结论继续成立，实际视觉帧率保持待测。

2026-10-29 追加[日志与代码离线核验](2026-10-29-sr-frame-delivery-offline-audit.md)：正常 CSS 层叠顺序为 SR canvas 覆盖 video；AST 与受控绘制确认三种 SR 路径统一记账。两路帧身份同时合批时，300 次原始 RVFC 形成40次/37次处理，复现4fps/3.7fps；实际设备原始字段及最终合成归属继续缺证。生产代码与调度参数保持原版本。

## 网络与恢复链

媒体采样全段接收 93,701 包、归一丢失增量 103，丢包比例 0.110%；RTT 中位 205ms、P95 328ms、最大 466ms，jitter 中位 13ms、P95 27ms。平均视频接收码率 6,217.5kbps，API 累计 jitter-buffer delay / emitted frame 为 17.42ms。1440p / 720p 稳定段丢包分别为 0.115% / 0.015%，两段长期供给均约 30fps。

路径具有较高 RTT 与短时尾部波动，低丢包和稳定接收/解码供给为共享帧源排查提供了本域证据。49/509 个样本缺少 selected candidate pair，完整网络窗口覆盖按可用字段解释。

浏览器统计 PLI 增量 5、FIR 0、NACK 23；解码关键帧增量 10。首个关键帧已解码，后续反馈请求与具体可用响应、解码输出之间的配对信息缺证。浏览器内部 NACK 修复结果、ReferenceChain、cleanAnchorCommitted、DisplayStable、repairability 与综合恢复评分在本路径为 unavailable。应用层 reconnect/recovery 为 0，生命周期门禁通过正常退出闭环。

## 自动门禁与观测开销

- `trace_streaming_session_lifecycle_gate.py`：PASS，1 次 operation、0 次 recovery、5 次 cleanup、1 次资源闭环。
- `trace_browser_webrtc_behavior_report.py --require-stability`：FAIL。实际违规为 seq 1961 的 `collectionRateAbove4Hz`：连续采集起点 133194/133434/133686/133938/134191ms，在 997ms 内计入 5 次；相邻间隔包含一个 240ms tick。getStats 在途重叠为 0，此项按周期时钟边界整改。
- `sharedSamples` 证据为 0：129 次 supervision 与 37 次 panel 消费命中不同 sampleId，共享事实一致性的实机配对证明继续待补。
- getStats 509 次，耗时中位 1ms、P95 2ms、最大 47ms；trace IPC 在途最大 2、排队 0、essential/debug 丢失 0，writer `traceBudgetNotice` 为 0；GPU 查询周期预算通过。
- 真实 GPU 工作耗时、scanout、glass-to-glass、堆/RSS 与设备 SR 开关配对继续属于外部测量范围。约 30→3.6fps 的记录差作为计数与输出一致性验收项，实际体验判断结合用户观感和可见层取证。

## 下一步

1. 先核验持续播放的 video 与 SR canvas 的实际可见层、GPU draw 调用和屏幕像素更新，再结合有界 RVFC、轮询计数、抵扣债务与 marker 观测，确定约 4Hz 记录的具体责任分支。
2. 将 video→marker 的持续缺口纳入现有显示监督闭环；供给恢复后单独验证 1440p 同尺寸 RCAS 的 CPU 上传/提交成本与保真回退资格。
3. 校准 250ms 采样的滑动窗边界并补齐 panel/supervision 同样本证据，再执行持续前台 SR 开关配对及跨平台功能/长测。

本轮执行日志分析和验证文档更新，生产代码保留当前版本；任务继续 In Progress。
