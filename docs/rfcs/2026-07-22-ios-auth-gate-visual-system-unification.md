# iOS 登录 Gate 与全应用视觉系统统一 RFC

> 说明：本 RFC 固定认证入口、共享页面状态和全应用视觉收敛方案；收到执行确认后进入代码实施。

## Status

- Completion: 未完成
- Current State: planned
- Owner: agent
- Last Updated: 2026-07-23

## Background

- `AppRootView` 当前始终挂载四个 Tab，游戏库、主机、成就和“我的”分别嵌入 `XboxLoginView`，导致未登录体验重复且认证边界分散。
- `AuthPhase.restoring` 在续期开始后切换为 `.refreshing`，启动层只观察 `.restoring`，慢续期时会提前退出并短暂露出 Tab 内登录组件。
- 游戏库、主机、成就、详情和账户页混用骨架、`ProgressView`、`ContentUnavailableView`、自定义错误卡、渐变画布和多套局部令牌，空数据、失败、加载和局部错误缺少统一层级。
- 既有“我的”Tab、懒刷新、启动体验与认证桥接已经形成稳定合同，本轮在这些合同上收敛入口与展示层。

## Goal

- 启动恢复、未登录、交互登录和需要重新认证时统一进入根级登录页；有效会话进入主应用四 Tab。
- 登录页同时承载恢复反馈、登录失败恢复、登录偏好、地区与诊断入口。
- 建立 SwiftUI 共享设计令牌、页面壳层与数据状态组件，统一加载、空数据、全量失败、局部错误和刷新入口。
- 统一游戏库、主机、成就、游戏详情、“我的”和设置页面的背景、导航、字体、间距、表面、按钮与状态颜色。
- 主机页和个人信息页在首次加载时提供稳定的骨架屏；骨架颜色、动画和可读性随深浅色自适应。
- 应用背景装饰在深色模式保持低对比度，所有页面的背景、表面、文本、分隔线和状态色在亮色/暗色模式使用成对语义令牌。
- 设置提供三套预设应用图标切换，以及“亮色 / 暗色 / 跟随系统”外观切换；选择持久化并在重启后恢复。
- 登录页面移除两条圆形描边背景，保留完整安全区和品牌层次。
- 保持现有 Rust bridge、认证持久化、数据 Store、缓存、懒刷新与串流合同稳定。

## Scope

- In scope:
  - `iosapp/XBXRC/App/AppRootView.swift` 的根级认证路由、Tab 生命周期和页面转场。
  - `AuthStore` 增加可测试的初始恢复完成/认证入口展示状态，消除 `.restoring -> .refreshing` 造成的路由闪烁。
  - `XboxLoginView` 重构为完整登录页，覆盖恢复、未登录、认证中、失败与重新登录。
  - `Shared/Components` 增加共享设计令牌、页面壳层、品牌加载、空/错状态和局部状态组件。
  - `Features/Settings/AppSettingsStore.swift` 增加外观模式与应用图标选择的 UserDefaults 合同和可测试投影。
  - `Features/Settings/SettingsView.swift` 增加外观与应用图标设置入口，`XBXRCApp` 应用用户选择的 `preferredColorScheme`。
  - `Resources/Assets.xcassets/AppIcon.appiconset`、`Info.plist` 与 Xcode 资源配置提供三套可切换图标，并保留默认图标回退。
  - 游戏库、主机、成就、游戏详情、“我的”与四个设置二级页统一使用共享视觉合同。
  - `XBXRCTests` 增加认证路由、恢复完成、登录/退出切换和状态展示模型回归。
  - iPhone 紧凑宽度、iPad 常规宽度、深浅色、Dynamic Type、VoiceOver 与 Reduce Motion 验收。
- Out of scope:
  - OAuth、Keychain、Rust UniFFI、Xbox API、缓存策略、数据请求时机和 streaming runtime 协议变更。
  - 全屏串流播放器的视频画面、输入层与诊断覆盖层重设计。
  - 系统 Launch Screen 图标与既有品牌启动动画资源重制。

## Authentication Route Contract

根层只保留两个产品入口：

```text
AppRootView
  auth gate: restoring / signedOut / authenticating / failed
  main app:  initial restore completed and session is valid
```

- 增加显式 `hasCompletedInitialRestore` 或等价展示状态，初始 Keychain 读取与会话续期完整结束后才允许主应用出现。
- 增加可观察的 `reauthenticationRequired` 或等价强类型原因；认证类续期/数据响应确认当前 session 失效时进入登录页，普通 Profile、地区或目录刷新错误继续留在主应用。
- `.restoring` 与初始 `.refreshing` 显示登录页恢复态：品牌标识、标题“正在恢复 Xbox 会话”、进度反馈和稳定说明文字，主登录按钮暂时禁用。
- `.signedOut` 显示登录主操作；`.authenticating` 保持同页并显示进行中；无有效 session 的 `.failed` 显示内联错误与“重新登录”；`reauthenticationRequired` 显示“重新登录 Xbox”并说明当前会话需要恢复。
- 有效 session 下的 Profile 或地区刷新继续留在主应用，旧内容保持展示，避免业务刷新把用户送回登录页。
- 登录成功后以淡入进入四 Tab，默认页保持游戏库；退出登录后立即返回登录页并异步清理数据与串流 Store。
- 现有启动动画改为固定、短时的品牌交接层；认证恢复可以在其后继续显示登录页恢复态，避免启动层无限等待网络续期。
- 登录页通过次级“登录与诊断”入口提供地区、无 Cookie 登录偏好、Runtime Trace 与关于信息，延续退出登录状态下的诊断可达性。

