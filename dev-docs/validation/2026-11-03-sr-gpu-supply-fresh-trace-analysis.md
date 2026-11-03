# SR GPU 补供帧新日志验收

2026-11-03。最新设备日志显示，SR 60fps 修复仍处于待验状态。GPU 探测器首次 CPU 操作合计196ms，超过50ms冷预算后停用；全会话补交付0帧。两段SR完整3秒窗口提交中位数51.388/46.972fps，解码60.033/60.142fps，主要帧差继续位于解码到候选供给之间。

## 样本与构建

采样文件为 [runtime-trace-1793612198686-1.jsonl](/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793612198686-1.jsonl)，范围2026-11-02 17:36:38.686–17:39:54.739，共196.053秒、1678行。媒体统计范围17:38:04.117–17:39:44.727，共100.611秒、313个样本。SHA256：`beb9551a6c0b918548c87f7d7e4ded830e63c344e63ba2de4c6d4035a1d7bcc7`。

构建为macOS/debug/browser/home、commit `85c39d6b`，日志源码指纹 `0290f209ce2662019930a2874a099edebd26fe01bcc41bbb3e8ff32c3e229d23`。当前398项源码输入与日志整体指纹有差异；帧源、GPU像素观测器、SR renderer及FramePacer四个模块与采样commit逐文件一致。运行时JS产物的独立指纹继续待补。

路径依次为SR g1→video g2→SR试探g3→video g4，共用videoGeneration1。全程目标60fps、源与输出2560×1440；SR为等尺寸passthrough、锐化0。实际升分辨率效果、尺寸变化、30fps配置及平台长测继续待验。

## GPU补供给停在冷启动

seq207，单调114,668ms，开始收集即时诊断：SR已经提交852帧，像素readCount0。seq223（`tsMs=1793612314678`）首次发布读取结果，firstRead记录114,680–114,876ms：

| 首次操作分项 | CPU同步耗时 |
| --- | ---: |
| WebGL2资源初始化 | 171ms |
| video纹理导入、缩略绘制及PBO/fence提交 | 25ms |
| getBufferSubData取回 | 0ms，阶段尚未执行 |
| CPU合计 | 196ms |

GPU异步等待记录0ms，实际等待阶段尚未执行。初始化包含context、shader编译/链接及纹理/FBO/PBO准备；25ms是整个upload分项，video导入、绘制和驱动等待内部比例继续缺少细分。

`outcome=readbackBudgetExceeded`，readCount/completedReadCount为1，warmReads、baselineCount、pairCount、changedPairCount、deliveredFrameCount均为0，探测canvas/PBO/timer释放归零。后续SR g3仍保留同一次firstRead和disabledReason，全会话polledFrameCount为0。

[read与冷预算判断](../../src/player/infra/render/video-frame-pixel-observation.ts:268)将初始化、上传和取回CPU成本累计后对照一次50ms冷预算，随后按8ms稳态预算执行。[停用状态](../../src/player/infra/render/video-frame-pixel-observation.ts:137)在后续observe入口直接结束；reset释放资源时保留disabledReason。两次SR使用同一video帧源，因此renderer重建后继续保留停用状态。

补帧效果的设备证据目前集中于“首次读取超预算→停用→补交付0”。warm成本、有效像素配对与补帧收益均为MISSING。诊断启用前的readCount0还需要连续probe eligibility事实解释，包括document.visibilityState、video.paused/readyState和请求FPS。

## SR同窗供给与回退

窗口按renderer代际、前台状态及五项完整覆盖筛选，允许重叠；video输出定义为浏览器推进，SR输出定义为成功本地提交。

| 阶段 | 完整3秒窗口 | 解码fps中位数 | 候选fps中位数 | 输出fps中位数 | 同窗解码→输出差中位数 |
| --- | ---: | ---: | ---: | ---: | ---: |
| SR g1 | 4 | 60.033 | 51.388 | 51.388 | 8.370fps |
| video g2 | 15 | 59.872 | 44.635 | 59.872 | −0.033fps |
| SR试探g3 | 1 | 60.142 | 46.972 | 46.972 | 13.169fps |
| video g4 | 39 | 59.859 | 52.206 | 59.839 | 0fps |

SR g1完整窗口原始seq257→313，g3为seq773。两段SR完整15秒窗口均为0；首段总运行35.215秒，其前29.180秒尚未开启窗口诊断。

即时同代际诊断与缓存窗口分别结算：

| SR区间 | 原始seq | 时长 | rAF观察率 | RVFC/候选率 | GPU补帧 | 候选→提交 |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| g1，含探测冷启动 | 207→320 | 6.026s | 56.090Hz | 48.457Hz | 0 | 292→291，99.66% |
| g3，增强试探 | 715→780 | 3.012s | 60.093Hz | 47.477Hz | 0 | 143→143，100% |

首段诊断前seq113→207，候选增加853、提交增加851，提交29.164fps；同期首段接收/解码59.888/59.917fps，freeze/drop增量0。诊断启用与刷新率估计更新seq216附近出现帧率边界，设备可见性、窗口状态及回调节拍的因果关系继续待受控证据确认。

两段SR候选到提交的即时损失分别1/0帧，重复绘制和提交失败0；主要持续差距由候选供给解释。seq312持续帧差回退，seq398效果improved；seq684健康增强试探，seq772试探失败回退，seq777试探结果neutral；seq862回退效果improved。最终网络恢复后seq1633有1个完整15秒窗口：解码59.967fps、video推进59.900fps。

