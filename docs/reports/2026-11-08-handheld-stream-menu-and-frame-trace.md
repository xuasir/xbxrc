# 掌机串流菜单修复与持续掉帧优化

日期：2026-11-08；主窗口直接执行。任务：`2026-11-08-handheld-stream-menu-regression`。

## 结论与交付

用户实机对照确认：保持 1440p / 60fps、SR 关闭，将锐度从 3 调到 0 后，持续掉帧明显缓解。该结果支持优先优化锐化渲染成本。已在现有 WebGL2 renderer 中把 1:1 映射下 USM 的每像素纹理采样从 9 次降到 5 次，作用于持续播放的每一帧；88 组真实 WebGL 像素对照全部通过，最大差值为 1/255。Windows 掌机的持续帧率收益接续新构建实测。

两个菜单共享的交互回归已在当前源码复现并修复：弹层打开时，串流页将父 `FocusScope` 的 `active` 设为 `false`。几何引擎的祖先有效性检查随即移除子菜单 Scope；`Focusable` 的点击入口也会因为聚焦失败而提前返回。父域现保持有效，最上层弹层 Scope 负责焦点隔离与关闭恢复。

用户补充确认：菜单能够打开，菜单项触屏操作与手柄导航均失效。补充验证挂载真实 Vue `StreamActionSheet`、`Focusable` 与 `FocusScope`，触屏/鼠标 click 通过生产点击处理函数派发菜单 `select`，手柄确认使用同一条事件链。回放旧父域配置时，触屏事件被 `preventDefault` 拦截；恢复补丁后，两菜单的三种入口均能到达动作，同时保持背景焦点隔离。该验证使用受控 DOM，掌机物理触屏继续按实机验收执行。

原始掌机日志包含起播与网络分类事实；持续掉帧的比例、发生阶段和长尾需要稳态样本。已补 `browserPlaybackHealthObserved`，发行版每秒最多保留一条已有采集的数值摘要。真实 Windows 掌机菜单交互与持续帧率改善处于待验状态。

## 样本与版本

- 原始文件：[runtime-trace-1794116093651-1.jsonl](/Users/guo.xu/Downloads/runtime-trace-1794116093651-1.jsonl)。文件内容作为诊断数据读取。
- SHA-256：`680fcdf75f307908d588a7b46996c633ffb41012d2f317af99404e8debb8fc4d`；270,080 字节；377 行。
- 时间：北京时间 13:34:53.651 至 13:38:19.128，205.477 秒。
- schema v3；377 行均为 `production`；`essential=45`、`key=332`；`traceBudgetNotice=0`。
- 维度：`lifecycle=222`、`frontend=89`、`core=60`、`network=6`。
- 设备构建：Windows / Browser / release；Git `21fd04f`，`workspaceDirty=true`，源码指纹 `f2c2fa171e9d3f589a2959e61a363a092191d88b714050f39847e904fdd81617`。
- 分析前工作区：Git `21fd04f0`，源码指纹 `aae88d0c11110abc4428ee5ce2658f687df4fcc4a26ed722d6ef35e13f7658c8`。两份源码指纹不同；本轮菜单复现与修复验证针对当前工作区。
- 配置：主机串流 2560×1440 / 60fps、WebGL2 + USM、sharpness=3、contrast=105、saturation=105、实验性 SR 关闭、手柄轮询目标 250Hz；自定义码率上限 100Mbps。

安全复算数据见 [metrics.json](../../dev-docs/validation/2026-11-08-handheld-stream-trace.metrics.json)。

## 最小时间线

| seq | 事件与观测 | 含义 |
| --- | --- | --- |
| 94 / 98 | `resolvingContext` / `compiledPlan` | 接收与显示目标均为 60fps；输入为 Browser physical gamepad |
| 127 / 133 | SessionReady / runtime launch | 会话进入可连接状态并启动 Browser runtime |
| 171 | H264 answer `42002a` | 实际采用 Baseline H264 |
| 187 / 190 | connected / firstPresented | 首次 video-frame callback 的连接后耗时 313.3ms |
| 191 | rendererCommitted | 1440p WebGL2 首次绘制完成，绘制累计样本仅 1 帧 |
| 192–194 | firstInbound / firstDecoded / firstKeyframeDecoded | 三个里程碑共享同一份初始 getStats 观测 |
| 201 | `pathStable`，RTT 基线 8.336ms | 初始网络分类稳定 |
| 332 / 341 | `networkEvidenceIncomplete` → `pathStable` | 3.995 秒后恢复分类，新的 RTT 基线 5.502ms |
| 376 / 377 | shell terminate / cleanupStarted | 文件终止在清理开始阶段，尾部验收需要下一份完整日志 |

