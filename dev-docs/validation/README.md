# 专项验证证据

本目录保留浏览器精简与串流启动验收中的报告、metrics 和生产模块回放。当前待验范围以 [浏览器 RFC](../../docs/rfcs/2026-10-28-browser-runtime-simplification-and-stability.md) 与 [公共可靠性 RFC](../../docs/rfcs/2026-10-28-streaming-common-reliability-simplification.md) 为准。

| 当前阅读入口 | 证据与待验项 |
| --- | --- |
| [焦点裁切、滚动条与界面密度](2026-11-09-ui-focus-inset-density/README.md) | 统一8px焦点安全边距、隐藏全部滚动条与底栏重复提示，收紧页面、卡片、设置及弹层间距；完整本地前端12场景、实际滚动及lint/typecheck/build/diff检查通过，1430临时服务已关闭 |
| [焦点保持、全交互导航与超窄屏完成验证](2026-11-09-gamepad-focus-action-completion/README.md) | 173 个交互声明全部覆盖；busy / A 去重、分类往返、选择 / 键盘 / 录像及≤600px上下分栏完成。完整本地前端114项、86文件803项全量、63项定向与lint/typecheck/build/trace通过，真实尺寸与截图已保存 |
| [SR / 锐化 / 默认渲染控制](2026-11-09-render-controls-clarity/README.md) | 共用三档控件、全局默认 / 本次覆盖、顶部实际模式与短回退原因；800 项全量测试和 Rust 26 项通过，深浅色及窄屏导航预览已保存；实机持续帧率和回退待用户验收 |
| [导航覆盖与 A 确认焦点跳转](2026-11-09-gamepad-focus-action-audit/README.md) | 60 个 Vue 文件 / 106 处导航声明；7 处原生下拉框与文字、媒体控件操作缺口；实际 Vue 复现保存 / 刷新回退，原生 focus 清空恢复事务；72 项现有回归通过，独立目标断言失败 |
| [主机串流自动续期排查](2026-11-09-xhome-auth-refresh-investigation.md) | 4小时XHOME有效期核对；Web-only状态使启动恢复被跳过、串流入口缺续期及重新登录清理核心凭据；旧逻辑通过、当前目标行为失败，修复接续自动续期 |
| [SR / 锐化新日志](2026-11-09-sr-sharpen-fresh-trace-analysis.md) | 140份像素诊断全关闭；USM约55–56.7fps，SR主段58.32/末尾3s53.11/再试48.17fps，video常规段59.945fps；冻结前同窗解码122→提交102，候选/提交配平。上游1.236s中断与码流负载差异单列 |
| [SR持续供帧与诊断干扰续查](2026-11-09-sr-frame-supply-investigation.md) | 默认dev像素取证改为显式开关；WKWebView复现149ms准备长尾。1440p H264 1:1 RCAS隔离对照解码/提交380帧、58.45fps、P95间隔18ms；持续缺口接续上方新日志 |
| [默认候选准入修复](../../docs/reports/2026-11-05-browser-frame-admission-fix.md) | 常规预算候选直通、低FPS滚动准入及SR恢复去重；60文件610项测试、12组节拍及19组设备入口回放通过，真实供给/GPU/present待新设备样本 |
| [固定槽阶段设备日志](2026-11-05-latest-frame-scheduling-fresh-trace-analysis.md) | 采样源码匹配；SR累计352候选→268提交+84限帧跳过，相位冲突由生产回放复现；网络/解码约60fps、输入184.75Hz、严格Browser门禁PASS；准入已按上方报告修复，源供给继续待验 |
| [Browser最新帧调度简化](../../docs/reports/2026-11-05-browser-latest-frame-scheduling.md) | 主机/云游戏共用即时提交与直接跳帧；像素退为低频诊断，初版60文件591项测试与9组节拍回放通过；固定槽规则已由上方默认准入修复替换 |
| [调度简化前渲染与输入设备日志](2026-11-05-render-input-fresh-trace-analysis.md) | 历史样本冷准备/输入发送/资源清理正常；健康SR候选47.79→提交47.46fps，233组像素配对变化0；该样本共享采样缺证，新样本已补齐 |
| [SR冷准备与运行态补供帧历史修复](../../docs/reports/2026-11-03-sr-cold-preparation-and-runtime-supply-fix.md) | 历史前置准备、一次有界重试与56组回放；当前采用上方最新帧策略，像素作为低频诊断 |
| [SR GPU补供帧历史设备日志](2026-11-03-sr-gpu-supply-fresh-trace-analysis.md) | 旧样本首次CPU196ms超冷预算停用、补帧0；保留修复前采样与源码身份 |
| [GPU异步补供给历史验证](2026-11-02-gpu-drawable-supply-validation.md) | 保留40组受控回放的历史合同；当前交付资格由RVFC/播放计数统一推进 |
| [像素读取成本分段](2026-11-02-video-pixel-read-cost-validation.md) | 历史cold/warm分段；当前诊断沿低频异步读取继续记录成本与像素差异 |
| [SR60fps 瓶颈续查](2026-11-02-sr-60fps-bottleneck-analysis.md) | 同代际 RVFC/rAF、候选交付和回放；SR60fps/实际呈现待验 |
| [最新网络与渲染分析](2026-11-02-network-render-fresh-trace-analysis.md) | 同窗网络、恢复、供帧、处理成本与长尾 |
| [公共启动修复报告](../../docs/reports/2026-10-30-common-startup-fixes.md) | 当前正式回归入口和五项问题的修复行为 |
| [home 首帧与退出新日志](2026-10-30-home-startup-fresh-trace.md) | 空 ICE 轮询后成功启动、首帧和资源清零 |
| [浏览器代码验证](2026-10-29-browser-policy-code-validation.md) | 精简初始基线、共享样本和稳定性工具 |

日期化 `.py` 和 `.cjs` 文件绑定对应样本或历史源码，用于复算该份报告；命令、输入身份与结果边界见同名 Markdown。最新分析仍依赖早期复算模块和回放，依赖链随本轮证据保留。历史 metrics 中的源码/脚本 SHA 表示采样时身份。

`browser-runtime-optimization-baseline.json` 是 [复杂度测量工具](../../scripts/measure-browser-runtime.mjs) 的固定基线；`browser-runtime-optimization-current.json` 是当时的测量结果。常规日志分析入口使用 [本地验收手册](../streaming-reliability-local-validation.md)中的 `.agents/skills/analyze-runtime-logs/scripts/` 工具，仓库级回归统一见 [tests/](../../tests/README.md)。
