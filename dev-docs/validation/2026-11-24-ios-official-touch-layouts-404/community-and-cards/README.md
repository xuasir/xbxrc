# 游戏触屏布局来源与全屏预览

2026-11-24。官方短版本、社区来源、本机保存和卡片入口完成；真实官方包保存及 Home 实机体验继续由正常 Xcode 应用验收。

列表使用紧凑纵向卡片：名称与默认标记位于上方，卡片保持完整点击入口；三行元数据与右侧垂直居中的胶囊保存按钮共用高度，使用 12pt 内边距和 4pt 行距；普通字号卡片高度验证在 96pt 内，保存命中保持 44pt。点击卡片直接打开全屏触控画布，复用触屏手柄调整页的 `TouchLayoutWorkspace` 工具栏、方向胶囊、设置面板和 `TouchLayoutCanvas` 游戏占位与竖屏分区。返回与保存都先完成本机保存，再退出；保存失败通过原生提示保留预览并支持重试，待适配布局保留预览入口。

- [最终卡片](community-touch-layout-cards.png)
- [竖屏全屏预览](community-touch-preview-portrait.png)
- [横屏全屏预览](community-touch-preview-landscape.png)
- [保存后的本机库](community-touch-saved-library.png)
- [最终 UI 摘要](fullscreen-ui-summary.json)、[编译摘要](fullscreen-build-summary.json)、[验证清单](validation.json)
- [此前 17 项包处理](touch-tests.txt)、[13 项资源库](storage-tests.txt)、[1 项社区路径](api-tests.txt)与[6 项 Swift 身份检查](selection-probe.txt)
- [真实公开社区样本](real-community-sample.json)：better-xcloud 不可变 commit `4416b1ab07588a3244a5c02ce1fa5dfa64d8df74` 的《莎木 II》布局成功封装，默认布局和作者 Kode-Z 保留；原始公开样本保存在 `fixtures/`。

官方布局目录使用标题详情 `details.supportedTabs`，短版本兼容 1–4 段数字并保留显示原值。社区来源沿游戏索引与共用布局引用，在同一 commit 下载并封装为既有资源合同；两类来源独立返回，失败来源使用共享重试，可用来源继续预览和保存。社区保存身份随内容更新保持稳定。

本轮复用上述 Rust 与 Swift 检查，最终只覆盖一条 UI 路径：列表 → 直接全屏 → 横竖屏切换 → 返回自动保存 → 本机库。视口方向及各按键屏幕边界均有断言。共享错误模型新增字段期间遇到生成绑定差异，沿正式脚本同步并保留四参数构造兼容；详情见 [失败及恢复诊断](failure-and-final-diagnostics.json)。[源指纹](source-hashes.json)和定向差异记录当前源码，最终截图使用 XCUIScreen.main 整屏抓取并按 EXIF 方向归一化；紧凑版最终 UI 通过，耗时 42.597 秒。

原始用户诊断继续保存在 Downloads；[既有分析](diagnostic-analysis.json)的 schema/privacy 检查通过，全局 gate 因片段内 6 个其他 metadata/image 开始事件缺少终态而为 FAIL。本次布局失败锚点为官方目录短版本解析，后续已完成对应修复；真实官方服务与 Home 输入体验保持待验状态。

临时清理覆盖登记的 r1/r2/r3/r4/r5、前轮 404 排查 r3 与一次性社区程序构建产物；共享 SDK、Rust 缓存、已安装应用、登录及本机布局库保留。清理清单、释放量和路径核验见 [清理记录](cleanup.json)。

最终 r5 按用户最新要求移除箭头并将保存按钮垂直居中。[卡片最小冒烟](card-only-ui-summary.json) 1 项通过，25.758 秒，临时前缀探针已精确恢复；此前全屏、旋转和返回保存结果复用。最终卡片编译通过，[局部效果](compact-card-detail.png)与[定向差异](task-changes-centered-save.diff)已归档。
