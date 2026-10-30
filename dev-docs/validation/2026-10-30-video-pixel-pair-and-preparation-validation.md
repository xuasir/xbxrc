# 可绘制像素配对与renderer准备分段验证

2026-10-30，沿已授权S4取证范围完成共享帧源的低频视频像素配对，以及renderer准备、首候选、首成功处理和交接分段。24组生产模块回放确认诊断off/dev的候选、处理与唯一图像数量一致；437项全量测试、lint、类型和生产构建通过。实际设备上的可绘制图像提前推进、SR60fps和呈现延迟继续使用用户手动新日志验收。

## 交付范围

- [像素观测](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/video-frame-pixel-observation.ts)由[共享帧源](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/video-frame-source.ts)的既有dev诊断开启。在有效native入口读取16×9缩略图，随后在8–50ms内、RVFC回调缺席、候选序号/主计数/源尺寸一致的rAF入口读取第二份。每组至少间隔500ms，通过500–602ms间隔扫描刷新相位，最多2组/秒、每组2次读取。连续native会结束上一配对，新的基线继续受采样预算限制。
- `frameIdentity.videoPixels`记录基线/有效配对/变化配对次数、失效原因、读取总耗时/max和最近8组时序及RGB差异统计。每次回读576字节，原始像素只保存在内部单个基线；日志记录`changedPixelCount`、`maxChannelDelta`和`absoluteRgbDeltaTotal`。离屏canvas承担诊断读取，显示renderer继续使用原WebGL纹理。
- API失败、context缺证或单次读取返回后耗时超过8ms时，停止该video后续像素探测并释放离屏资源。隐藏、暂停、媒体未就绪和最后消费者取消时清理配对及canvas；native回调、计数/序号改变、尺寸改变和超过50ms按各自原因结束配对。低频资源快照提供`pixelProbeCanvases`、`pixelProbePendingPairs`，正常退出均归零。
- [处理观测](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/render/render-observation.ts)的`resources.preparation`保存准备开始/结束、首候选时间/序号、首成功实时处理开始/提交时钟；普通WebGL和SR都沿现有初始化及候选路径记录。截图单独记账；当前processor的首次事实持续保留，context恢复维持原归属。
- [播放资源](/Users/guo.xu/Documents/code/games/xbxrc/src/player/app/media/PlaybackService.ts)的`lifecycle`保存请求时源尺寸/候选序号、attach调用开始/返回、异步完成、首帧ready通知、commit开始、旧renderer释放及commit完成时钟。当前/待提交资源随原快照投射，失败资源释放前保存`failedLifecycle`与`failedPreparation`。native video的ready与媒体真实首帧独立解释。

像素采样沿首次dev诊断读取启动；从off/production开始且没有启动dev采集的video观察期间，额外canvas与像素回读均为0。每个renderer的初始化和首候选只增加有限的单调时钟读取，资源快照继续读取数值。供帧选择、抵扣、latest-only、限帧、硬故障回退与网络运行时沿当前合同。

## 证据语义与成本

有效配对中的RGB变化证明两次采样之间video的可绘制缩略像素发生变化；同一native回调数量、同一候选序号与同一主计数由配对资格保证。相等样本表示缩略像素相等，静态画面和缩略采样覆盖范围都影响观测；`pairCount=0`表示严格时序资格缺少有效配对。Canvas2D当前图像、WebGL当前导入图像与物理屏幕呈现分别验收，逐帧身份和GPU/零拷贝继续缺证。

Canvas2D读取可能触发浏览器内部转换或GPU同步，16×9回读尺寸与原视频读取成本分别解释。`readDurationTotalMs/max`覆盖canvas/context创建、drawImage和getImageData；8ms阈值在当前读取返回后阻止后续探测，首次慢调用仍进入成本记录。正常媒体帧率、候选/处理delta与像素读取时间联合读取，避免把诊断诱发的局部停顿归到原播放路径。

准备开始→结束描述初始化经过时间：普通WebGL包括setup的await返回，SR包括同步setupGl。attach调用开始→返回描述当前同步调用耗时，返回→完成描述异步/微任务等待。准备结束→首候选、首候选→首成功实时处理和旧renderer释放区间各自保留原始时钟；首帧截图或ready通知与首次实时处理可能采用不同资格。首次draw里的延迟编译/源变化由首处理耗时和上传分段联合解释。

