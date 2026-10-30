# 新诊断日志：SR 跨计数抵扣问题确认

2026-10-29：新日志确认共享 VideoFrameSource 的数量抵扣持续筛掉带有新 native/RTP 身份的 RVFC。播放计数与 native 呈现计数存在进度偏移，`counterFramesAheadOfNative` 被作为已交付帧余额保留；后续正增量 native 回调进入 `pollAlreadyDelivered`，新帧交付随播放计数的合批刷新约为3.6fps。源供给、SR提交成本与帧身份抵扣分别取证，当前优先修正帧身份去重。

## 范围与构建

- [日志](/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793267609746-1.jsonl)：北京时间17:53:29.747–17:55:30.998，121.251s、1620行；浏览器媒体采样单调时钟42.811–110.268s，共67.457s。
- macOS debug、cloud、schema3/dev。构建源码摘要`77fe234cf708c9ec1041daadd982fbce26026e98611879d3ca171a1e5a2bbd94`与当前728个源码输入路径完全匹配，绑定[诊断补齐版本](2026-10-29-sr-frame-diagnostics-validation.md)。
- 67个telemetry均含mediaDiagnostics，renderer/video generation全段为1。预算保持60fps，一次管线选择、四次plan应用；应用层SR回退、恢复、重连及质量动作均为0。自动软件操作保持暂停，本轮生产代码保持原版。
- [机器可读数据](2026-10-29-sr-diagnostics-fresh-trace-validation.metrics.json)包含完整统计与原始分支样本。

## 原始身份到 SR 提交的闭环

诊断开始seq220、observedAtMs43762时，SR成功处理累计29帧；末次seq1564、110062ms累计299帧，增量270。诊断窗口原始RVFC为1746次，其中接受265次、拒绝1481次，拒绝比例84.82%；轮询补足5次。265+5=270，接受marker与成功SR处理增量完全对应。

累计原因包括counterAdvanced232、nativeAdvanced38、pollAlreadyDelivered1481、counterUnchanged3930。`pollAlreadyDelivered`表达抵扣分支，该余额也在RVFC入口读取播放计数时增加；轮询实际交付数量为5，标签名称单独使用会造成入口归因偏差。

249条去重后的受控采样证据与sampledCount249一致，环形缓存证据完整传入日志。162条native样本的presentedFrames状态全部为advanced；其中48条拒绝样本的RTP状态全部为advanced。mediaTime在native样本中保持0，nativeUnchangedWithMediaAdvanceCount为0。当前native计数的正推进与RTP新身份已经获得实机证据。

| 日志锚点 | 输入与抵扣 | 结果 |
| --- | --- | --- |
| seq295，样本at47222 | 播放计数增9，native增2；counterAhead从0增到7 | 接受一个marker，数量余额进入后续回调 |
| seq314，at48372 | native增1、RTP推进；播放计数保持128，counterAhead从10减到9 | nativeAdvance=0，newFrame=false |
| seq1545，at108534 | presentedFrames1910→1911，RTP955716566→955719536；播放计数1955保持原值，counterAhead46→45 | `pollAlreadyDelivered`，新RVFC身份被筛掉 |
| seq1564，at109118 | 播放计数增14、native增1，counterAhead36→49 | 接受一个marker，并将大量余额留给后续native抵扣 |

源变化发生前已经出现抵扣：诊断第一个完整秒RVFC接受30/30；seq277时接受42/52，之后提交持续约4fps。seq499–501在58.403–58.405s将实际源从2560×1440切到1280×720、SR输出变为1920×1080；renderer代际继续为1，抵扣缺口持续存在。

应用层机制已经由原始字段和生产代码复现确认。浏览器内部为什么形成两路计数偏移，仍需平台侧事实补充；当前代码将这种偏移视作可抵扣的同帧数量，足以持续压低新帧交付。

## 完整窗口供给与处理成本

以下取supervision消费的完整覆盖窗口，再取窗口间中位数；1440p选择47–58.403s的低交付段，720p在源变化后至少3s/15s开始统计。

| 指标 | 1440p，11个3s窗 | 720p，49个3s窗 | 720p，37个15s窗 |
| --- | ---: | ---: | ---: |
| received fps | 27.27 | 30.04 | 29.97 |
| decoded fps | 27.27 | 30.04 | 29.97 |
| video计数推进fps | 26.31 | 29.76 | 29.95 |
| 接受marker fps | 3.63 | 3.63 | 3.62 |
| SR成功处理fps | 3.63 | 3.63 | 3.62 |
| 同步上传/提交成本ms | 29.59 | 9.94 | 10.02 |

全部telemetry中的latest overwrite、pacing wait、submit failure为0，末次成功提交耗时最大42ms。720p约10ms的处理成本与约3.6fps交付同时存在，定位继续指向候选形成。64次归因为frameDeliveryLimited，启动附近3次为renderStable。

提交年龄采样中位236ms、P95463.9ms、最大963ms；包含启动附近供给冻结。该指标表达距最近提交的年龄，真实submit-to-present继续为unavailable。

## 实际 DOM 显示状态

67份快照均记录canvas connected=true、opacity=1、visibility=visible、displayNone=false；canvas与video同父且位于其后。canvas为absolute、zIndex=auto，video为static、zIndex=auto，记录的四层祖先opacity均为1。WebGL实际alpha=false、preserveDrawingBuffer=false、antialias=false、contextLost=false；desynchronized属性缺证。

媒体canvas矩形为800×450、x0/y59。video元素矩形800×568、object-fit=contain；两种源均为16:9，video内容的contain区域与canvas矩形一致。DOM和正常CSS层叠证据确认SR canvas覆盖媒体区域，video持续作为底层纹理供给。screenPresentState仍为unavailable，实际屏幕像素更新与用户观感的对应继续需要上屏事实。

## 网络、恢复与预算

采样区间接收15818包，归一正丢失31包，约0.196%；净丢失增量29。RTT中位212ms/P95222ms/max229ms，jitter中位2ms/P9531ms/max33ms。浏览器freeze增量23次/6.923s，framesDropped增量2；接收、解码和稳态供给约30fps。上游冻结可以影响计数推进和回调节拍，本地数量抵扣将进度偏移延续为持续候选缺口。

PLI增量2、FIR0、NACK544、关键帧解码增量6；各请求与具体修复结果的配对、ReferenceChain、cleanAnchorCommitted、DisplayStable和NACK有效率为unavailable。应用层恢复/重连0，正常退出资源闭环通过。

browser behavior与browserRuntimeStability门禁PASS，sharedSamples=8；生命周期PASS。既有门禁验证资源、样本共享及预算边界，本轮帧身份证据进一步确认了供给闭环的具体待修项。trace在途最大3、排队0、丢失0；getStats成本中位1ms/P952ms/max40ms。

## 离线对照与下一步

[偏移复现脚本](2026-10-29-sr-counter-offset-replay.cjs)加载现有生产模块，GPU/DOM使用此前替身。两组输入的native呈现计数与RTP均逐帧推进，原始RVFC均30fps、300次：计数对齐组保持30fps；播放计数在2s后增加48帧偏移，后8s接受32个marker，稳态4fps，224个native回调被筛掉，末次抵扣余额40。全部断言通过，结果见机器可读记录counterOffsetReplay。

建议继续在同一VideoFrameSource修正去重资格：有效RVFC帧身份承担新帧交付，轮询补足按已证明交付的同帧身份去重；为两路累计计数建立明确基线与有效范围，使进度偏移被诊断和结算。FramePacer、60fps用户预算、SR算法和网络接收帧率保持现有边界。完成该修正后，再验收30fps源的SR覆盖与实际可见输出。