跨层里程碑以各自 `observedAtMs` / `sampledAtMs` 解释；IPC 写入时间用于文件排序。

## 网络、恢复与六段性能证据

| 维度 | 当前证据 | 判断边界 |
| --- | --- | --- |
| 网络健康 | 初始 574 个接收包、丢包 0、jitter=4ms；两次稳定分类的 RTT 基线约 5–8ms | 后段拥塞、丢包及重传吞吐需要持续样本；分类 unknown 的原因是观测缺证 |
| 恢复链健康 | 初始收到 6 帧、解码 6 帧、关键帧解码 3 帧，首次呈现完成；PLI/FIR/NACK 初始计数均为 0 | 请求效果、后段冻结和恢复闭环缺少采样；Rust 的 ReferenceChain / cleanAnchorCommitted / DisplayStable gate 随 Rust trace 单独验收 |
| media supply | 首份 getStats 报 2560×1440、53fps，6 帧解码总耗时 68.499ms，均值 11.4165ms | 这是起播窗口，稳态源供给和解码帧率待量化 |
| steady supply | 持续 sessionReady 轮询；周期帧率观测数为 0 | 会话就绪轮询提供控制面事实；持续画面供给需要逐段帧率 |
| submit / present 长尾 | 首帧 JS/WebGL 绘制 21.5ms，其中纹理上传 20.5ms、submit 0.5ms；首个 GPU query 为 pending，GPU 时间样本数 0 | 60fps 帧预算约 16.667ms；首帧含冷启动成本，持续 CPU/GPU P95/P99 与物理 present 仍待观测 |
| 帧供给 delta | 起播 received 6 → decoded 6，浏览器 framesDropped=0；rendererCommitted 时绘制计数仅 1 | 两者来自不同采样时点；稳态五段供给差与限帧/覆盖计数待量化 |

Browser 呈现追踪来自 `videoFrameCallback`，首回调 `presentedFramesDelta=1`、`expectedDisplayLeadMs=-1.5`。持续 sourceFps、callback gap、presentedFrames jump、dropped-like streak 与 backpressure 缺少周期记录。

源码确认缺证来源：`browser-runtime-trace.ts` 在 production 构造 payload 前过滤 `browserWebRtcStatsObserved`、`browserObservationConsumed`、`renderTelemetryObserved`、`browserMediaResourcesSnapshot` 等详细事件。当前样本的队列丢弃通知计数为 0。

技能脚本结果：Browser behavior gate 因 `missing browser WebRTC getStats samples` 返回 FAIL；稳定性为 MISSING。零样本的恢复效果分数采用缺证语义，后续使用有效观测窗评价。

## 持续 USM 优化与实机对照

用户明确反馈掉帧持续发生，并在锐度 0 对照中报告明显缓解。该对照尚未附带新日志；锐化成本成为当前优先优化方向，持续 CPU/GPU 耗时与各段帧率由补齐的周期摘要量化。原日志的 105/105 色彩参数在当前 auto 计划中也会启用 WebGL，锐度 0 对照的实际渲染路径需要对应构建的观测确认。

原 USM 使用中心像素与八个邻居计算 3×3 高斯核 `[1,2,1]²/16`。当前补丁在输出尺寸与源尺寸相等时，使用四个 `(±0.5, ±0.5)` 源像素偏移的双线性采样求平均，数学上得到相同的模糊核；加上已有中心采样共 5 次。非 1:1 映射继续使用原 9 次采样，锐化强度映射和色彩处理保持一致。现有源尺寸 FBO 锐化 pass 也满足该条件，资源装配与帧调度沿用现有实现。

真实 WebGL2 对照在 macOS Chrome 157 执行，覆盖边界、噪声、棋盘、渐变、0 / 0.6 / 1 强度、非 1:1 映射和 CAS；其中两组直接使用 2560×1440、强度 0.6、contrast/saturation=105。88 组共比较 29,693,856 个 RGBA8 分量，129 个分量出现舍入差异，最大差值为 1/255。生产 GLSL 与当前 renderer 的 SHA 身份匹配，调度回放使用生产模块及受控 DOM/WebGL。

