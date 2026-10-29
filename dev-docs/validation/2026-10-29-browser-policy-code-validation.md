# 浏览器策略 S0–S3 代码改造验证记录

日期：2026-10-29。当前状态：代码与自动验证完成，S4 用户设备验收待进行。

新日志后的用户修订已实现：标准 SR 目标、共享帧计数补足、立即提交及接收帧率权限见[专项验证](2026-10-29-browser-standard-output-and-latest-frame-validation.md)。本文命令与维护规模已更新到该修正版本。

依据：[成果计划](../../docs/isu/2026-10-29-browser-policy-stability-and-native-quality.md)、[完整功能优化 RFC](../../docs/rfcs/2026-10-28-browser-runtime-simplification-and-stability.md)。实现基于 `bec3acfb` 和当前工作树；逐文件源码 SHA-256 见[维护规模快照](browser-runtime-optimization-current.json)。

## 交付合同与自动证据

| 范围 | 当前行为 | 自动证据 |
| --- | --- | --- |
| S0 源尺寸与节拍 | 处理 backing 保持当前真实源宽高下限；真实 video 尺寸优先于请求与缓存；源超过 GPU 能力进入原生 video。共享帧 marker 驱动 latest-only，必要限流使用累计截止时间 | render policy / SR ladder / renderer 用例；受控 RVFC、rAF 与 GPU mock 覆盖 30/60fps × 90/144/165Hz、32/34ms 边界、latest 覆盖、隐藏恢复、截图单列、零 backing 和 GPU 上限 |
| S1 控制域隔离 | 基础显示计划消费用户、源、显示与能力事实；传输证据控制有能力的网络动作；本地 renderer 提交与成本决定增强回退 | network / render health / runtime profile / runtime 回归；健康 LAN、高 RTT、低运动低码率与字段缺失保留有效画质；仅 renderer 停顿触发本地保真回退 |
| S2 五段帧链 | received、decoded、video advancement、callback、当前 renderer submitted 分列，最新覆盖/限流/失败/截图分别计数。网络窗口按路径隔离；renderer 窗口按代际、targetFps 和实际源/输出尺寸隔离 | StatsService / browser-stats 用例覆盖单采集、数值副本、provider 复用对象、身份/计数重置和真实尺寸；面板 view model 与诊断字段接线回归 |
| S3 稳定软动作 | 完整 3s 本域症状与 15s 同域间隔，上一动作先结算效果。网络/关键帧效果至少等待 3s 与两次有效 RTT；超过 15s 期限为 unknown。播放重建使用新视频代际与本地 3s 覆盖 | policy / runtime 用例覆盖短尖峰、重复/晚到样本、缺证、隐藏失效、晚到协商、无效果、健康长窗和一次增强试探回退 |
| 局部预算闭环 | 两次关键帧和一次播放重建后，完整新代际健康长窗关闭旧 episode；下一次持续故障获得新局部预算。公共连接恢复继续使用 120s / 两次预算 | runtime 集成用例覆盖预算耗尽、健康关单、下一次故障与既有入包宽限；全过程保持同一 client，并验证连接仲裁调用 |
| 观测与资源预算 | trace 过滤先于 payload 构造，4 个 IPC 在途、32 个排队；每秒汇总帧计数，GPU 运行期检查按周期执行；uniform、texture/FBO 按配置和尺寸复用 | trace off / production / dev、写入失败与队列压力回归；真实 renderer 的 GPU mock 覆盖稳定复用、尺寸增长、context loss；runtime off/dev 集成用例验证相同媒体决策 |
| 播放交接与释放 | 候选 renderer 以 5s 首帧期限交接；隐藏预算暂停期限并同步旧 renderer；释放统一清理 timer、帧订阅、截图请求、GPU/DOM、音频和麦克风晚到轨道 | PlaybackService / PlayerClient / frame-source / capture-queue / AudioEffectsService / MicrophoneService 用例覆盖暂停、取消、替换、失败和晚到完成 |
| 报告闭环 | 共享样本、五段帧差、实际尺寸下限、动作覆盖/间隔/能力/效果、GPU/trace 预算和资源释放可复算；按单调 observedAtMs 解释 IPC 乱序，周期快照与生命周期资源事件共同验收 | 18 项 Python 正负/集成 fixtures；包含实际 `browserMediaResourcesSnapshot` 名称、ready timer 残留、共享帧轮询上限/退出、网络效果期限、renderer 代际、采集间隔/策略窗语义与 behavior report 的显式稳定性门槛 |

