# 专项验证证据

2026-11-12 整理：保留当前待验资料、近期交付摘要和正式工具依赖。已替代的回放、重复截图、逐场景 JSON 与临时校验输出已清理。

已跟踪的历史产物可从[清理前的 Git 基线](https://github.com/xuasir/xbxrc/tree/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation)查询；文档中的历史证据链接绑定清理前文件。任务状态以[当前账本](../../docs/project-task.md)为准。

## 当前待验与专项资料

| 入口 | 保留用途 |
| --- | --- |
| [帧时钟、诊断调度与供给空窗](2026-11-10-browser-frame-freshness-clock-and-diagnostics/README.md) | 时钟校准、供给配对、CPU/GPU 完成代理与远端长测 |
| [Worker 新鲜度设备分析](2026-11-10-worker-freshness-latest-trace-analysis.md) · [独立解码供给复验](2026-11-10-decoded-supply-latest-trace-analysis.md) | 当前供给分层、网络/恢复、提交长尾与原始指标复算 |
| [独立解码供给夹具](../../docs/reports/2026-11-09-browser-decoded-frame-supply.md) · [分辨率与 Worker 回放](2026-11-10-session-resolution-worker-freshness/README.md) | 解码帧交付、能力回退、单帧在途与 latest-only 行为 |
| [SR 采样与 USM 保护](2026-11-10-sr-sharpen-sampling-optimization/README.md) | 数值一致性、真实 EASU 放大与硬件成本对照 |
| [轻流动正式前端](2026-11-10-ui-motion-vitality/README.md) | Vue 装配、输入和减少动态效果回放 |
| [手柄映射、额外按钮与四路震动](2026-11-11-gamepad-locked-input-and-extra-buttons/README.md) | 当前离线输入夹具与实体手柄验收 |
| [高级诊断交互](2026-11-11-pc-diagnostics-first-version/README.md) · [生产 trace 验证](2026-11-08-production-trace-validation.md) | 诊断采集、导出与正式 trace 证据合同 |
| [Browser P0](2026-11-02-browser-stable-p0-validation.md) · [发行隔离](2026-11-02-desktop-edition-isolation.md) | 安装身份、依赖闭包、双平台与实机验收 |
| [TAK 布局与引擎核对](2026-11-12-tak-layout-and-engine/README.md) | 官方来源、schema、原方法和兼容性行为证据 |

## 近期交付摘要

| 入口 | 摘要 |
| --- | --- |
| [云游戏菜单性能](2026-11-12-cloud-menu-performance-implementation/README.md) | 生命周期、目录规模与缓存结果 |
| [主机缓存](2026-11-12-home-host-cache/README.md) | 热返回、更新、账户隔离与失败保留 |
| [云游戏刷新与分类栏](2026-11-12-cloud-refresh-and-filters/README.md) | 栏目稳定、横向分类与焦点显露 |
| [高清素材](2026-11-12-hd-ui-assets/README.md) · [手柄图片](2026-11-12-xbox-controller-art-quality/README.md) | 当前资源来源、尺寸、哈希与原版图标恢复 |
| [快捷键与统一震动](2026-11-12-shortcut-hint-unified-vibration/README.md) · [官方震动时序](2026-11-12-official-rumble-share/removal/README.md) | 当前设置合同与 Share 撤销后的验证 |

保留的日期化回放与夹具服务于待验任务，依赖随相应入口保留。[固定复杂度基线](browser-runtime-optimization-baseline.json)由[测量工具](../../scripts/measure-browser-runtime.mjs)读取。常规日志验收见[本地验收手册](../streaming-reliability-local-validation.md)，正式回归入口见[tests/](../../tests/README.md)。
