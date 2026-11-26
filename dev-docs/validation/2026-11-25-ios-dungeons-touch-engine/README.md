# iOS 地下城官方布局保存与触控能力验收

本轮补齐官方 `v` 前缀 schema、控件背景与摇杆帽图片绘制、拉动提示前景、状态素材切换，以及触摸板/格斗组合/单轴/死区/曲线输入。用户截图的 `gameplay_default` 和 `dialogues` 保存门禁集中于 schema 识别及重复图片位置诊断。

最终 Simulator 验收为 **9 项通过，0 项失败**：8 项单元测试与 1 条 UI 流程，使用 iPhone 18 Pro / iOS 27.0 / Xcode 27。定向测试通过后完成四张截图检查与证据归档。

| 证据 | 内容 |
| --- | --- |
| [测试摘要](images-test-summary.json) · [运行状态](image-run-status.json) · [构建日志](build-and-test.log) | 9 项通过，0 跳过，0 失败；相关源文件与测试开始时的源码一致 |
| [卡片保存](screenshots/second-layout-card-saved.png) · [预览](screenshots/second-layout-touchpad-preview.png) · [本机列表](screenshots/second-layout-saved-library.png) | 直接点击第二布局保存，预览拖动，保存退出，检查本机记录 |
| [素材诊断图](screenshots/official-background-and-knob-assets.png) · [视觉检查](visual-review.json) | 原始 CGImage、背景/knob 复用、cap 色彩、透明度与状态切换 |
| [检查记录](checks.json) · [范围格式化](final-format-validation.json) | 12 个 Swift 文件本轮修改范围格式化通过；前轮 lint 与边界检查记录复用 |
| [能力范围](support-matrix.md) | 已实现能力、官方字段来源及全量 TAK 仍待对齐的能力 |
| [源码哈希](source-hashes.json) · [任务差异](task-changes.diff) · [共享文件范围](shared-file-scope.json) | 可恢复基线的文件提供任务差异；原有修改的共享文件保留最终快照与修改范围 |
| [执行清单](execution-manifest.json) · [清理记录](cleanup.json) | 记录中断后的临时目录恢复、产物归档、清理范围和磁盘空间 |

UI 验收使用 Halo Infinite 游戏入口下的官方格式夹具，其第二布局同时包含 `v3.1`、touchpad 和背景图片。真实 Minecraft Dungeons II 游戏包与实机串流操作由设备侧验收。[用户原因记录](user/user-message-evidence.json)保留聊天附件中已确认的字段和提示。
