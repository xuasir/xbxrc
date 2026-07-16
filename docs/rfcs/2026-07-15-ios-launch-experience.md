# iOS 品牌启动页与启动过渡 RFC

## Status

- Completion: 未完成
- Current State: planned
- Owner: agent / iOS App
- Last Updated: 2026-07-15

## Background

- iOS 当前使用空 `UILaunchScreen`，系统首帧缺少 XBXRC 品牌识别。
- `AuthStore` 启动时会恢复 Keychain 会话并刷新资料，冷启动期间需要与应用主题连续的等待反馈。
- 应用主界面已经形成深墨绿、石墨蓝、品牌绿光晕与 Liquid Glass 视觉语言。

## Goal

- 建立从系统 Launch Screen 到 SwiftUI 根视图的连续启动体验。
- 首帧保持轻量、稳定、可适配深色与浅色模式。
- 会话恢复超过短暂阈值时提供克制的加载反馈，并完整支持 Reduce Motion 与 VoiceOver。

## Visual Direction

采用“暗色空间画布 + 中央品牌信标”方案。

- 背景：沿用 `AppThemePalette`，深色从 `#0D1B12` 过渡至 `#0F0F10`、`#12161E`；浅色从 `#E8F3EB` 过渡至 `#F4F7F5`、`#EBF1F8`。
- 氛围：左上品牌绿光晕与右下冷灰蓝光晕，透明度保持在 14%–22%，让系统首帧自然衔接主界面。
- 品牌：中央提取现有 XBXRC AppIcon 的 Xbox 核心符号，视觉尺寸 80pt；外层使用 120pt 低对比度齿轮信标环，呼应 AppIcon 轮廓并避免形成第二个卡片层级。
- 文字：图标下方仅保留 `XBXRC`，22pt semibold，字距 0.14em；页面不承载标语和功能说明。
- 加载：系统 Launch Screen 保持静态；SwiftUI 恢复层在 250ms 后淡入 60pt 细线进度轨道，使用不确定进度动画。
- 转场：恢复完成后整体 220ms 淡出，内容层同步 0.98 → 1.0；Reduce Motion 下仅执行淡出。

视觉稿：[`designs/2026-07-15-ios-launch-screen-dark.svg`](../designs/2026-07-15-ios-launch-screen-dark.svg)

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

1. 增加可自适应深浅色的 Launch Screen 背景与品牌资产，配置系统静态首帧。
2. 新增 `LaunchExperienceView`，在恢复阶段延迟展示轻量加载反馈并处理辅助功能。
3. 在 `XBXRCApp` / `AppRootView` 接入根层过渡，保证恢复完成、失败和无会话三条路径稳定退出。
4. 补充状态门禁与 XCTest，执行 Device/Simulator 构建和视觉截图验收。

## Validation

- [ ] 冷启动首帧与 SwiftUI 接管帧无明显闪白、跳色或缩放跳变。
- [ ] 有会话、无会话、刷新成功、刷新失败四条路径都能退出启动层。
- [ ] 快速恢复时不出现一闪而过的加载条，慢恢复时 250ms 后出现反馈。
- [ ] 深色、浅色、iPhone/iPad、横竖屏构图保持中心稳定。
- [ ] Reduce Motion 关闭缩放与呼吸动画，VoiceOver 只播报一次“正在启动 XBXRC”。
- [ ] Xcode Device/Simulator build、XCTest 与 `git diff --check` 通过。

## Risks

- 系统 Launch Screen 只支持静态内容，动态反馈必须在 SwiftUI 接管后执行。
- 系统首帧与 SwiftUI 背景色存在微小色差时会形成闪屏，需要复用同一组颜色值并进行截图比对。
- 恢复层退出条件直接绑定 `AuthPhase`，失败与无会话路径需要明确覆盖，避免页面长期遮挡。

## Progress

- [x] Step 1: 完成现有 AppIcon、主题色、根视图和认证恢复状态检查。
- [x] Step 2: 完成视觉方向、尺寸、动效与辅助功能规格。
- [ ] Step 3: 等待视觉确认后实施系统 Launch Screen 与 SwiftUI 恢复层。
- [ ] Step 4: 完成构建、测试与模拟器视觉验收。

## Execution Notes

- Date: 2026-07-15 | Status: planned
- Update: 已保存深色视觉稿与完整落地规格。
- Decision: 系统静态首帧与应用内恢复层共享同一构图；加载反馈仅存在于 SwiftUI 阶段。
- Risk/Blocker: 代码实现等待用户确认当前视觉方向。
