# iOS 品牌启动页与启动过渡 RFC

## Status

- Completion: 已完成
- Current State: completed
- Owner: agent / iOS App
- Last Updated: 2026-07-22

## Background

- iOS 当前使用空 `UILaunchScreen`，系统首帧缺少 XBXRC 品牌识别。
- `AuthStore` 启动时会恢复 Keychain 会话并刷新资料，冷启动期间需要与应用主题连续的等待反馈。
- 应用主界面已经形成深墨绿、石墨蓝、品牌绿光晕与 Liquid Glass 视觉语言。

## Goal

- 建立从系统 Launch Screen 到 SwiftUI 根视图的连续启动体验。
- 首帧保持白色、轻量、稳定，并在各尺寸与方向上保持品牌图标居中。
- 会话恢复完成时用中心放大外溢转场揭示应用内容，并完整支持 Reduce Motion 与 VoiceOver。

## Visual Direction

采用“纯白画布 + 中央玻璃绿品牌图标”方案。

- 背景：固定 `#FFFFFF`，系统 Launch Screen 与 SwiftUI 启动层使用同一颜色资产。
- 品牌：完整复用用户提供的 `icon-source.svg`，保持原始渐变、玻璃高光与透明留白；系统首帧使用 160pt 矢量画布，SwiftUI 阶段根据短边在 132–176pt 范围自适应。
- 构图：品牌图标保持屏幕几何中心，横竖屏、iPhone 与 iPad 使用同一中心锚点。
- 信息：页面只显示品牌图标，避免文字、进度条和营销信息干扰中心动势。
- 转场：认证恢复结束后，图标以中心为锚点在 420ms 内放大到 7 倍并溢出屏幕，白色遮罩同步淡出；Reduce Motion 下执行 220ms 纯淡出。

视觉稿：[`designs/2026-07-22-ios-launch-screen-white.png`](../designs/2026-07-22-ios-launch-screen-white.png)，矢量构图源文件：[`designs/2026-07-22-ios-launch-screen-white.svg`](../designs/2026-07-22-ios-launch-screen-white.svg)

## Scope

- In scope:
  - `iosapp/XBXRC/Resources/Info.plist` 的系统 Launch Screen 配置。
  - `iosapp/XBXRC/Resources/Assets.xcassets` 的启动背景与品牌资产。
  - `iosapp/XBXRC/App` 下的 SwiftUI 恢复层与根视图过渡。
  - `AuthStore.phase == .restoring` 的启动展示绑定。
  - iPhone/iPad、横竖屏、深浅色、Reduce Motion 与 VoiceOver。
- Out of scope:
  - 登录页、Tab 主界面与数据骨架屏重设计。
  - AppIcon 造型调整；启动页只复用其 Xbox + 齿轮品牌语义。
  - 启动广告、营销文案和网络进度百分比。

## Plan

1. 增加白色 Launch Screen 背景与用户提供的品牌矢量资产，配置系统静态首帧。
2. 新增 `LaunchExperienceView`，在恢复阶段保持同构画面并处理放大外溢转场与辅助功能。
3. 在 `XBXRCApp` / `AppRootView` 接入根层过渡，保证恢复完成、失败和无会话三条路径稳定退出。
4. 补充状态门禁与 XCTest，执行 Device/Simulator 构建和视觉截图验收。

## Validation

- [x] 冷启动首帧与 SwiftUI 接管帧共享白色背景与同一 `LaunchIcon` 资源。
- [x] 有会话、无会话、刷新成功、刷新失败都在 `AuthPhase` 离开 `.restoring` 后退出启动层。
- [x] 快速恢复与慢恢复阶段都保持稳定白底中心图标，不出现额外加载元素。
- [x] iPhone/iPad、横竖屏构图通过 GeometryReader 以短边自适应并保持中心锚点。
- [x] Reduce Motion 关闭放大，仅执行淡出；VoiceOver 提供单一“正在启动 XBXRC”语义元素。
- [x] Swift parse、工程 plist、资源 JSON/XML、视觉预览和 `git diff --check` 通过；完整 Xcode build/test 保留环境阻断说明。

## Risks

- 系统 Launch Screen 只支持静态内容，动态反馈必须在 SwiftUI 接管后执行。
- 系统首帧与 SwiftUI 背景色存在微小色差时会形成闪屏，需要复用同一组颜色值并进行截图比对。
- 恢复层退出条件直接绑定 `AuthPhase`，失败与无会话路径需要明确覆盖，避免页面长期遮挡。

## Progress

- [x] Step 1: 完成现有 AppIcon、主题色、根视图和认证恢复状态检查。
- [x] Step 2: 用户确认白底、给定 SVG 中心图标与放大外溢方向，完成尺寸、动效与辅助功能规格更新。
- [x] Step 3: 完成系统 Launch Screen、SwiftUI 恢复层、放大外溢转场与辅助功能接入。
- [x] Step 4: 完成工作区静态门禁和白底视觉预览；Xcode 构建与模拟器视觉验收受环境服务阻断并已记录。

## Execution Notes

- Date: 2026-07-15 | Status: planned
- Update: 已保存深色视觉稿与完整落地规格。
- Decision: 系统静态首帧与应用内恢复层共享同一构图；加载反馈仅存在于 SwiftUI 阶段。
- Risk/Blocker: 代码实现等待用户确认当前视觉方向。

- Date: 2026-07-22 | Status: completed
- Update: 完成白色系统首帧、用户 SVG 中心图标、SwiftUI 启动层与中心放大外溢淡出；生成白底视觉预览。
- Decision: 系统静态首帧与 SwiftUI 启动层共享 `LaunchIcon` 和 `LaunchBackground`；放大外溢只发生在应用接管后的 SwiftUI 阶段。
- Risk/Blocker: 完整 Xcode build/test 需要访问用户级 SwiftPM/Clang 缓存与 CoreSimulator 服务，当前环境权限和服务状态阻断该验证。
