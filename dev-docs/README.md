# 开发与验收入口

| 文档 | 用途 |
| --- | --- |
| [rust-mod-organization.md](rust-mod-organization.md) | Tauri Rust 模块职责与命名规范 |
| [gamepad-navigation.md](gamepad-navigation.md) | 几何导航、焦点与公共输入接入 |
| [first-frame-to-priming-scheduling.md](first-frame-to-priming-scheduling.md) | 原生视频首帧到 priming 的调度参考 |
| [streaming-reliability-local-validation.md](streaming-reliability-local-validation.md) | 当前串流公共生命周期、恢复与实机验收 |
| [validation/](validation/README.md) | 浏览器供帧、渲染与启动问题的专项验证 |

任务方案和结果进入 [docs/](../docs/README.md)，历史 pipeline 分析统一进入 [docs/isu/](../docs/isu/)。可复用工具见 [scripts/](../scripts/README.md)，日志门禁回归见 [tests/](../tests/README.md)。