## 回放与回归

[生产模块回放](2026-10-30-video-pixel-pair-and-preparation-audit.cjs)直接加载共享帧源、FramePacer与两种renderer。每组600个源图像/10秒、60Hz刷新；6种输入×2种renderer×诊断off/dev共24组。[metrics](2026-10-30-video-pixel-pair-and-preparation-validation.metrics.json)保存逐组计数、像素证据、准备时钟、源码/脚本SHA及737输入构建指纹。

| 每种renderer，dev，10秒 | 处理fps | 像素读取 | 有效配对 | 变化配对 |
| --- | ---: | ---: | ---: | ---: |
| 250ms合批/50Hz native，运动图像 | 50 | 20 | 2 | 2 |
| 同上，静态图像 | 50 | 20 | 2 | 0 |
| 同上，每次读取成本0.25ms | 50 | 20 | 2 | 2 |
| 250ms合批/50Hz native，poll先到 | 52 | 34 | 16 | 16 |
| 合批/50Hz native，1秒隐藏观察 | 50 | 21 | 4 | 4 |
| 逐帧60Hz供给，用户30fps上限 | 30 | 18 | 0 | 0 |

两种renderer结果一致；off/dev逐项检查候选、轮询接受、实时处理和唯一图像完全一致。纹理导入图像与候选图像均唯一，提交失败/重复处理0，原生回调/rAF及诊断资源结束后归零。0.25ms采用受控时钟成本；实际读取耗时由新设备日志验收。50Hz同步合批输入的供给继续50fps，取证结果保持当前交付行为。

新增10项回归覆盖：计数不变/缺RVFC时的独立像素变化与静态相等、off额外读取0、读取预算与证据上限、snapshot深拷贝、native/计数/尺寸/年龄失效、隐藏恢复、API/context/慢读取关闭，以及两种renderer首候选/成功处理和Playback异步attach/旧资源释放分段。定向3文件109项、全量48文件437项通过；`pnpm lint:fix`、`pnpm exec vue-tsc --noEmit`、`pnpm build`（270模块）和`git diff --check`通过。

既有9组供给/4组绘制路径、3组48帧偏移、36组播放计数配对和20组抵扣/节拍前后对照通过。节拍回放使用原metrics保存的精确基线模块。冗余绘制回放的历史HEAD来源已改为原报告保存的commit `eeaa5bb7f393f980251fd0ac6248bae5fb537f9a`，三个before源码SHA逐项匹配后执行，4组当前绘制结果继续通过；历史构建和当前构建各自固定。

## 新日志验收

[16:52日志](2026-10-30-browser-frame-counter-pairing-fresh-trace-validation.md)继续作为设备基线：接收/解码约60fps、正常SR完整窗44–50fps、五段候选1094→处理1090、最终video完整15秒窗推进59.933fps。同1440p上传0.477ms，正常SR提交年龄max32ms、网络停顿段max370ms。丢失0.31870%、5次freeze共2.516s、同一inbound/SSRC突增516包丢失和缓冲积压保留锚点；ReferenceChain/cleanAnchorCommitted/DisplayStable、NACK有效率、实际GPU与submit-to-present继续缺证。

新构建fingerprint为`c4398f8831ce4df5211588847608f4369c53cc338ef93b9a1851b21325f3714c`（737输入）。用户手动保持1440p/60fps源、前台运动画面并开启dev诊断，记录60–120秒SR/video阶段、一次尺寸变化及正常退出；自动实机操作继续暂停。分析顺序：

1. `videoPixels`有效配对/变化配对、8–50ms资格、失效原因与读取成本；成本超限/配对不足保持缺证，有效变化支持继续评估当前video提前读取机会。
2. 同renderer的`lifecycle`和`preparation`拆分之前467ms准备/等待及相邻回调停顿；每个阶段按自己时钟比较，回调稀疏与供给停顿继续读接收/解码事实。
3. 完整3s/15s接收、解码、video推进、候选/处理delta及提交年龄；网络突发丢失/缓冲长尾、恢复链配对、GPU/真实呈现和平台/长测继续单独验收。下一步根据设备独立像素证据确定补供给方向，S4保持In Progress。

复算：`node dev-docs/validation/2026-10-30-video-pixel-pair-and-preparation-audit.cjs <output.metrics.json>`。