CPU draw duration 表达 JavaScript/WebGL 提交成本；GPU 时间和真实 scanout 在面板显示 unavailable。离线调度与资源用例使用受控时钟和 GPU mock，设备运行帧率、GPU/CPU 长尾、像素与感知画质由 S4 配对测试取证。

## 完整自动验证

| 命令 | 结果 |
| --- | --- |
| `pnpm lint:fix` | 通过 |
| `pnpm test` | 44 文件 / 326 项全部通过 |
| `pnpm exec vue-tsc --noEmit` | 通过 |
| `pnpm build` | 266 个模块，生产构建通过 |
| `python3 -B -m unittest discover -s scripts/tests -p 'test_browser_runtime_stability_report.py'` | 18 项通过 |
| `git diff --check` | 通过 |

本批保持 Tauri + Vue 3 + TypeScript + Rust 与现有浏览器/公共执行链；本批 Rust 源码改动为零。

## 维护规模

统计包含基线生产文件和 `src/player`、`src/streaming/runtime` 下新增生产文件；测试与报告脚本单列。删除文件退出当前集合，逐文件原始行、AST 语句、分支与 SHA-256 保存在正式快照中。

| 指标 | bec3acfb 基线 | 当前 | 变化 |
| --- | ---: | ---: | ---: |
| 生产文件 | 52 | 62 | +10 |
| 原始行 | 15,674 | 15,222 | -452 |
| AST 语句 | 7,039 | 6,898 | -141 |
| 分支 | 1,137 | 1,297 | +160 |
| runtime 直属可变绑定 | 审计 129 / 本轮起点 94 | 74 | 较审计 -55 / 较本轮起点 -20 |

新增分支主要承担取消、代际、资源能力、观察期限与缺证保护。结构减法体现在重复状态、混合控制输入、重复采集与热路径工作收敛；维护规模与设备运行效果分别结算。

复测命令：

```sh
node scripts/measure-browser-runtime.mjs dev-docs/validation/browser-runtime-optimization-baseline.json dev-docs/validation/browser-runtime-optimization-current.json
```

## S4 用户手测交接

自动启动应用与串流继续按用户约定暂停。已取得源码摘要完全匹配的[新 Mac/cloud 日志](2026-10-29-browser-policy-fresh-trace-validation.md)：源尺寸下限、公共生命周期、动作归属与退出资源清理通过；有效前台窗口的回调/提交供给稀疏、共享面板样本及完整配对/长测继续验收。[此前 Mac 手测](2026-10-29-browser-runtime-manual-analysis.md)和[策略正确性审计](2026-10-29-browser-render-policy-correctness-audit.md)保留为历史依据。

| 待验范围 | 取证要求 |
| --- | --- |
| Mac home/cloud 与直连/TURN | 新构建完成启动、首帧、前台播放、显示设置/截图/mic 和正常退出；home 媒体阶段与 cloud 媒体阶段各有样本 |
| Windows WebView2 / ROG / FSE | 按实际输入后端记录设备、WebView/GPU、显示刷新率/DPR、网络和 codec；完整功能及生命周期组合沿 RFC V1–V14 执行 |
| 源像素与感知画质 | 1440p/2160p 源在小/同尺寸/大视口及 DPR1/1.5/2 下取得真实 backing、SR 开/关与保真回退截图；同源静态细节、振铃、色彩和动态画面配对比较 |
| 帧供给与观测开销 | 同设备/服务/网络/渲染配置至少三轮配对；记录 received→decoded→video→callback→submitted 差值、冻结和 CPU/GPU/上屏长尾，另比较 trace 与面板开关 |
| 故障和长测 | 受控网络断开/恢复、WebGL/context loss、后台/睡眠、mic/截图失败与资源清理；home/cloud 各至少2h，核心发行组合追加8h |

新 trace 绑定当前 revision、dirty 源码摘要、实际构建、配置与 session/operation/peer/video/renderer 身份。使用 `trace_browser_webrtc_behavior_report.py TRACE --require-stability` 和 `trace_streaming_session_lifecycle_gate.py TRACE --require-cleanup --require-closed`，恢复批次追加 `--require-recovery`；freshness、构建匹配和多文件轮转沿[本地验收手册](../streaming-reliability-local-validation.md)执行。

任务保持 In Progress。优先取得 Mac 当前构建的 1440p SR 开/关前台配对与正常退出样本，再补主机媒体、Windows/掌机及长测；完整验收后写最终任务 Report。
