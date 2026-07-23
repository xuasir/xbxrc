# iOS Auth Gate Visual System Unification Report

## Summary

- Related RFC: [`docs/rfcs/2026-07-22-ios-auth-gate-visual-system-unification.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-07-22-ios-auth-gate-visual-system-unification.md)
- 已完成主机页/个人资料骨架屏、深浅色外观设置、三套应用图标切换、深色背景纹理降对比度和登录背景圆边移除。

## Delivered

- 主机页和个人资料页首次加载展示固定几何骨架，复用 `SkeletonPulse` 并支持 Reduce Motion/VoiceOver。
- 设置新增“亮色 / 暗色 / 跟随系统”外观选择和三套应用图标选择，选择持久化并在根视图立即生效。
- 登录背景移除两条圆形描边，暗色主题 LaunchIcon 纹理降至低对比度并增加模糊。

## Changes

- `HostListView` 使用主机 Hero、信息行和操作按钮骨架；`ProfileView` 使用头像、身份、统计和内容占位，并在加载期间保持布局稳定。
- `AppSettingsStore` 增加 `AppAppearanceMode`、`AppIconPreset`、UserDefaults 合同和成功后提交的 `setAlternateIconName` 流程；`XBXRCApp` 注入 `preferredColorScheme`。
- `AppIconForest.appiconset`、`AppIconMidnight.appiconset` 与 PBX alternate icon 配置接入 Asset Catalog；`AppearanceSettingsView` 提供设置入口和错误恢复。

## Validation

- `find iosapp/XBXRC iosapp/XBXRCTests -name '*.swift' | xargs xcrun swiftc -parse` 通过。
- Asset Catalog `Contents.json` 经 `jq` 校验，Info.plist/PBX 经 `plutil -lint` 校验，三套备用图标经 `actool` 编译并生成 alternate icon 元数据；PNG 尺寸均为 1024×1024。
- `git diff --check`、圆边残留源码门禁和定向设置/主题检索通过。

## Risks

- 当前机器 CoreSimulator/SwiftPM 缓存目录权限阻断完整 `xcodebuild`；沙箱外重试因审批服务 503 未获授权。
- `setAlternateIconName` 需要在支持备用图标的真机或完整 iOS 运行环境完成一次成功、失败和取消路径验收。
- 深浅色与最大 Dynamic Type 的截图验收需要可用 Simulator/Device 环境。

## Follow-up

- 在可用 Xcode 缓存和 CoreSimulator 环境执行 Device/Simulator `build-for-testing` 与 XCTest。
- 在真机验证三套图标切换、重启恢复、系统外观联动及深浅色截图矩阵。
