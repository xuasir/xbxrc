# SR 跨计数抵扣修正验证

2026-10-30，用户授权继续修正。沿既有浏览器 runtime RFC 执行，验证方式为生产模块加载、受控输入和 DOM/WebGL 替身。

## 故障与修正

[17:53–17:55 设备日志](2026-10-29-sr-diagnostics-fresh-trace-validation.md)记录1746次原始 RVFC，其中1481次被跨计数余额抵扣；采样 `presentedFrames` 与 RTP 均推进。720p 完整窗口接收/解码约30fps、marker/SR约3.63fps，提交成本均值9.94ms。故障入口在共享帧源候选形成。

旧 `counterFramesAheadOfNative` 在 RVFC 与轮询入口同时累积播放计数推进；两路累计计数产生偏移后，余额会连续吞掉新的 native 身份。当前 `polledFramesSinceNative` 仅在实际轮询形成新 marker 时增加。下一次有效 native 身份消费当前窗口的额度，并立即释放剩余；重复 native 身份保留窗口，RVFC 入口读取播放计数只更新基线。native 先交付、播放计数随后推进的抵扣保留，计数来源变化、回退和消费者中断清理资格。

同一 video 继续共用一个 RVFC、一个 rAF、同一消费者集合与取消 epoch。首次零计数、缺证后的计数恢复、原始 RTP metadata、暂停恢复和截图路径均沿原路径执行。

## 回归证据

先增加失败回归，再修改生产代码。修复前两文件6项失败：48帧偏移下，10秒300个源帧仅实时绘制92次；同一帧源内的连续有效 native 身份和轮询剩余额度释放也失败。修复后两文件59项全部通过。

| 10秒受控输入 | 源帧/RVFC | 接受 marker | 实时 SR 绘制 | 第2–10秒交付 |
| --- | --- | --- | --- | --- |
| 对齐计数，RVFC先到 | 300/300 | 300 | 300 | 30fps |
| 第2秒播放计数增加48，修复前 | 300/300 | 92 | 92 | 4fps |
| 相同偏移，修复后RVFC先到 | 300/300 | 300 | 300 | 30fps |
| 相同偏移，修复后轮询先到 | 300/300 | 300 | 300 | 30fps |

轮询先到样本：16次轮询交付，16次晚到 native 去重；合计300个 marker，实时 GPU调用300次。两种输入顺序的 pacing wait、latest overwrite和提交失败均为0。偏移输入中 `presentedFramesDelta` 累积347，保留现有合并推进量语义；marker与绘制次数分别为300，两种计数按各自含义读取。

新增回归覆盖60/144Hz的实际 renderer 处理数量、相邻窗口抵扣、49帧轮询批次消费1并释放48、重复native、native回退、播放计数来源变化/回退、RTP回绕/重复透传，以及暂停后旧代际回调吸收。既有15fps RVFC、30/60fps源、60/144Hz、mediaTime=0的补帧回归继续通过。RTP状态保留原始数值比较；32位回绕可显示regressed，帧资格继续读取native身份。

## 诊断合同与检查

`mediaDiagnostics.frameIdentity.schemaVersion=2`。`polledAdvanceBefore/After`记录轮询交付对应的推进额度；`creditedPollFrames/expiredPollFrames`记录当前有效身份的消费与释放；`nativeReset`记录native基线回退。原始metadata、两路delta和最终newFrame仍同次取证。惰性采集、接受/拒绝各最多2Hz、最近8条证据，以及off/production诊断读取0次的合同继续通过。

- `pnpm test`：45文件347项通过。
- `pnpm lint:fix`、`pnpm exec vue-tsc --noEmit`、`pnpm build`通过。
- 原9组离线输入、RCAS/单通道超分/双通道EASU+RCAS与截图绘制记账验证通过。
- [偏移回放脚本](2026-10-29-sr-counter-offset-replay.cjs)当前断言修复后合同，修复前数值保留为历史证据。[验证数据](2026-10-30-sr-counter-credit-fix-validation.metrics.json)包含当前源码指纹、逐文件SHA-256和复现数值。

## 设备证据范围

旧设备日志证明SR canvas的DOM布局可见、不透明并覆盖媒体区；实际屏幕合成、像素更新、GPU耗时和submit-to-present仍待设备证据。旧日志的网络丢包约0.196%、RTT中位212ms；freeze累计23次/6.923秒，ReferenceChain、cleanAnchorCommitted与DisplayStable内部事实unavailable。当前离线结果证明候选交付与绘制逻辑；网络、恢复与稳态显示的设备效果使用新构建日志验收。

mediaTime固定0、两路计数不同源时，轮询与native的同帧对应依赖当前窗口内推断。单次偏移形成的轮询剩余额度在下一次有效native身份结束后归零；下一份设备日志重点对齐原始RVFC推进、接受/拒绝原因、marker/SR增量和提交年龄长尾。自动软件操作保持暂停，整体RFC继续按原S4范围验收。