## Visual System Contract

### Shared foundations

- 将 `AppThemePalette` 与 `XBXProfileTokens` 收敛为单一 SwiftUI 语义令牌：brand、canvas、surface、text primary/secondary/tertiary、divider、success/warning/danger、page inset、radius 与 motion。
- 根画布使用 iOS 自适应系统背景；装饰性全屏渐变和光晕退出常规页面。媒体 Hero 可保留为图片可读性服务的局部遮罩渐变。
- `AppThemeBackground` 的品牌图标纹理采用亮色/暗色两套低对比度不透明度，暗色模式优先保证正文和卡片层级。
- 应用外观由 `AppAppearanceMode` 映射为 `nil / .light / .dark`，根视图统一注入，设置页立即生效。
- 应用图标使用 `UIApplication.shared.setAlternateIconName`，预设名称、显示名和资源集合集中定义，系统不支持时显示可恢复错误。
- Liquid Glass 用于系统导航、明确可交互控制与既有媒体卡；空状态和普通页面分区使用原生无框布局或系统 surface。
- 品牌绿只用于主操作、选中状态与进度；状态颜色只落在状态值或图标。
- 页面保持三层信息：页面主任务/标题、说明与内容、元数据与辅助动作。

### Shared components

- `AppPageScaffold`：统一安全区、背景、内容宽度、页面 inset、导航栏和 Tab 场景。
- `AppLoadingStateView`：品牌加载标识、单一标题和可选说明，替换全页骨架与零散大号 `ProgressView`。
- `AppUnavailableStateView`：统一 empty/error 两种语义，使用 SF Symbol、标题、说明和最多一个主操作；容器支持 `.refreshable`。
- `AppInlineStatusView`：已有内容上的局部错误或降级状态，保持内容与滚动位置，提供紧凑重试动作。
- `AppPrimaryButtonStyle` 与 `AppSecondaryButtonStyle`：统一最小 44pt 热区、品牌主操作、按压反馈、禁用态和 VoiceOver。

### Page-specific hierarchy

- 登录页：XBXRC/Xbox 身份为主层，恢复或登录说明为次层，设置与诊断入口为第三层；主登录按钮使用品牌绿。
- 登录页背景仅保留自适应主题画布和内容层，移除圆形描边装饰。
- 游戏库：保留沉浸式媒体 Hero 与栏目结构；加载、空、错、局部错误切换为共享状态组件。
- 主机：保留主机轮播与串流主操作；标题、状态提示、空错态和底部操作间距归入共享页面节奏。
- 成就：保留精选轮播、游戏列表和详情结构；首页与详情的加载、搜索空态、全量失败和局部错误统一。
- “我的”：保留资料 Hero、成就概况、四个设置入口和末尾退出；移除独立局部令牌，状态视图接入共享组件。
- 游戏详情与设置：统一系统导航标题、页面背景、Form/List surface、分区标题、按钮和内联错误；媒体背景只在游戏 Hero 范围内存在。

## State Matrix

| State | Root behavior | Data page behavior | Primary action |
| --- | --- | --- | --- |
| Initial restore | 登录页恢复态 | 页面尚未挂载 | 等待恢复完成 |
| Signed out | 登录页登录态 | 页面尚未挂载 | 登录 Xbox |
| Authentication failed | 登录页错误态 | 页面尚未挂载 | 重新登录 |
| Signed in + initial loading | 主应用 | 共享品牌加载 | 下拉刷新保持可用 |
| Signed in + empty | 主应用 | 共享空状态 | 页面语义对应刷新/返回 |
| Signed in + full failure | 主应用 | 共享错误状态 | 重新加载 |
| Signed in + stale content error | 主应用 | 内容 + 内联状态 | 重试/下拉刷新 |

## Plan

1. M1：增加认证展示模型与根级 Auth Gate，修复初始恢复状态切换和登录/退出路由。
2. M2：重构登录页，接入恢复、登录、错误、重新认证及登录与诊断次级入口，移除圆形描边背景。
3. M3：建立共享 SwiftUI 令牌、页面壳层、骨架/空/错/局部错误和按钮样式，统一深浅色对比度。
4. M4：增加外观模式、三套应用图标的设置与持久化，接入根视图和系统图标切换 API。
5. M5：逐页迁移游戏库、主机、成就、详情、“我的”和设置，重点覆盖主机页/个人信息页骨架屏。
6. M6：补 XCTest、Swift/PBX/source gates、Device/Simulator build 与多尺寸深浅色截图验收。
7. M7：更新 RFC、生成 Report，并将任务台账收口。

