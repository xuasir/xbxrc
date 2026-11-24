# iOS 图文对齐与选择控件样式验收

2026-11-24。沿既有 SwiftUI 设计规范修复图标文字对齐、内容 Tab、胶囊筛选及工具按钮，公共样式由现有组件统一装配。

- 图标中心对齐首行文字中心，多行提示保持同一规则；入口图标在24～32pt间缩放，标题和说明使用剩余宽度；等待指示和图标共用槽位。
- 内容 Tab 与胶囊选项预留选中字重的宽度，切换保持相邻选项的位置；统一底槽、内缩、操作尺寸和选中动画，支持减少动态效果。
- 云目录重置、触屏保存与设置入口、官方布局菜单及串流诊断分组使用公共控件；长选项沿局部横向滚动呈现。

[验收汇总](validation.json)合并4个唯一用例，云筛选和诊断分组使用首轮通过结果，内容 Tab 和原生快照使用最终通过结果。

| 验收 | 结果 | 证据 |
| --- | --- | --- |
| 共享样式原生绘制 | 通过；390pt深浅色、320pt英文辅助字号、1024pt英文宽窗，共4组 | [深色](styles-phone-dark.png)、[浅色](styles-phone-light.png)、[窄屏大字号](styles-narrow-accessibility.png)、[宽窗](styles-tablet-light.png) |
| 云目录筛选、菜单与搜索 | 通过；控件中线和44pt命中、选中状态及真实筛选绑定 | [工具胶囊](calibration-cloud-tool-capsules.png)、[搜索](calibration-cloud-search-results.png) |
| 游戏内容 Tab 与相册类型 | 通过；内容切换、类型切换及空状态位于悬浮底栏上方可用区域中心 | [概览](priority-game-overview.png)、[相册](priority-game-album-empty.png) |
| 串流诊断分组 | 通过；四组切换、指标显示与复制摘要 | [诊断面板](streaming-diagnostics-upper-right.png) |

使用 Xcode27、iOS27的 iPhone18Pro 模拟器执行 arm64无签名构建与定向 XCTest，最低部署保持iOS26。1024pt证据采用独立窗口的原生绘制。[最终结果](final-test-summary.json)、[首轮结果](initial-test-summary.json)、[编译与测试摘要](build-test-excerpts.txt)及[诊断修正](failure-diagnostics.json)保留完整的结论来源。

[任务差异](task-changes.diff)、[源哈希](source-hashes.json)与[范围格式化](final-swift-format.json)记录本轮修改；[原生绘制探针](native-style-smoke.swift)保存在证据目录，[恢复记录](temporary-source-restoration.json)确认应用测试文件恢复至任务开始时的原件。

[清理记录](cleanup.json)确认构建、测试和导出操作结束，本轮临时目录、DerivedData、结果包、日志及重复导出清理684.18MiB。保留9张稳定截图、验收摘要、诊断、源哈希和复现探针；现有WebRTC、Rust缓存和模拟器继续复用。

实机重点复核“我的”、游戏详情的 Tab 与触屏设置的胶囊切换观感。
