# 串流安全区与公共弹出层背景验收

2026-11-23。串流入口、左右快捷栏与诊断面板复用 `appSurface(prominent: true, forceDark: true)`：系统材质配合深绿色罩层，降低透明度时使用 `#415D4E` 实色面板。快捷栏保持 52pt，菜单层级位于触屏与指标上方。

正式触屏绘制、44pt 命中区域和编辑位置读取同一窗口安全范围；竖屏控件位于下半区。左右横屏、竖屏和 iPad 的入口、菜单与诊断均已完成原生交互验收。

| 验证 | 结果与证据 |
| --- | --- |
| iPhone 18 Pro | 3 项串流 UI 通过：两项安全区结果见 [初次摘要](r6-phone-initial-summary.json)，诊断分组、复制与开关见 [最终摘要](r6-phone-final-summary.json) |
| iPad Pro 13-inch (M5) | 相同 3 项 UI 全部通过，见 [最终摘要](r6-ipad-final-summary.json) |
| 布局、模板与输入 | 复用前序 16 项定向验收及输入接管回归，见 [r1](r1-test-summary.json) 和 [r4](r4-input-and-ui-summary.json)；相关 UI 与几何源码哈希保持一致 |
| iPhoneOS Debug | [设备构建通过](device-build-summary.json) |
| 格式与边界 | Swift 严格格式、`pnpm lint:fix`、串流会话边界、工程语法及差异检查通过 |

诊断切换测试逐项验证原生选中状态，并等待目标行进入无障碍树。前序即时查询失败、生成绑定同步期间的编译错误及审查超时保存在 [失败诊断](failure-diagnostics.json)，最终两端测试与设备构建通过。

最终保留 24 张验收 PNG（包含 6 张模板布局证据）及 6 份窗口安全区记录。关键界面：[手机横屏菜单](final/iphone-streaming-safe-menu-landscape-left.png)、[手机诊断](final/iphone-streaming-diagnostics-upper-right.png)、[iPad 横屏菜单](final/ipad-streaming-safe-menu-landscape-left.png)、[iPad 竖屏](final/ipad-streaming-two-rails-portrait.png)。两张历史诊断图片单独保存在 `diagnostics/`。

[验证摘要](validation.json)、[源码哈希](source-hashes.json)、[本轮差异](r6-task-changes.diff)、[窗口几何](observed-safe-frames.json) 与 [清理记录](cleanup.json) 共同记录本轮结果。最终截图的逐项哈希见 [保留清单](retained-artifacts.json)。

本轮临时目录 1.58 GiB 与 22 份已替代证据完成清理；`target/ios-webrtc` 和 `target/ios` 共享缓存保留，目录及保留证据哈希已核验。

实机使用新构建确认菜单配色、触屏遮挡和真实串流期间的操作手感。
