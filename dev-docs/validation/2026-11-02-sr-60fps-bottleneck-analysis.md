# SR 60fps 供给瓶颈续查

2026-11-02。当前证据把 SR 的主要帧率缺口定位到浏览器解码后、共享帧源候选交付前的边界。两段 SR 同代际实时区间中，rAF 观察约58.8/60.1Hz，RVFC 约52.6/54.8Hz；rAF 新候选接受均为0，候选到提交为263/264和165/165。更深层的 drawable 更新、浏览器 metadata 更新、合成调度及 GPU 等待，需要独立设备事实继续分解。

## 范围与复算

已核对 macOS 应用日志、仓库 `runtime-logs` 和临时回退目录，最新可用文件仍是2026-10-30 17:43:02采集的 [runtime-trace-1793353382323-1.jsonl](/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/runtime-trace-1793353382323-1.jsonl)。2829行、204.115秒，媒体统计窗口136.286秒；737个源码输入匹配，日志SHA256为 `03d8b97c9ebe55a87b5accfe23c16c9551f7563f04c7445e0a777d90546249d2`。

本轮接续关联聊天中断的“SR为什么不能跑满60帧”分析。复跑已有网络/窗口分析与36组生产模块回放，增加同代际回调节拍差、回调入口时钟差及诊断启用边界的独立核对。[原完整报告](2026-11-02-network-render-fresh-trace-analysis.md)保留网络、生命周期和浏览器观测字段的详细结果。本次SR为2560×1440等尺寸 `passthrough`，锐化0，实际放大效果路径另需设备验收。

## 同代际证据

下表采用实时 `mediaDiagnostics` 首末差，覆盖每段相同renderer与前台状态。它与完整3秒供给窗口各自结算，缓存telemetry字段与实时诊断各自保留其采样时点。

| SR区间 | 原始seq | 时长 | rAF观察 | RVFC观察 | rAF新候选 | 新候选→成功提交 | 成功提交fps |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| g1，诊断启用后 | 243→352 | 5.017s | 295次，58.800Hz | 264次，52.621Hz | 0 | 264→263，99.62% | 52.422 |
| g3，增强试探 | 957→1030 | 3.012s | 181次，60.093Hz | 165次，54.781Hz | 0 | 165→165，100% | 54.781 |

完整3秒供给窗口中，g1共4个，解码/候选/提交中位数60.231/51.830/51.496fps；g3共1个，60.492/54.254/54.254fps。同窗解码→提交差中位数9.044/6.238fps。两段SR的完整15秒窗口均为0，长期足帧继续待验。最终恢复前台的video有56个完整15秒窗口，推进中位数59.942fps。

共享帧源在诊断启用后的全会话累计6922次rAF观察、接受新候选0。播放质量计数更新471次，其中462次在RVFC入口、9次在rAF入口；video/g3/恢复前台video的同代际更新平均间隔258.23/260.55/258.63ms。g1累计区间包含诊断启动前的14,594ms左侧缺证，单独保留这一边界。

g1/g3的回调时间戳间隔均值为：rAF 17.048/16.674ms，RVFC 18.882/18.279ms。RVFC时间戳到JS入口的平均时钟差0.015/0ms；该字段测量浏览器回调时钟到入口的差，源帧等待和预计显示点晚到分别由其他证据评价。有界样本的 `expectedDisplayLeadMs` 中位数−5/−13ms，表示回调相对浏览器预计显示点晚到。

## 代码为何形成这一上限

- [video-frame-source.ts](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/video-frame-source.ts:171)持续注册RVFC与rAF，统一判定当前帧身份。播放计数晚到时先抵扣已交付native推进；计数批量更新和观察机会之间的相位决定rAF能否取得独立候选。
- [候选交付](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/video-frame-source.ts:334)对一次新观察建立一个metadata对象、递增一次localSequence并通知消费者。`presentedFramesDelta > 1`表达计数推进量，当前HTMLVideoElement提供当前图像；批次中历史图像需要另有可访问的帧对象才能逐张处理。
- [SR候选处理](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/SuperResolutionWebGL2Renderer.ts:567)收到新候选立即尝试绘制，持有一个latest候选。[FramePacer](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/frame-pacer.ts:14)以累计截止时间执行用户上限，rAF承担已有候选的重试。60fps目标表达允许的处理预算，额外绘制机会仍需独立新图像。

本次g1/g3的少量latest覆盖和pacing等待集中于局部交接；g1诊断后的帧损失为1，g3实时区间为0。这些损失的规模与每秒6–9帧的解码→提交差分别结算。当前观察窗口中rAF已有接近60Hz的机会，下一步优先查清其间drawable图像和metadata的独立更新。

## 生产模块回放

重跑现有 `2026-10-30-browser-frame-counter-pairing-audit.cjs`，直接加载当前生产帧源、renderer和FramePacer。每组10秒、源图像600张，逐次检查实际导入图像身份；36组全部通过，off/dev的候选、提交和唯一导入图像计数一致。

| 输入条件，SR路径 | native回调 | rAF补帧 | 成功提交/唯一导入图像 | 输出fps |
| --- | ---: | ---: | ---: | ---: |
| 逐帧主计数，native45Hz | 450 | 150 | 600/600 | 60 |
| 逐帧主计数，native50Hz | 500 | 100 | 600/600 | 60 |
| 250ms合批计数，native45Hz，native先观察 | 450 | 0 | 450/450 | 45 |
| 250ms合批计数，native50Hz，native先观察 | 500 | 0 | 500/500 | 50 |
| 250ms合批计数，native50Hz，poll先观察 | 500 | 41 | 520/520 | 52 |
| 逐帧主计数，native60Hz，用户上限30fps | 600 | 0 | 300/300 | 30 |

