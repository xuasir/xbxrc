# iOS 显示生命周期修复后的新 Trace 复验

2026-11-23。后台就绪补送与底层轨道身份去重已出现运行证据；当前帧供给目标保持 Validation Pending。对齐 95 个完整前台窗口、95.24 秒，接收/解码 59.94FPS、GPU 完成 52.52FPS、系统 drawable 上屏 52.48FPS，解码到实际上屏计数差 12.45%。

## 范围与门禁

原件：[runtime-trace-ios-1795401150207-1.jsonl](../../../runtime-logs/ios/runtime-trace-ios-1795401150207-1.jsonl)，来自用户 Downloads 的同名文件。单个启动 session，26163 行，schema v3、dev；时间 2026-11-23T10:32:30.206+08:00 至 2026-11-23T10:39:48.672+08:00，共 438.466 秒。配置回报 iPad8,6 / OS 27.0、2560×1440、80Mbps 上限，全部 5042 次提交保持 1440p、metalZeroCopy。

- [串流 core 门禁](core-analysis.json)：PASS。首轮 peerEpoch=1 处于连接阶段，fallback 回退重建后的 peerEpoch=2 完成控制、媒体、显示和清理闭环；全量原始结果压缩保留为 [core-analysis.full.json.gz](core-analysis.full.json.gz)。
- [基础全流程门禁](base-analysis.json)：FAIL，精确失败为 seq=128、131 两个 imageCandidateStarted 缺少同 operationId 的 imageCandidateSucceeded / imageCandidateFailed。启动、库页面必需锚点、schema、序列、预算及隐私均通过。
- seq 从 1 连续至 26163，完整行解析通过，恢复尾行与重复/回退序列均为 0。隐私违规 0，压力通知 0。原始文件 15.74MiB，配置 dev 单文件上限 32MiB、最多 6 份；本次验收范围为用户提供的单个原始文件。

## 生命周期与覆盖

| seq | tsMs | domain / event | 事实 |
| --- | --- | --- | --- |
| 394 | 1795401471772 | ios-app / scenePhaseChanged | to=background |
| 425 | 1795401476745 | ios-streaming / displayPipelineReady | peerEpoch=1 |
| 434 | 1795401479703 | ios-app / scenePhaseChanged | to=active |
| 435 | 1795401479706 | ios-streaming / displayPipelineReadyDelivered | peerEpoch=1, reason=foreground, presentationRevision=3 |
| 436 | 1795401479786 | ios-streaming / streamingStateChanged | state=waitingForFirstFrame |
| 438 | 1795401480044 | ios-streaming / sessionResumeValidated | peerEpoch=1 |
| 440 | 1795401480046 | ios-streaming / fallbackTurnRetry | peerEpoch=1 |
| 472 | 1795401483953 | ios-streaming / displayPipelineReady | peerEpoch=2 |
| 482 | 1795401484790 | ios-streaming / peerConnected | peerEpoch=2 |
| 543 | 1795401485326 | ios-streaming / displayFirstPresent | peerEpoch=2 |
| 545 | 1795401485328 | ios-streaming / streamingStateChanged | state=playing |
| 26148 | 1795401581540 | ios-streaming / sessionTerminal | peerEpoch=2, reason=user.stop |
| 26159 | 1795401582190 | ios-streaming / accessReleased |  |

管线在后台建立，前台 scenePhaseChanged 到 displayPipelineReadyDelivered 的源时钟差为 3.087ms，送达原 peerEpoch=1 的 presentationRevision=3。恢复先发布 waiting，再完成 validateResume；随后沿既有 fallback 路径重建 peerEpoch=2，首显后发布 playing。playing 共 1 次，两个 peer 各绑定 surface 1 次，旧版每秒重复绑定已收敛。首轮 peer 的就绪补送与后续 peer 的正常播放分别保留原始锚点。

本次覆盖创建会话期间进入后台、恢复前台、首帧播放、菜单可见性通知以及 user.stop 清理。播放期间的多次前后台恢复、同 peer 的后台持续供给后直接恢复播放，继续由后续运行覆盖。控制菜单可见性通知 4 条，只提供 button / autoHide 与 visible；具体子页、调整动作和主线程任务栈需要更细的观测。

## 帧供给与前后口径

完整前台口径采用同 attempt、generation、peerEpoch、sampleId 且两侧时间边界一致的窗口；实际 sampleWindowMs 分别核对，首次 GPU 完成前与跨前后台窗口单列。FPS 为 counter delta 之和除以实际窗口时长之和，窗口按 [CSV](steady-windows.csv) 逐项复算。

| 完整前台 95.24 秒 | 计数 | 加权 FPS |
| --- | ---: | ---: |
| 接收 / 解码 | 5709 | 59.94 |
| 提交 / GPU 完成 | 5002 | 52.52 |
| 系统 drawable 上屏 | 4998 | 52.48 |

沿前轮解码 ≥50FPS 的正常供给口径比较，当前 94 个窗口共 94.24 秒：

| 正常供给口径 | 前轮第二段 | 本轮 |
| --- | ---: | ---: |
| 解码 FPS | 59.94 | 60.57 |
| GPU 完成 FPS | 58.62 | 53.04 |
| drawable 上屏 FPS | 58.32 | 52.97 |
| 解码→GPU 计数差 | 2.19% | 12.44% |

