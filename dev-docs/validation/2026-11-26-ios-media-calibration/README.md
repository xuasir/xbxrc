# iOS 接收统计、色彩与音频校准验收

日期：2026-11-26；三项代码及自动验收完成。[RFC](../../../docs/rfcs/2026-11-26-ios-media-calibration.md) · [交付报告](../../../docs/reports/2026-11-26-ios-media-calibration.md)。长时稳定性由用户指定留到后续。

14 项 iOS 定向 XCTest、首轮 15 项 Rust 监督测试全部通过，XCTest 运行时警告与测试失败均为 0。iOS 冒烟使用获授权启动的既有 iPhone 18 Pro / iOS 27.2；完整 Simulator 测试构建包含全部修改源码，原生测试覆盖真实 NV12 Metal 呈现和默认/反馈两种 WebRTC 工厂的音频配置及输入可用性。

新增统计/色彩/音频事实模块通过 iPhoneOS SDK Swift 6 严格类型检查。格式、plist、共享会话边界及 diff 检查通过。源代码实现保持 Rust 会话/监督、libwebrtc 接收/解码与原有 Metal presenter。

## 结果索引

- `xctest-summary.json` / `xctest-tests.json` / `smoke-test-cases.log`：14 项 iOS 测试的结构化结果与逐项明细。
- `rust-smoke.json` / `rust-smoke.log`：15 项共享监督测试。
- `shared-compatibility.json` / `shared-compatibility.log` / `concurrent-changes.diff` / `validated-sources/`：验收后共享监督模块收到另一任务的恢复/延迟字段改动，追加 18 项兼容冒烟通过；本轮缓冲字段保留，已验源版本与后续变更分别归档。
- `device-typecheck.json` / `format.json` / `checks.json`：真实 Device SDK、新增源码格式及边界验证。
- `sdk-contract.json`：固定 M137 的 receiver 能力与音频输入/首选属性合同。
- `implementation.diff` / `execution-manifest.json`：本轮修改与创建路径、源码摘要、测试命令及进程完成依据。
- `failure-diagnostics.md` / `device-typecheck-initial.*`：已修复的编译导入和结果导出诊断。
- `cleanup.json`：临时目录移除、释放量、磁盘空间及模拟器恢复。

## 实机事实边界

| 维度 | 本轮交付与验收 | 实机接续 |
| --- | --- | --- |
| 网络健康 | RTT/jitter/loss 原口径保留，缓冲目标/最低/窗口均值补齐，旧监督基线通过 | 真实路径、丢包及缓冲变化 |
| 恢复链健康 | 异步依赖完成后捕获事实快照；旧反馈/恢复门槛和连续窗口测试通过 | 真实关键帧/恢复后连续解码与呈现 |
| media supply | 接收统计累计值和每流基线保持有界，缺失/重置/停供测试通过 | 真实 byte/packet/frame 增量 |
| steady supply | 共享 3s/15s 窗口接入有效缓冲样本；旧连续覆盖回归通过 | 实际持续供给与 fresh 覆盖 |
| submit/present 长尾 | 真正的 Metal 帧呈现冒烟通过，既有 GPU/drawable P95/P99 观测保留 | 实际负载下的提交/上屏长尾 |
| 帧供给 delta | received/decoded/submitted/GPU completed/drawable presented/local drop 原同窗口径保留 | 真实会话同窗差值 |

本轮使用离线计数、人工帧及系统音频配置验证。真实色彩、声道听感、蓝牙切换与端到端延迟以 Home / Cloud 实机短测为依据。必要证据保留，临时产物按本轮清单清理；共享依赖与系统管理缓存保留。