相较[修复前10:59日志](2026-11-02-pixel-cost-fresh-trace-analysis.md)，SR两段完整3秒输出48.087/58.444→51.388/46.972fps。两份独立会话的阶段长度和网络分别保留；本轮GPU补交付0，修复收益继续待设备有效运行证据。

## 渲染成本与长尾

1329次首处理之后的成功绘制，CPU同步总均值0.561ms，上传0.517ms、GL提交0.035ms。首SR资源准备1870ms、候选等待9437ms、首draw214ms，其中首次上传213ms；请求到首提交11,549ms。初始源尺寸为0，候选等待单独结算。SR重试资源准备10ms、首draw23ms，请求到提交37ms。两次SR→video分别11/1ms。

11个即时提交年龄样本P50/P95/max为10/109/201ms；201ms来自seq223，和探测器196ms冷操作处于同一采样段。其余10个样本为2–17ms。年龄表达采样时最新成功提交的新鲜度，GPU执行、submit→present、glass-to-glass及物理屏幕长尾继续MISSING。

`trackingSource`全部videoFrameCallback。SR g1/g3同代际callback gap增量3/0，多帧计数跳跃49/38，dropped-like50/38；推进量363/181、回调292/143，最大回调间隔211/51ms。回调合批与实际GPU失败分别通过各自证据判断。sourceFpsEstimate全部因mediaTimeDeltaTooSmall缺证，源帧率采用RTC增量；sourceFrameIntervalMs、maxPresentedFramesDeltaSinceLastSample、droppedLikeStreak和持续renderBackpressure缺字段。独立renderFrameDropped/renderBackpressureChanged/renderCauseClassified事件均为0；renderCause采用telemetry投影，frameDeliveryLimited65、renderStarvation2、decodeBackpressure3、renderStable1。

## 网络与恢复边界

媒体窗口新增接收5863/解码5758帧，平均58.274/57.230fps；正丢失164/231308=0.070901%，RTT P50/P95/max为14/72.45/736ms，jitter P95为18.4ms。313个样本连接状态均connected。freeze4次共4.803秒，解码drop104、pause0；四次freeze都在原生video阶段，SR两段freeze/drop增量均0。

- seq434→455，单调126,519–127,527ms，持续接收包，接收帧/解码帧连续五个窗口为0，累计1260ms。seq462恢复接收101帧、解码5帧、drop80、freeze1.489s，buffer窗口均值755.337ms；seq466再drop10。seq491另记freeze0.449s、drop14。该扰动发生在像素冷操作结束11.643秒以后，媒体停顿与本地探测停用分别保留证据。
- seq1228→1237，连续756ms收包/接收帧/解码帧0；seq1243包恢复820、loss31，媒体仍0；seq1249记freeze1.410s、PLI+1、关键帧decode+1，随后恢复解码16/15帧。
- seq1272→1289，连续1007ms收包/媒体0；seq1293包恢复1679、loss9，媒体仍0；seq1296记freeze1.455s、PLI+1、关键帧decode+1，随后恢复解码15帧。

后两次有包停顿、PLI与新关键帧解码关联事实。远端、网络和浏览器内部处理的责任划分需独立证据。媒体窗口PLI+2、FIR+0、NACK+379、关键帧decode+7、RTX包+2355；NACK与RTX计数按各自粒度结算。

按固定恢复顺序，pictureRecoveryTransition、pictureRecoveryBlockerObserved、videoIngressTermination、firstFrameLatencyObserved和H264内部事件均0。逐请求抑制/合并、usable response、ReferenceChain、cleanAnchorCommitted、DisplayStable、NACK及时/迟到/跳过/过期、建链成功率、repairability连续性与综合恢复评分均MISSING。默认receive报告PASS且决策样本0；严格必需媒体/显示闭环退出2，原因是链条缺证。

native midsegment的media/steady supply为SKIPPED，提交/呈现年龄n/a；浏览器实际media与steady supply采用本报告同窗指标。browser行为与runtime稳定性PASS，2组共享样本、97组frame-chain样本通过。schema3/dev，essential/key/debug为70/995/613，traceBudgetNotice及前端dropped/droppedEssential均0。seq1659正常退出，媒体、renderer、回调及像素资源清零。

## 验证与下一步

[复算脚本](2026-11-03-sr-gpu-supply-fresh-trace-analysis.py)核对1678个唯一seq、89个完整窗口、同代际增量、CPU分项总值、生命周期顺序和稳定SHA，全部通过。[metrics](2026-11-03-sr-gpu-supply-fresh-trace-analysis.metrics.json)保存具体窗口、冷启动、恢复时间线与指纹。通过项表示复算一致性；SR60fps仍待有效设备结果。

下一步优先前置GPU冷资源准备，分开测量context、shader编译/链接与首次video导入，并明确冷启动失败后的有界恢复资格；稳态8ms预算、结果8ms新鲜度及单在途合同继续保留。同步补齐probe eligibility事实，取得warm读取、有效像素配对和实际补交付后，验收持续运动场景至少30秒的SR完整3s/15s供给与真实呈现。当前原生video在网络恢复后的完整15秒窗口推进59.900fps，S4保持In Progress，实机采集由用户手动完成。

```sh
python3 -B dev-docs/validation/2026-11-03-sr-gpu-supply-fresh-trace-analysis.py
```
