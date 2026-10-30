# 解码后渲染成本与源帧率保障审计

2026-10-30，承接ROG本地1440p串流接收/解码60fps、渲染约40fps及关闭SR后84%绘制提交率的用户观察。本轮审查当前浏览器解码→HTMLVideoElement→共享帧源→WebGL/SR→canvas提交路径，读取生产代码并回放纯策略函数。设备操作保持暂停。

## 结论

共享帧源、latest-only、累计截止时间、源像素下限和资源复用的基础设计支持低延迟。增强成本测量与健康准入的闭环仍需完善，当前实现可以把明显的持续帧率损失分类为renderStable。近期抵扣修正解决候选形成阶段的故障；纹理上传、GPU处理和提交阶段的工作量仍按现有路径执行。

## 成本分布

| 路径 | 每个实时新帧的主要工作 | 证据与影响 |
| --- | --- | --- |
| video呈现 | 浏览器video呈现及可选CSS/SVG滤镜 | NativeRenderer按USM配置安装卷积滤镜；纯原生基线需要将锐化和色彩设为中性 |
| 普通WebGL | 一次video→RGBA纹理更新，CAS或USM及颜色处理 | Renderers.ts:681上传；CAS中心+十字邻域共5个采样点，USM共9个；关闭SR仍由auto优先选择WebGL |
| 普通WebGL尺寸变化 | 源尺寸锐化FBO，再映射至输出，合计两次绘制 | Renderers.ts:704；保证锐化核按源像素取样，sharpness=0时也按尺寸差异进入此路径，存在优化空间 |
| SR等尺寸 | 一次video→RGB纹理更新，一次RCAS绘制 | SuperResolutionWebGL2Renderer.ts:971；1440p源→1440p输出运行5点RCAS |
| SR放大 | 单pass融合shader，或EASU+RCAS两pass | 融合shader含17个源采样调用及EASU/RCAS算术，precision highp float；单pass省中间写回，实际速度依赖设备。路径优先选择编译成功的融合shader，GPU性能比较证据待补 |
| 截图 | 同步readPixels、大数组、逐行翻转与2D拷贝 | 按截图请求执行；1440p时raw/flipped两个数组约29.5MB，可能造成单次主线程长尾 |

1440p RGBA每帧容量14,745,600字节，60fps像素容量约0.885GB/s。这个数值用于估算纹理工作量；实际GPU/CPU复制、YUV→RGB转换与共享表面路径由WebView和驱动决定。应用API的video纹理更新属于需要单独测量的阶段。

稳态纹理与FBO复用，uniform仅在配置/尺寸变化时更新；能力getParameter绑定context创建，getError通常最多1Hz，FBO完整性检查绑定分配。逐帧计数是小规模数值工作。shader编译、canvas backing改变及大纹理创建集中在初始化/尺寸或管线切换，可能造成瞬时卡顿；SR初始化会同时编译融合、EASU和RCAS程序，等尺寸路径只执行RCAS。

## 健康判定缺口的可重复证据

直接加载当前resolveBrowserRenderCause函数，输入fresh、接收/解码60fps、目标60fps、提交年龄25ms、存在最新候选，结果如下：

| 候选fps | 提交fps | CPU提交均值 | 当前分类 |
| --- | --- | --- | --- |
| 60 | 50.4（84%） | 4ms | renderStable |
| 60 | 40 | 4ms | renderStable |
| 40 | 40 | 4ms | renderStable |
| 60 | 50.4（84%） | 20ms | renderStable |
| 60 | 40 | 20ms | renderStarvation |

原因位于browser-render-health.ts:23–31：提交停滞门槛至少180ms；持续提交不足采用低于候选/预算的80%且平均CPU成本超出帧间隔的组合；候选不足采用低于解码66%。40/60约66.67%，84%提交都可能落入稳定分支。BrowserRenderHealth以renderStarvation积累压力，稳定标签将影响增强回退和重试资格。

两类故障分别归因：候选不足继续处理共享帧源/回调交付；候选充分而提交不足继续处理renderer。持续GPU过载可以同时表现为CPU提交较低、GPU队列长尾和实时提交不足，当前CPU平均成本门槛对此缺少覆盖。

## 测量边界

RenderObservation的计时包围CPU侧上传和GL命令提交。它没有等待GPU完成，当前snapshot只提供均值与历史最大值，GPU耗时、compositor呈现和真实submit-to-present继续unavailable。60fps的帧间隔16.67ms应约束关键路径和持续吞吐；CPU/GPU可以流水重叠，预算分析需要结合阶段耗时、候选/提交增量与长尾。

旧Mac样本的9.94ms为720p下CPU提交均值；ROG的1440p驱动和GPU成本需要其同窗日志与设备阶段测量。用户当前接收/解码60fps支持把调查重点放在解码后，网络丢包/缓冲和ReferenceChain等恢复链健康仍需ROG日志事实。当前记录区分media supply、steady supply、候选交付和submit/present各阶段，避免用接收正常替代显示验收。

## 建议执行顺序

1. 优先修正健康判据：以同机同源纯呈现的稳定长窗为基线，分别判断候选/解码与提交/候选差值；稳定提交不足单独形成可观察状态，CPU/GPU成本支持原因分类和动作效果。建议增强相对基线的稳定帧损控制在1–2%，明确用户FPS上限与显示能力边界。
2. 补有界分段测量：纹理上传CPU耗时、GL提交CPU耗时、支持平台的异步GPU timer，以及候选等待、实际提交间隔的P95/P99；热路径只累计数值，低频快照取证。GPU计时缺证时继续使用完整窗口的提交差值与长尾结算。
3. 建立纯呈现→单pass锐化→实际需要的放大的性能阶梯。源=输出优先单pass；增强全中性时复用现有video或直接copy路径；sharpness=0的尺寸变化跳过多余锐化FBO。保持源尺寸下限、用户配置与当前媒体路径。
4. GPU成本持续超预算时保真回退，核验帧率和长尾实际恢复，健康长窗后一次增强试探；新增判断保留既有时间覆盖、动作间隔、证据缺失和用户意图保护。

建议优先处理健康判据与上传/GPU分段测量，再根据ROG同源对照选择算法优化。生产代码保持本轮起点，后续整改沿既有RFC确认方向执行。

[策略回放与逐文件SHA-256](2026-10-30-postdecode-render-cost-audit.metrics.json)绑定本轮代码。审查参考当前[策略设计](../../docs/isu/2026-10-29-browser-policy-stability-and-native-quality.md)及[执行RFC](../../docs/rfcs/2026-10-28-browser-runtime-simplification-and-stability.md)；原生源下限、网络/显示权限隔离与低延迟latest调度继续作为有效约束。