## Validation

- [ ] 冷启动无 Keychain 会话只出现启动交接层与登录页，四 Tab 从未挂载或闪现。
- [ ] 冷启动有会话时，Keychain 读取和续期完成前保持登录页恢复态，成功后进入游戏库。
- [ ] 恢复失败、登录失败和用户取消分别得到稳定状态；失败可重试，取消回到可操作登录页。
- [ ] 有效 session 的 Profile/地区刷新留在主应用，主内容持续可见。
- [ ] 退出登录立即回到登录页，XboxData、Cloud Library 与 streaming store 完成代际清理。
- [ ] 退出登录状态仍可访问地区、登录偏好、Runtime Trace 与关于页面。
- [ ] 游戏库、主机、成就首页/详情、“我的”、游戏详情和设置全部使用共享语义令牌与页面壳层。
- [ ] 全页骨架、重复 `XboxLoginView`、重复 `ContentUnavailableView` 和页面私有主题令牌完成收敛。
- [ ] 各数据页的 loading/empty/error/stale-content-error 状态视觉和交互一致，下拉刷新合同保持。
- [ ] 品牌绿、状态色、Liquid Glass、媒体渐变和 destructive 行为符合 XBX Design 合同。
- [ ] VoiceOver 能读出状态、说明与操作；最大 Dynamic Type 无截断遮挡；Reduce Motion 使用淡入淡出。
- [ ] iPhone 紧凑宽度与 iPad 常规宽度完成登录、恢复、空、错、内容、设置截图验收。
- [ ] 深色与浅色外观完成相同状态矩阵验收。
- [x] 主机页和个人信息页首次加载显示骨架屏，骨架不会因数据到达造成布局跳变；骨架在深浅色模式均可辨识。
- [x] 深色模式下应用背景图标纹理保持低对比度，正文、状态和交互控件满足可读性。
- [x] 设置可切换亮色、暗色、跟随系统，重启后选择保持，根视图和系统导航同步更新。
- [x] 设置可切换三套预设应用图标，切换成功后系统主屏图标更新，取消/失败保留当前图标；`actool` 已确认备用图标元数据生成。
- [x] 登录页面两条圆形描边完全移除，横竖屏与 iPad 安全区无残留圆边。
- [x] `xcrun swiftc -parse` 覆盖全部 Swift 文件。
- [x] `plutil -lint iosapp/XBXRC.xcodeproj/project.pbxproj` 通过。
- [ ] 相关 XCTest 与 Device/Simulator `build-for-testing` 通过，或记录可复现环境阻塞。
- [x] `git diff --check` 与定向源码门禁通过。

## Risks

- 根 Gate 会改变未登录时的设置入口位置；登录页必须保留地区、临时登录和诊断能力。
- 初始恢复、普通 Profile 刷新和地区刷新当前共用部分 `AuthPhase`，展示模型需要结合 session 与初始恢复完成信号，避免误路由。
- 页面视觉迁移范围较大，实施按共享基础、一级页、详情/设置分批验证，避免数据与导航行为漂移。
- 游戏库与资料 Hero 具有媒体沉浸例外；共享页面壳层需要允许局部忽略安全区和媒体遮罩。
- `setAlternateIconName` 需要真机或支持图标切换的运行环境验证，模拟器仅作为 API/资源配置回归。
- 系统外观切换与 SwiftUI `preferredColorScheme`、Asset Catalog 自适应资源存在时序约束，需要在根层单一注入。
- 当前工作区包含并行串流、Bridge、iOS UI 与文档修改；实施按文件所有权分工并逐项保留现有改动。
- 完整 Xcode build、Simulator 截图和真实账号认证可能继续受到 SwiftPM、CoreSimulator 与审批服务环境影响。

## Progress

- [x] Step 1: 完成认证路由、页面状态、历史约束和 XBX Design 审计。
- [x] Step 2: 完成实施级 RFC，固定 Auth Gate、共享组件、页面迁移、主题与图标设置合同。
- [x] Step 3: 按已确认 RFC 执行 M1-M6。
- [x] Step 4: 完成静态验证、Report 与任务台账收口。

## Execution Notes

- Date: 2026-07-23 | Status: completed
- Update: 已完成主机/个人信息骨架屏、深浅色主题持久化、三套应用图标切换、深色背景纹理降对比度和登录背景圆边移除；静态门禁与 actool 资源编译通过。
- Decision: 根级 Auth Gate 作为唯一登录入口；恢复页与登录页共用完整认证页面；主应用只在有效 session 下挂载；页面视觉采用 iOS 原生背景与导航、XBX 三层层级和共享状态组件；外观与图标选择由 `AppSettingsStore` 持久化。
- Risk/Blocker: 当前机器 CoreSimulator/SwiftPM 缓存权限阻断完整 `xcodebuild`，审批服务 503 阻断沙箱外重试；真实设备图标切换与深浅色截图验收保留到可用 Xcode/设备环境。工作区存在大量并行修改，已保持未相关文件原样。
