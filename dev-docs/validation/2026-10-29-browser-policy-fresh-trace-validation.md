# 浏览器策略新构建日志验证

2026-10-29：当前 Mac cloud 样本验证了真实源尺寸下限、生命周期暂停/恢复、动作域隔离与正常退出资源清理。有效前台短窗仍有 video advancement → callback → renderer submitted 的供给缺口，完整稳定性证据为 MISSING，S4 继续验收。

依据：[成果计划](../../docs/isu/2026-10-29-browser-policy-stability-and-native-quality.md)、[实施 RFC](../../docs/rfcs/2026-10-28-browser-runtime-simplification-and-stability.md)。数值与行锚点见[机器可读验证记录](2026-10-29-browser-policy-fresh-trace-validation.metrics.json)。

后续版本说明：用户追加的标准 SR 目标与最新帧供给修正已完成[代码与自动验证](2026-10-29-browser-standard-output-and-latest-frame-validation.md)。本文日志及源码匹配结论对应修复前分析版本，作为后续配对基线保存；新代码运行效果需绑定新构建日志。

## 日志与构建身份

来源目录：`/Users/guo.xu/Library/Logs/com.xuasir.xbxrc/runtime-logs/`。

| 日志 | 时段（北京时间） | 范围 |
| --- | --- | --- |
| `runtime-trace-1793254996687-1.jsonl` | 14:23:16.687–14:24:01.048，44.361s | 空闲宿主，107行，串流 operation 为0 |
| `runtime-trace-1793255066737-1.jsonl` | 14:24:26.737–14:30:08.559，341.822s | 1次 cloud 串流，1,796行；浏览器媒体采样130.869s |

两份日志均为 schema v3 / dev / Mac debug，Git revision 为 `bec3acfb`、workspaceDirty=true。日志构建源码 SHA-256 与按 `src-tauri/build.rs::source_fingerprint` 算法复算的分析时726个源文件完全一致：

```text
fac1d02f956893c249f1833739e7a009222075d7337d070a2b517be0366202f8
```

主日志使用 H264 constrained-baseline，answer 的 `profile-level-id=42e02a`。用户确认测试期间频繁切换窗口或应用。媒体采样区间内前台41.111s、后台89.758s，前台拆为22段，最长9.319s，连续前台15s窗口为0。

## 门禁结果

| 检查 | 结果与证据 |
| --- | --- |
| 浏览器基础行为 | PASS；本地/远端SDP、connected、首入包、首解码、首关键帧解码与首video呈现齐全；connected→首video呈现约1,149ms |
| 公共生命周期 | PASS；两份文件合并，要求匹配revision/源码摘要、完整cleanup与closed；1个operation、1个session resource、5组cleanup，violation与evidenceGap均为0 |
| 浏览器完整稳定性 | MISSING，唯一缺项为`sharedSamples`，内部failures为0；268个采样、46个新鲜监督观察、45个实际尺寸观察、40个五段帧链样本。面板consumer观察为0 |
| 严格behavior入口 | `--require-stability` 返回FAIL / exit 2，原因为上述共享样本缺证；完整门槛继续保留 |
| 空闲首文件 | 按空闲宿主范围检查；其浏览器媒体门槛缺少SDP与媒体样本，由主文件承担串流验收 |

当前媒体路径为浏览器 WebRTC / WebView 解码，Rust原生视频的ReferenceChain、cleanAnchorCommitted、DisplayStable和真实scanout取证继续按平台可观测边界记录为unavailable。

## 最小时间线

| 行锚点（主文件seq） | 事件与结果 |
| --- | --- |
| 363 / 389 | 14:27:19.035开始启动；14:27:27.628创建cloud session |
| 430 / 437 | 14:27:41.287进入session执行；14:27:41.793绑定浏览器runtime启动入口 |
| 497 / 506 / 507 | 14:27:49.713连接；14:27:50.857首解码与首video呈现 |
| 504 / 505 | 实际1440p源，SR为passthrough，计划与输出2560×1440；真实视口较小时仍保留源尺寸 |
| 674 / 678 | 49.35s隐藏段后恢复；135ms内提交SR首帧，真实GPU backing为2560×1440，renderer generation=1 |
| 711 / 713 / 715 | 14:28:35.340源尺寸重评；WebRTC随后采样确认实际源1280×720，输出1600×900，沿同一renderer更新 |
| 946 / 1219 | 720p源、SR upscale、五段各有完整3s覆盖的前台样本；回调交付缺口持续存在 |
| 1772 / 1773 / 1783 | 14:30:02资源归零并完成runtime stop；14:30:03.877远端close返回closed=true |

## 画质下限与策略归属

