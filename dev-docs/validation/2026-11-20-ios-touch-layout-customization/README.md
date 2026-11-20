# iOS 触屏布局模板与可视化调整验收

2026-11-20。[RFC](../../../docs/rfcs/2026-11-20-ios-touch-layout-customization.md) · [Report](../../../docs/reports/2026-11-20-ios-touch-layout-customization.md)。

## 交付行为

- “控制器 → 触屏”保留显示模式，提供通用 Xbox / 本地模板选择，以及“自定义布局”“触屏手柄调整”两个导航子页；正式页面隐藏测试入口。
- 编辑页直接拖动标准控件，横竖屏分别保存位置，支持命名、更新、另存、恢复位置及删除。保存后自动选中；返回时丢弃草稿。模板最多 20 项。
- 调整页渲染当前模板，透明度 20%～100%、大小 75%～150% 实时预览，保存后供串流使用。外观随模板独立保存，通用 Xbox 继续保留自身外观偏好。
- 标准资源、现有 Surface、输入累加器和 StreamInputSession 继续共用。最终 sockets 同时决定绘制、命中、无障碍范围与摇杆内部距离；竖屏手柄始终位于下半区，命中直径至少 44pt。
- 移动偏好保留 v2 键和默认字段迁移；旧测试布局回到通用 Xbox，损坏模板独立过滤，有效模板及其他设置保留。

## 验证结果

| 验证 | 结果与来源 |
| --- | --- |
| Swift 单元 | 91 项通过，含 7 项新增模板/存储/几何/释放回归；`unit-tests.txt` 为 r3 的实际单元阶段输出 |
| iPhone 18 Pro | 5 个独立 UI 流程通过；入口与输入工具、正式竖屏串流来自 r2，模板完整流程来自 r4，竖屏编辑/取消/删除来自 r6；`accepted-ui-tests.txt` 逐项记录来源 |
| iPad Pro 13-inch (M5) | r7 两项关键子页 UI 测试通过，覆盖拖动、保存、切换、重启、外观预览、竖屏边界、草稿取消与删除回退；`ipad-summary.json` |
| iPhoneOS 构建 | 最终 r7 Debug / arm64 构建通过，最低 iOS 26.0，`CODE_SIGNING_ALLOWED=NO`；`device-build-summary.txt` |
| 静态检查 | Swift 格式、`pnpm lint:fix`、工程文件、串流会话边界和 diff 检查通过 |
| 源码与截图 | 10 个验收源码 SHA-256 一致；10 张最终截图和 1 张定位诊断图已归档 |

`validation.json` 汇总最终验收口径；`manifest.json` 记录最终截图所属设备、轮次和测试。`phone-portrait-summary.json` 是最终竖屏测试的原生摘要。

`phone-r4-summary.json` 保留该轮完整原生摘要：模板流程通过，删除定位失败；后续 r6 已完成该流程。其他初轮编译、设备占用、事件合成超时、缩放容器范围和确认面板定位问题见 `failure-diagnostics.json`。这些历史失败保留其原始状态，最终接纳的逐项结果以 `accepted-ui-tests.txt` 为准。

预览为离线黑色游戏画布，使用正式触屏资源、绘制和输入组件；界面测试经 DEBUG 夹具进入内部工具或正式串流覆盖层。真实本地/云游戏的触控手感与外接手柄共同操作接续用户实机验收。

## 临时产物收尾

本轮临时根 `/private/tmp/xbxrc-ios-touch-layout-customization-20261120` 的 r1～r7 已在构建、测试、诊断进程结束后清理，包含 DerivedData、xcresult、重复导出、录屏、日志与一次性格式配置。目录原占用 1553.44 MiB，清理后可用空间净增 1562.50 MiB，剩余约 61.80 GiB。

必要截图、摘要、失败诊断与源码哈希保留；共享 `target/ios`、`target/ios-bindgen`、`target/browser`、WebRTC SourcePackages 和共享 Swift 格式配置继续保留。路径与空间核验见 `cleanup.json`。
