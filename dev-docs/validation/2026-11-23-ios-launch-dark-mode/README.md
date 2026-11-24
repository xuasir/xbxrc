# iOS 启动画面深浅色验收

2026-11-23。系统启动屏和 SwiftUI 恢复层共用 `LaunchBackground`：浅色 `#FFFFFF`，暗色采用应用画布色 `#202E27`。系统首帧按系统外观选择，SwiftUI 接管后按有效应用外观选择，支持应用设置覆盖系统外观。

[验收摘要](validation.json)记录生产启动视图与资源的隔离模拟器构建、8组背景像素和居中校验、手机/iPad深浅色视觉复核、Swift格式/解析与资源检查。系统暗色首帧和SwiftUI截图的最大通道差为4/255；两阶段均呈现深色背景。

保留[手机系统暗色首帧](phone-system-dark.png)、[手机应用暗色启动层](phone-app-dark.png)、[手机系统浅色首帧](phone-system-light.png)、[手机应用浅色启动层](phone-app-light.png)、[平板暗色](tablet-app-dark.png)和[平板浅色](tablet-app-light.png)。

[清理记录](cleanup.json)记录验收应用卸载、模拟器原外观恢复及本轮临时根目录清理；源代码哈希与原暂存区核验见验收摘要。实机冷启动过渡由用户手动确认。
