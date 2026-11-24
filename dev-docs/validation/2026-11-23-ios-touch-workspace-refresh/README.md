# iOS 真实触屏视口与刷新入口验收

2026-11-23。两个触屏配置页直接复用正式 Surface 与实际窗口尺寸，设置默认收起；手机方向胶囊请求系统旋转，iPad 根据窗口几何显示当前方向。大字号将标题、操作和胶囊分行呈现。我的及子页、个人游戏详情、云目录统一通过原生下拉刷新更新内容，分区重试和媒体链接更新保留各自用途。

## 最终验证

| 范围 | 通过证据 |
| --- | --- |
| 最终手机画布与方向/草稿/保存 | [r6-phone-test-summary.json](r6-phone-test-summary.json)，2 项通过；原生探针覆盖两个子页、4 组窗口输入，共 8 张快照。 |
| 最终 iPad 画布与系统方向/草稿/保存 | [r6-ipad-test-summary.json](r6-ipad-test-summary.json)，2 项通过；两页、4 组窗口输入，共 8 张快照。 |
| 手机模板、外观持久化和下拉恢复 | [r5-phone-test-summary.json](r5-phone-test-summary.json)，11 项通过，含 7 项模板单元、原生快照及 3 条 UI 流程。 |
| iPad 模板、外观持久化和方向 | [r5-ipad-test-summary.json](r5-ipad-test-summary.json)，3 项通过。 |
| 资料页返回和栏目独立重试 | [r2-test-summary.json](r2-test-summary.json)，12 项通过；该轮下拉用例经 r3/r5 定向复验通过。 |
| iPad 宽窗操作与相册返回 | [r4-ipad-test-summary.json](r4-ipad-test-summary.json)，3 项通过；该轮方向用例经 r5/r6 修正并通过。 |
| 设备编译与静态检查 | [device-build.json](device-build.json)、[validation.json](validation.json)：iPhoneOS arm64、最低 26.0，Swift 格式/解析、前端 lint、工程与串流边界通过。 |

原生探针输入为 390×844、844×390、320×800 大字号与 1024×768；对应实际 Surface 分别为 390×422、844×390、320×400、1024×768，位置和原始变换同时经过断言。手机与 iPad 各执行完整矩阵。工具页采用深色游戏表面，`tablet-light` 文件名记录宿主主题输入。

## 关键截图

- [手机横屏画布](touch-editor-landscape-dark.png) · [手机竖屏画布](touch-editor-phone-dark.png)
- [手机窄屏大字号调整](touch-adjustment-narrow-accessibility.png) · [手机窄屏大字号编辑](touch-editor-narrow-accessibility.png)
- [iPad 画布与方向提示](ipad-touch-editor-tablet-light.png) · [iPad 窄窗大字号](ipad-touch-adjustment-narrow-accessibility.png)
- [设置收起后的实际触屏](touch-template-adjusted-full-viewport.png) · [重启后保存外观](touch-template-restored-after-relaunch.png)
- [下拉刷新恢复](cloud-pull-refresh-recovered.png) · [我的页](annotations-my-overview.png) · [分区重试恢复](section-errors-cloud-recovered.png)

[retained-images.json](retained-images.json) 记录 27 张保留截图的来源、轮次和 SHA-256。XCUIScreen 导出保留系统的 EXIF 方向；原生绘制矩阵以明确窗口尺寸生成 PNG。

## 诊断与收尾

[failure-diagnostics.json](failure-diagnostics.json) 保留早期返回定位、下拉触发阈值、iPad 方向请求和大字号视觉调整记录；最终通过摘要对应各项解决结果。[native-render-probe.swift](native-render-probe.swift) 为可复用的临时验收探针，正式测试源码已按 [temporary-source-restoration.json](temporary-source-restoration.json) 精确还原。

本轮临时根为 `/private/tmp/xbxrc-ios-touch-workspace-refresh-20261123`。归档后按登记轮次清理结果包、重复导出、临时 DerivedData、日志、基线备份和一次性配置；过程与磁盘核验见 [cleanup.json](cleanup.json)。共享 WebRTC SDK、Rust 缓存、模拟器及其他任务产物继续保留。

真实设备按键大小、拖动手感及本地串流/云游戏的输入体验由用户手动验收。