45次FPS预算变化包括1次初始60和22次隐藏/22次恢复，前台目标恒为60、隐藏预算为0。48次render plan应用由44次生命周期变化、2次真实源尺寸冻结、1次容器尺寸与1次刷新率估算组成。全程使用同一SR renderer generation。

45个有效尺寸观察均保持当前真实源的宽高下限。1440p阶段实际backing为2560×1440；实际源变为720p后，显示需求1600×900承担上采样。应用层quality、renderer软回退、局部恢复和连接重建动作为0；SDP协商始终只有初始一轮。

这些字段支持源尺寸变化与本地处理尺寸分别归因：传输采样确认源由1440p变为720p，本地输出始终保持当时实际源下限。远端降源的内部决策及SR感知画质继续以服务/同源画面对照补证。

## 五段供给与提交长尾

筛选条件：当前renderer generation=1、实际720p源、同一完整3s窗内received/decode/video/callback/renderer覆盖均≥2999ms，并且整个窗口落在一次连续前台段内。得到14个样本，其中部分3s窗口重叠；下表为各段窗口均值的样本中位数。

| 阶段 | fps |
| --- | ---: |
| received | 29.45 |
| decoded | 28.76 |
| video/compositor advancement | 25.98 |
| callback delivery | 15.28 |
| renderer submitted | 13.48 |

按每个同窗样本先求差再取中位数：received→decoded为0.00fps、decoded→video为2.33fps、video→callback为11.94fps、callback→submitted为1.80fps。主要缺口位于video推进到RVFC回调交付，随后还有latest覆盖/节拍产生的提交差。当前证据支持持续回调稀疏/合批；WebView调度、主线程与GPU/合成器各自贡献继续通过连续前台配对测量。

14个恢复后至少3s的前台telemetry采样中，renderer提交年龄中位213.5ms、P95 497.55ms、最大597ms。9个可得CPU提交间隔均值样本的中位6.25ms、P95 7.67ms；该分布描述采集间隔内均值，逐帧CPU/GPU及真实上屏P95/P99继续待测。

trackingSource均为videoFrameCallback；46次源节拍估算均缺证（mediaTimeDeltaTooSmall）。全局RVFC gap计数344、合批计数914，包含后台区间。前台14个telemetry窗口的video advancement/callback count约1.78；源FPS缺证继续保持用户60fps预算。

最后可得renderer计数：实时提交551、latest覆盖72、pacing wait97，重复实时处理、提交失败、截图额外绘制均为0。累计浏览器freezeCount=186、freezeDuration=67.496s，覆盖130.869s媒体采样中的后台/前台，连续前台稳态冻结需要单独样本核验。

## 网络、恢复与观测

媒体采样区间RTT中位234ms、P95 281ms、最大334ms；jitter中位13ms、P95 32ms。增量4035 lost / 52839 received，按lost/(lost+received)得到7.09%丢包；平均视频接收3441kbps，jitter buffer delay / emitted frame约323.94ms。当前路径存在真实丢包与缓冲积累，高RTT路径时延另有稳定基线。

浏览器PLI增加3、NACK增加7334、重传包增加6496、关键帧解码增加7；请求消息与重传包分别计数，NACK有效率、恢复锚点/显示闭环的浏览器内部证据为unavailable。应用层恢复调用为0，此批有界恢复动作与健康15s关单效果继续待受控故障/连续前台取证。

30/268个采样缺少selectedCandidatePair。部分完整3s帧窗内loss/RTT路径覆盖仅约2.75s，观测缺证资格如实保留。按本批运行状态，客户端保持初始计划与已有平台WebRTC控制。

getStats采集耗时中位1ms、P95 3ms、最大26ms；前台约250ms节拍、后台约1s节拍，采集重叠与频率门槛通过。trace采样最大IPC在途2、排队0，debug/essential丢失和Rust writer预算通知均为0；末次GPU周期查询计数44。

clientClosed的video/audio element、当前/候选renderer、RVFC、frame/capture waiter、ready timer、mic track/permission、audio context/node、button timer全部归零。堆/RSS和设备GPU系统回收由2h/8h长测验证。

## 分析工具校准与后续取证

本轮修正`browser_runtime_stability_report.py`的帧率窗口语义：`windowMs/rateScope`指采集间隔，`policyWindowMs/windowFps/windowCoverageMs`分列策略窗均值与各段覆盖。原有五段瞬时数值继续保留，3s窗口数据单独解释；17项Python正负/集成fixtures全部通过。

S4优先补一组当前构建连续前台60–120s样本：打开诊断面板取得panel/监督共享样本，维持相同源、网络、显示设置做SR关/开配对，正常退出。以五段帧差、回调间隔、CPU/GPU提交、提交年龄和实际画质判断帧供给；主机媒体、Windows/ROG、受控恢复与2h/8h仍保留原验收范围。