采样数量描述 shader 的算法成本；实际 GPU 耗时、Windows FPS 和物理 present 需设备观测。独立数据见 [USM metrics](../../dev-docs/validation/2026-11-08-usm-sampling.metrics.json)，复算入口为 [USM audit](../../dev-docs/validation/2026-11-08-usm-sampling-audit.cjs)。像素对照基线为 Git `21fd04f06849c37fa21c54469ef225c1a4332f1b`，源码和 shader 的完整 SHA 保存在 metrics。

## 修复范围

- [Stream.vue](../../src/pages/Stream.vue)：父焦点域保持有效；两菜单、画面/音量/文本、诊断详情及警告/失败弹层继续使用各自 Scope。
- [stream-focus-scope.test.ts](../../src/pages/stream/stream-focus-scope.test.ts)：直接编译生产页面的父 Scope 配置，调用生产几何引擎验证两菜单的焦点有效性、背景隔离、方向/确认与关闭后的承接节点；真实 Vue 组件验证触屏/鼠标点击及手柄确认到菜单选择事件的完整链路。
- [browser-runtime-trace.ts](../../src/streaming/runtime/browser-runtime-trace.ts)：production 摘要以 `frontend/key` 保留，按 session/runtime/client 身份限频；最多 4 个在途 IPC、32 个排队，hard fault 优先于摘要。
- [browser-runtime.ts](../../src/streaming/runtime/browser-runtime.ts)：摘要复用现有 getStats / renderer / callback 事实，保留网络、累计解码/丢帧/冻结与 RTCP 计数、五段帧率、3s/15s 策略窗、真实尺寸、CPU/GPU 耗时分位数和提交计数。原始计数按 `sampleId` 匹配，缺证显示 `counterObservationState=unavailable`。实际 scanout 标为 unavailable。
- [Renderers.ts](../../src/player/infra/render/Renderers.ts)：USM 的 1:1 路径采用四个半像素双线性采样计算高斯模糊，含中心共 5 次；非 1:1 路径继续使用原核。
- 生产摘要采用固定字段；详细 DOM/像素诊断沿用 dev 通道。采集、网络、帧调度和输入控制继续遵循现有合同。

## 验证

| 验证 | 结果 |
| --- | --- |
| 两菜单回归，修复前 | 初轮 2 项子 Scope 有效性失败；补触屏验证后回放旧配置，4 项全失败，触屏事件被 preventDefault |
| 菜单、几何引擎、Scope 生命周期与 UI 输入交接，修复后 | 初轮 5 文件 / 44 项通过；补触屏后的菜单文件 4 项通过，覆盖真实组件的触屏/鼠标点击与手柄确认 |
| 生产采样限频与队列预算 | 10,000 次重复调用仅构造一次摘要；新 client 即时获得首样本；4/32 预算与 hard fault 优先通过 |
| production / off / dev 媒体控制对照 | 相同 renderer / transport 操作结果；production 的详细 mediaDiagnostics 调用数 0 |
| 计数绑定与白名单 | 当前 sample 计数正确；异 sample 保留缺证；额外 URL 字段从摘要中排除 |
| 渲染、帧源、锐化数学与策略定向回归 | 5 文件 / 163 项通过 |
| 真实 WebGL2 像素对照 | 88 组全部通过，包含两组 1440p；RGBA8 最大差值 1/255 |
| 持续调度回放 | 10 秒、60fps 源、144Hz 显示：601 候选 → 601 提交；限帧、重复和失败均为 0，1 个 program、0 个 FBO，与基线相同 |
| 全量前端回归 | 72 文件 / 688 项通过 |
| `pnpm lint:fix` / `pnpm typecheck` / `pnpm build` | 通过 |
| audit CLI / 脚本语法 / `git diff --check` / 报告链接 / 日志与源码 SHA 复核 | 通过；重新生成的浏览器测试函数与 GLSL 和已执行像素对照一致 |

## 掌机验收

用包含本轮补丁的 Windows 构建，恢复锐度 3，保持原 1440p / 60fps / USM / SR 关闭配置，在连续运动画面中串流 60–90 秒。分别打开两个菜单，验证触屏选项确认、手柄方向/确认、B/Esc、菜单到子面板的切换与关闭后的游戏输入归位；正常结束串流后导出完整日志。

下一份日志优先复算 `frameSupply.receivedFps → decodeFps → videoPresentFps → callbackFps → rendererSubmitFps`，结合策略窗覆盖、CPU/GPU P95/P99、rateLimitDrop / overwrite / freeze 累计变化定位持续掉帧。
