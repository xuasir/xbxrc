# 播放计数配对与回调时序验证

2026-10-30，沿已授权S4帧交付取证推进。现有dev身份环已加入播放质量/WebKit计数配对、回调入口时序及主计数更新累计；36组生产模块回放和427项全量测试通过。受控输入确认逐帧计数可将45/50Hz RVFC补满60fps；250ms合批计数与native同步更新时仍为45/50fps。实际设备更及时计数、可见图像对应关系与保留SR效果的60fps继续取证。

## 改造范围

- [共享帧源](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/video-frame-source.ts)：保留当前主计数选择；复用每次供帧读取的播放质量原始total/dropped，备用WebKit仅在既有dev样本中读取。选中WebKit时复用已有读取，读取失败继续由有效native身份供帧。
- [身份观测](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/video-frame-identity-observation.ts)：`pairedCounters`记录两种原始total/dropped、扣除drop后的计数、上个配对样本时间和各自增量。`callbackEnteredAtMs`、`callbackIntervalMs`、`callbackTimestampIntervalMs`、`callbackClockLagMs`区分JS入口间隔与浏览器回调时间戳。
- `callbackTiming`分RVFC/rAF/timeupdate累计观察次数、实际入口间隔及时间戳滞后；`counterCadence`累计主计数更新次数、增量、更新间隔和更新入口归属。消费者中断清理时序基线，累计诊断保持原采集范围。schemaVersion=2新增可选字段，既有协议、renderer、限帧、抵扣与latest-only继续沿现有合同。
- 诊断首次读取才启动，接受/拒绝各至多2Hz、8条证据、固定数量累计状态；备用计数额外读取合计至多4Hz。dev回调入口每次增加一次单调时钟读取；off/production的额外时钟/备用计数诊断读取为0。设备额外CPU成本继续由新日志检验。

计数配对发生在同一次回调观察，主计数复用入口读取，备用计数读取在采样分支内；`pairedCounters.observedAtMs`表示配对完成时间。原始total/dropped分别记录，便于区分计数推进、drop修正与基准回退。抽样增量描述两个配对样本之间的变化，逐帧可见图像身份仍需要RTP/native计数及回调时序联合取证。

## 生产模块回放

[回放脚本](2026-10-30-browser-frame-counter-pairing-audit.cjs)直接加载当前帧源、限帧与两种WebGL renderer，沿用已有DOM/WebGL受控环境。9种输入×2种renderer×诊断off/dev，共36组，每组600个源图像/10秒、60Hz刷新。导入时记录当前源图像身份，分别检查候选和导入图像唯一性；[metrics](2026-10-30-browser-frame-counter-pairing-validation.metrics.json)保存全部结果、8条配对证据、累计时序、源码与脚本SHA-256。

| 受控供给 | native回调/10秒 | 轮询候选/10秒 | 实际唯一图像处理/10秒 |
| --- | ---: | ---: | ---: |
| 逐帧播放计数、45Hz native | 450 | 150 | 600 |
| 逐帧播放计数、50Hz native | 500 | 100 | 600 |
| 250ms合批、45Hz native、批次先到native | 450 | 0 | 450 |
| 250ms合批、50Hz native、批次先到native | 500 | 0 | 500 |
| 250ms合批、45Hz native、poll先到 | 450 | 41 | 460 |
| 250ms合批、50Hz native、poll先到 | 500 | 41 | 520 |
| 250ms合批、45Hz native、备用计数逐帧更新 | 450 | 0 | 450 |
| 250ms合批、50Hz native、备用计数逐帧更新 | 500 | 0 | 500 |
| 逐帧60Hz供给、用户30fps上限 | 600 | 0 | 300 |

两种renderer结果一致，诊断off/dev的native数、候选、轮询、提交、唯一图像和latest覆盖数逐项一致。dev每组备用计数读取40次、证据上限8；off读取0。所有组候选图像与纹理导入图像均唯一，提交失败/重复绘制为0；主动30fps上限的覆盖计数按正常限帧解释。

批次先到native的输入专门保持批次时刻存在native回调，同时在其他帧保留精确45/50Hz回调数；该输入复现新日志中主计数增量在native入口被消费、rAF接受0。poll先到的输入增加少量真实当前图像机会，晚到native按已有额度去重；41个轮询交付包含对native候选的替代，实际总增益分别10/20帧。

逐帧备用计数场景仅扩展诊断，当前供帧仍使用播放质量主计数；日志可以观察备用计数相对主计数领先的事实。受控备用计数与当前图像一一对应由模拟输入定义，设备上的WebKit解码计数与当前video可绘制图像的关系继续缺证。每批15帧推进提供一次当前图像读取机会，历史图像的独立处理资格需要独立帧接口。

## 验证结果

- 新增4项回归：配对读取及原始drop计数、主读取复用、dev采样预算/关闭诊断零额外读取、真实入口与时间戳间隔、计数批次、消费者暂停后重建基线、快照深拷贝、无效/异常备用计数。定向3文件99项通过。
- `pnpm lint:fix`、`pnpm exec vue-tsc --noEmit`、`pnpm test`（48文件/427项）、`pnpm build`（269模块）和`git diff --check`通过。既有off/production/dev媒体诊断边界回归继续通过。
- 原9组供给/4组绘制路径、3组48帧偏移和20组抵扣/累计限帧前后对照全部通过。历史20组基线使用已保存源码，原结果保持可复算。

736个源码输入的当前fingerprint为 `62536906cc72eb9433808b156885a591845155a8e92f57d5e024e8693f820a65`。本轮完成前端生产构建及离线验证，自动实机操作保持暂停。

## 实机验收与下一步

[16:09日志](2026-10-30-frame-credit-window-and-pacing-fresh-trace-validation.md)继续作为设备基线：SR候选→提交100%、约49fps；完整解码约60fps、最终原生video完整15秒窗推进59.975fps；上传均值0.490ms、提交年龄P9570ms/max116ms。网络丢失0.04914%、RTT中位15ms，1次1.414秒freeze包含1009ms收包零推进。浏览器内部ReferenceChain/cleanAnchorCommitted/DisplayStable、NACK有效率、GPU/零拷贝与真实submit-to-present继续缺证。本批代码效果采用新构建配对日志验收。

建议新构建持续前台60–120秒，固定1440p/60fps源并打开诊断面板，记录SR开启及原生video阶段和正常退出。按同一renderer/完整3秒与15秒窗口联合读取：

1. `counterCadence`增量识别主计数更新频率、批次大小和native/rAF消费归属；累计max包含整个采集期，前后台通过政策边界分段。
2. rAF拒绝时的配对计数及此前native/RTP身份判断备用计数是否更及时，以及该增量是否对应新图像；抽样计数的领先量与两种计数基准分别解释。
3. 回调实际入口间隔、时间戳滞后及metadata预计显示时间定位交付长尾；源帧、候选、成功处理delta与最新提交年龄共同验收。

复算：`node dev-docs/validation/2026-10-30-browser-frame-counter-pairing-audit.cjs <output.metrics.json>`。下一步以新配对日志确认可靠逐帧身份，再选择当前帧源内的补供给方案。