回放证实当前代码在逐帧主计数条件下能交付600张唯一图像。合批计数条件下，其补帧机会受计数更新时序约束；改变先后相位可增加部分交付。假设更快WebKit备用计数的回放仍主要跟随native，因为当前备用读取承担低频诊断；设备466组配对的备用计数有效样本为0。真实设备的可绘制图像身份、GPU和上屏仍采用独立证据验收。

## 网络、恢复与渲染成本

媒体统计新增接收8169帧、解码8170帧，平均59.940/59.947fps；正包丢失133/333422=0.039889%，RTT P95=17ms、jitter P95=7ms，冻结、解码丢帧和暂停增量均为0。前台接收/解码59.943/59.967fps。网络和解码供给覆盖当前SR目标的源预算。

919次首提交后的成功处理，CPU同步调用均值0.491ms，其中上传0.457ms、GL命令提交0.029ms。两次冷首处理19/22ms，同尺寸SR重建34ms；这些成本单独结算为初始化/首处理开销。实时最新提交年龄P50/P95/P99/max为11.5/28.15/32.83/34ms。GPU执行时间、submit→present、glass-to-glass及物理上屏长尾继续缺少直接证据。

浏览器恢复计数：首样本已有PLI2、关键帧解码3；媒体窗口新增PLI0、FIR0、NACK455、关键帧解码4、RTX包472。逐请求抑制/合并、usable response、NACK及时/迟到/跳过/过期、建链成功率、repairability连续性和综合恢复评分均为unavailable。按固定顺序审计，`pictureRecoveryTransition`、`pictureRecoveryBlockerObserved`、`videoIngressTermination`、`firstFrameLatencyObserved`及H264内部事件均为0；ReferenceChain、cleanAnchorCommitted和DisplayStable缺少浏览器内部事实。

默认receive报告PASS且反馈决策样本0；严格必需媒体/显示闭环检查退出2，原因为证据MISSING。native midsegment的media/steady supply为SKIPPED，年龄为n/a；本报告采用browser同窗供给判断实际播放。browser行为与runtime稳定性PASS，24组共享样本通过。trace为dev，budget notice与前端dropped均为0。

`trackingSource`全部为videoFrameCallback；117个telemetry的sourceFpsEstimate因mediaTimeDeltaTooSmall缺证，源节拍以RTC增量评价。g1/g3最大回调间隔112/49ms，callback gap 1/0，多帧跳跃与dropped-like各422/19；这些数值表达浏览器回调合批。独立renderFrameDropped、renderBackpressureChanged、renderCauseClassified事件均缺样本，持续backpressure和droppedLikeStreak待补；归因投影为frameDeliveryLimited110、renderStable7。

## 诊断启动前的30fps

seq184→243，首提交40,550ms至诊断启用55,108ms共14.558秒，候选与提交均新增436，即29.949fps；相近RTC窗口解码59.981fps。首telemetry累计437次回调、875帧计数推进、391次多帧跳跃，rendererTargetFps=60，latest覆盖与pacing等待均为0。该阶段已经形成候选节拍约30Hz的事实。

seq243的像素readCount仍为0，seq266首次观测到readCount1、总耗时24ms，超过8ms预算后停止，有效配对0。30fps阶段发生在像素探测读取之前；24ms首次读取可影响局部采样后的调度。seq250在单调55,492ms记录refreshRateEstimateChanged，targetFps继续为60。诊断启用、窗口/DOM变化和刷新率估计共同出现于这个边界，具体设备原因继续由受控对照判断。

相同输入的36组回放中off/dev交付一致，支持当前诊断代码对供给合同保持一致。实际页面采样、窗口焦点/遮挡、系统节能、合成器刷新及GPU等待的影响需要记录各自状态。首段诊断启用前的rAF节拍缺证；后续两段rAF约60Hz与RVFC约53–55Hz分别提供独立判断窗口。

## 后续执行顺序

1. 在现有dev像素探测中分段记录canvas/context初始化、drawImage和getImageData成本，保留8ms慢读停止与低频预算，取得有效的RVFC缺席期间像素配对。运动画面下的像素变化可证实drawable更新；缩略像素相等继续按有限证据解释。
2. 用同主机、同源、相同运动场景和持续前台条件，固定诊断采样预算，记录诊断启用、窗口焦点/可见性/遮挡、刷新与系统节能状态，复验30→53fps边界。将rAF机会、RVFC计数/时间戳、drawable更新和提交分别对齐。
3. 根据drawable事实选择现有共享帧源内的补供给策略，再验收保留SR效果的60fps、完整3s/15s供给和真实上屏长尾。当前流畅播放优先使用原生video；S4维持In Progress，实机采集继续由用户手动执行。

[复算指标](2026-11-02-sr-60fps-bottleneck-analysis.metrics.json)保存同代际时钟、170个完整窗口核对、36组图像身份回放和代码指纹。[复算脚本](2026-11-02-sr-60fps-bottleneck-analysis.py)：

```sh
python3 -B dev-docs/validation/2026-11-02-sr-60fps-bottleneck-analysis.py
```