正常供给口径排除本轮 1FPS 窗口，随后计数追赶窗口仍完整保留，故解码加权值为 60.57FPS。完整前台口径同时展示这两段，解码加权值为 59.94FPS。两份日志均为 1440p，场景、网络条件、运行时负载及采样长度各自记录；这些观测用于定位当前损耗及判断待验状态。对比输入、逐窗口和公式见 [comparison.json](comparison.json)、[health-summary.json](health-summary.json) 与 [可复算脚本](analyze_display_trace.py)。

逐事件计数：5047 次接受、5 次 mailbox 覆盖、5042 次提交与 5042 次 GPU 完成；每次提交都有同 attempt/generation/peer/frameId 的完成。730 帧应用丢弃全部属于 ingressCoalesced，接受加入口合并丢弃对应 5777 次入口帧。最后一份 RTC/显示采样距 user.stop 源时间约 301.66ms，最终逐帧事件与末次累计快照各自保留边界。

## 六维健康

| 维度 | 证据与结论 |
| --- | --- |
| 网络健康 | peer2 为 IPv4 UDP、srflx→srflx。RTT 中位 15ms / P95 44ms，jitter 中位 4ms / P95 9ms；video 作用域最后 packetsReceived=156253、packetsLost=42，约 0.027%。首轮未闭合连接经既有 fallback 重建。 |
| 恢复链健康 | peer2 串流 core PASS；validateResume 按 waiting→validated 顺序执行，随后正常重建、首显和 playing。最终 NACK=33、PLI=1、FIR=0；95 份 supervision 中 88 份 quality/network stable、94 份 fresh，恢复布尔事实前 5 次 active，后 90 次 inactive。ReferenceChain/RecoveryLedger 的 SDK 内部状态沿已有能力边界解释。 |
| media supply | 完整前台接收与解码均 59.94FPS，RTC 累计 framesReceived=framesDecoded=5760、framesDropped=0。所有提交尺寸为 2560×1440。 |
| steady supply | 首显后连续前台约 96 秒。16 个 GPU 完成间隔超过 100ms；seq24930 窗口接收/解码仅 1 帧，GPU 间隔约 1027.81ms；SDK 最终 freezeCount=2、freezeDuration=1314ms。低供给窗口与恢复后的追赶窗口均保留。 |
| submit/present 长尾 | 单帧提交→GPU 完成 P50 0.938ms / P95 1.651ms / P99 2.596ms；drawable 获取 P95 0.093ms，绘制准备 P95 0.664ms。滚动 drawable P95 快照最大 32.411ms、P99 快照最大 40.142ms；这些是滚动分位数。GPU→MainActor 最大 663.881ms、P99 17.681ms；GPU 释放帧槽 P95 0.00208ms。 |
| 帧供给 delta | 对齐前台解码5709→GPU5002→drawable4998，计数差分别707 / 711帧。全段入口合并730帧、mailbox覆盖5帧；提交与GPU完成配平。各窗口使用同sampleId和实际时间范围，100ms健康合并的采样边界纳入解释。 |

## 损耗定位与下一步

seq25465 的 GPU 已完成，MainActor 处理等待 663.881ms；seq25468 紧接记录 ingressCoalesced=38，后续 GPU 完成间隔为 666.294ms。该窗口 RTC 解码仍有 61 帧、丢包增量 0，原始行分别保留在 [锚点](evidence-anchors.json) 的 25460–25478 段。seq13395 合并 18 帧、seq15512 合并 13 帧，也对应本地呈现长间隔。

原入口在 SDK 回调保存 latest 帧，消费与 Metal 唤醒由 MainActor 执行；成功 GPU 完成独立释放 mailbox，后续记账与 scheduler 唤醒在 MainActor 回调继续。当前计数和上述时序支持优先定位 MainActor 的执行等待与入口消费机会。实际占用栈由下一轮采样确认，菜单/诊断刷新、设置动作与调度竞争作为具体检查范围。ingressQueueMs 仅统计最终被接受的最新帧；被合并的旧帧具备累计丢弃量，补充 drain 任务排队时间可以完整观察一次主线程停顿。

seq24930 的 1FPS 窗口显示一次接收/解码供给缺口，随后出现追赶；这一段同时核对发送端和接收栈的供给事实。保持现有 latest-only、Rust 控制/信令、libwebrtc 与 Metal 主线，后续以降低本地长等待、减少入口合并损耗为目标，继续核验网络、恢复和显示六维指标。

完整门禁的图片问题见 [image-pairing-diagnostics.json](image-pairing-diagnostics.json)。seq128、131 候选开始后，两条邻接网络完成均为 HTTP200；CircularCardCarousel 在 await 返回后的取消检查可直接结束任务，形成同 operationId 的终结观测缺口。实际取消原因的独立锚点继续补齐。[源码摘录](source-excerpts.txt) 和 [源码指纹](source-hashes.json) 保存解释所依据的版本；此前七份验收源码保持一致。

复验结果见 [validation.json](validation.json)，原件与仓库副本 SHA-256 一致。临时目录和清理核验见 [cleanup.json](cleanup.json)，归档完整性见 [artifact-manifest.json](artifact-manifest.json)。原件、前轮证据及共享 Rust/WebRTC 缓存继续保留。

本轮分析与导出命令均已返回完成状态，专用临时根目录已清理，移除分配量4.89MiB；清理后可用空间51.88GiB。原始用户Trace、项目副本、源码/分析证据和共享Rust/WebRTC缓存保留。
