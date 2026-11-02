# 文档入口

桌面主线采用 Tauri + Vue 3 + TypeScript + Rust，当前边界以 [AGENTS.md](../AGENTS.md) 和代码为准。

| 目录/入口 | 用途 |
| --- | --- |
| [project-task.md](project-task.md) | 当前进行中、待验与阻塞任务，以及近期完成摘要 |
| [project-task.archive.md](project-task.archive.md) | 合并后的历史任务索引；原快照和完整历史备注可按文件内记录的 Git 基线读取 |
| [isu/](isu/) | 探索与历史能力分析 |
| [rfcs/](rfcs/) | 方案、执行进度、决策与验收计划 |
| [reports/](reports/) | 已交付结果与证据边界 |
| [references/](references/) | 长期协议、构建与设计参考 |
| [designs/](designs/) | 设计源文件和预览资源 |
| [dev-docs/](../dev-docs/README.md) | 当前开发规范、本地验收与专项验证证据 |
| [scripts/](../scripts/README.md) | 构建、发布、维护与诊断工具 |
| [tests/](../tests/README.md) | 仓库级运行日志门禁回归及夹具 |

## 当前主线参考

- [Rust 模块组织](../dev-docs/rust-mod-organization.md)、[几何手柄导航](../dev-docs/gamepad-navigation.md)。
- [统一手柄输入 RFC](rfcs/2026-10-27-unified-gamepad-input.md)与[交付报告](reports/2026-10-27-unified-gamepad-input.md)：Browser/Native 后端、公共 owner、中性释放和 Windows/ROG 待验项。
- [浏览器策略与供帧精简 RFC](rfcs/2026-10-28-browser-runtime-simplification-and-stability.md)：当前策略与 SR60fps 验收；[专项验证索引](../dev-docs/validation/README.md)。
- [串流公共可靠性 RFC](rfcs/2026-10-28-streaming-common-reliability-simplification.md)与[本地验收手册](../dev-docs/streaming-reliability-local-validation.md)。
- [receive/恢复边界 RFC](rfcs/2026-06-02-webrtc-recovery-boundary-redesign.md)：ReferenceChain、RecoveryLedger、clean anchor 与显示闭环。
- [FFmpeg SDK 构建说明](references/ffmpeg-sdk.md)。

## 维护约定

开发任务通过 task-run 登记：简单任务内联追踪，复杂任务使用 RFC 与 Report，探索使用 deep-brainstorm。模板由对应 [skills](../.agents/skills/) 维护。

历史 ISU、RFC 和 Report 描述各自日期的实现与验证结果；阅读时结合当前主线入口和源码核对。历史完成任务的逐次快照、旧命令和已删除代码路径可从 Git 获取。清理时按有效替代、引用关系和正式回归覆盖判断，持续保留待验项及其证据。
