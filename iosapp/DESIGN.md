***

version: alpha
name: XBX Remote Client (iOS)
description: Xbox 远程客户端 iOS 版（XBXRC）视觉设计系统。所有令牌均提取自 iosapp 真实 SwiftUI 实现，非臆造。品牌为 Xbox 绿、暗色优先且完全适配 light/dark。
colors:
brand: "#107C10"
accent: "#0FAB21"
canvas: "#0F0F10"
canvasTop: "#0D1B12"
canvasBottom: "#12161E"
surfaceElevated: "#1A1B1E"
surfacePanel: "#2B2B2B"
textPrimary: "#FFFFFF"
textSecondary: "#D1D1D6"
textTertiary: "#A1A1AA"
textDisabled: "#71717A"
success: "#2BC24A"
warning: "#FDB900"
danger: "#FF5252"
typography:
titleBold: system .title bold
headlineSemibold: system .headline semibold
subheadline: system .subheadline
footnoteMedium: system .footnote medium
platform: SwiftUI (iOS, dark/light adaptive)
scope: iosapp 客户端现状提取
---------------------

# Design System: XBX Remote Client (iOS)

> **来源与范围说明**：本文件仅登记 `iosapp/` SwiftUI 客户端当前真实存在、可由代码直接证实的视觉令牌与组件惯例。与桌面版 `xbx-design` 系统（`.agents/skills/xbx-design/`）共用同一 Xbox 视觉语言（同为 `#107C10` 绿、暗色优先、表面分级、状态色一致），构成跨端一致的设计语言。未包含任何新增设计建议。

***

## Overview

面向 iOS 的 Xbox 远程游玩客户端视觉基调：**暗色优先、品牌绿为动作色、以渐变光晕背景营造深邃感**。

- **调性**：深色深邃 + 品牌绿高亮，冷静、游戏向。完整支持 light/dark（`UIColor` 按 `userInterfaceStyle` 返回不同分量），无强制锁定。

- **背景哲学**：不依赖纯色画布，而是 `AppThemeBackground` 通过三层 `LinearGradient`（top→canvas→bottom）+ 双 `RadialGradient` 光晕（品牌绿置左上、冷色青置右下）+ 半透明浮雕启动图标叠加构成身份化背景。`AllowsHitTesting(false)`、`accessibilityHidden(true)`。

- **输入与动效**：默认 SwiftUI 交互语义；所有可访问动效尊重 `AccessibilityReduceMotion`（reduce 时缩短/跳过位移动画）。

- **启动体验**：白底 + `LaunchIcon` 缩放淡出（`easeInOut`，普通 0.42s / reduce 0.22s，图标放大至 scale 7），`zIndex(100)` 覆盖。

***

## Colors

全部取自真实代码。品牌示意：`color(red:16,green:124,blue:16)`= `#107C10`；`Assets.xcassets` 的 `AccentColor` 实际分量 `0.060/0.670/0.130` ≈ `#0FAB21`（更亮的品牌绿），代码常量统一用 `#107C10`。

### Brand

| 令牌      | 取值        | 用途                                           |
| ------- | --------- | -------------------------------------------- |
| `brand` | `#107C10` | 主题 tint、登录图标、action 按钮 glass tint、SVG 浮雕图标颜色 |

### Canvas / Background（`AppThemePalette`）

| 令牌             | 深色                   | 浅色                     | 用途      |
| -------------- | -------------------- | ---------------------- | ------- |
| `canvas`       | `#0F0F10`            | `#F4F7F5`              | 主画布底色   |
| `canvasTop`    | `#0D1B12`            | `#E8F3EB`              | 背景渐变的顶部 |
| `canvasBottom` | `#12161E`            | `#EBF1F8`              | 背景渐变的底部 |
| `brandAura`    | rgba(16,124,16,0.22) | rgba(43,194,74,0.16)   | 左上品牌光晕  |
| `coolAura`     | rgba(55,99,118,0.18) | rgba(129,172,196,0.14) | 右下冷色光晕  |

### Semantic（`XBXProfileTokens`）

**状态色**

| 令牌        | 取值        | 用途       |
| --------- | --------- | -------- |
| `success` | `#2BC24A` | 成功 / 在线  |
| `warning` | `#FDB900` | 警告 / 降级  |
| `danger`  | `#FF5252` | 错误 / 破坏性 |

**表面**

| 令牌                | 深色        | 浅色    |
| ----------------- | --------- | ----- |
| `surfaceElevated` | `#1A1B1E` | White |
| `surfacePanel`    | `#2B2B2B` | White |

**文字四级**

| 令牌              | 深色        | 浅色        |
| --------------- | --------- | --------- |
| `textPrimary`   | White     | `#0F0F10` |
| `textSecondary` | `#D1D1D6` | `#55555E` |
| `textTertiary`  | `#A1A1AA` | `#7C7C88` |
| `textDisabled`  | `#71717A` | `#A9A9B2` |

**边界**

| 令牌             | 取值                          |
| -------------- | --------------------------- |
| `borderSubtle` | 深色 white 8% / 浅色 black 8%   |
| `divider`      | 深色 white 12% / 浅色 black 12% |

**媒体上层文字（over media，恒白）**

| 令牌                 | 取值        |
| ------------------ | --------- |
| `onMediaPrimary`   | White     |
| `onMediaSecondary` | White 76% |
| `onMediaTertiary`  | White 58% |

***

## Typography

使用 SwiftUI 系统语义字体，无自定义字体。层级通过相机名 + 语义 weight 表达：

| 语境            | SwiftUI 字体                                        | 出处                                  |
| ------------- | ------------------------------------------------- | ----------------------------------- |
| 主标题           | `.title.bold()`                                   | `XboxLoginView`（"连接 Xbox"）          |
| 按钮 / 强调文本     | `.headline.weight(.semibold)`                     | `XboxLoginButtonStyle`、玻璃 action 按钮 |
| 次级说明          | `.subheadline` `.foregroundStyle(.secondary)`     | 登录副标题                               |
| 错误提示          | `.footnote.weight(.medium)` + `.red`              | 登录错误条                               |
| Gamerscore 币值 | `.system(size:9, weight:.black, design:.rounded)` | `GamerscoreCoinIcon`                |

文字语义颜色映射：`.foregroundStyle(.primary/.secondary)` 走系统色阶；显式令牌取 `XBXProfileTokens.text*`。

***

## Elevation

层级主要依赖**表面明度差与系统 glass**，其次为受控阴影：

- **表面分级**：`surfacePanel`(基) → `surfaceElevated`(升)。overlay 卡片/面板用 `surfaceElevated`。

- **品牌发光**：登录手柄图标阴影 `shadow(brand 22%.opacity, radius 18, y 8)`，是品牌对象的签名辉光。

- **玻璃效**：iOS 26 `glassEffect(.regular)` 用于按钮与错误条；`interactive()` + `tint(brand)` 表达动作对象。

- **轨道卡片**：`OrbitCardCarousel` 使用 scale/opacity/rotate/offset + `mask` + 描边 `stroke` + 轻阴影构造景深，卡片之间通过位移与 z-index 分层。

- **浮雕**：背景启动图标使用 `blur` + `opacity`（深色 0.035 / 浅色 0.10）制造水印级深度。

***

## Components

### AppThemeBackground（全局画布）

三层线性渐变 + 双径向光晕 + 浮雕 `LaunchIcon`，随 `colorScheme` 切换深浅，见 **Canvas** 与 **Elevation**。

### Glass Action 按钮

胶囊形（`contentShape(Capsule())`，minHeight 54），`.foregroundStyle(.primary)`，`glassEffect(.regular.interactive().tint(brand))`，按下 `scale(0.975)` + `brightness(-0.03)`，动画 `.snappy(0.22)`（reduce 关闭）。登录按钮字体 `.headline.weight(.semibold)`。

### Hero 轮播（RecentlyPlayedHeroCarousel）

高度由 `LibraryLayoutMetrics` 动态：`heroMinimumHeight=480` / `heroMaximumHeight=540` / `heroViewportFraction=0.54`；海报 112×168、圆角 14；recent 卡 250×148。轮播拖拽 `withAnimation` `.easeOut(0.16)` 与 `.snappy(0.68, extraBounce:0.06)`。

### 轨道 / 书架（LibraryShelf）

横向 `LazyHStack(spacing:12)`，横向内边距 16，标题 + 计数 + 箭头，按压态 `LibraryPressStyle`。

### 卡片

- **游戏海报卡**：poster 112×168、圆角 14，媒体上层文字用 `onMedia*` 白系。

- **轨道卡片**：圆角 + `mask` + 描边 `stroke` + 轻阴影，见 Elevation。

### Gamerscore 币图标（GamerscoreCoinIcon）

小型徽章：`.green` 填充、白色 `.black .rounded` 9pt 数字，形式为金币角标。

### 错误提示条（登录）

`.red` 前景 + `Label`（`exclamationmark.triangle`），`glassEffect(.regular.tint(red 10%))`，圆角 14，横向内边距 16 / 竖向 12。

### 个人信息（Profile）

沉浸式 hero，头像尺寸 92，卡片 `cardRadius=12`，页面内边距 `pageInset=16`；状态色取自语义令牌。登出需确认。

### 导航

`TabView` 四段：游戏库 `/rectangle.stack.fill`、主机 `/desktopcomputer`、成就 `/trophy.fill`、我的 `/person.crop.circle.fill`，tint = `AppThemePalette.brand`。播放时为全屏 `StreamingPlayerView`（`ignoresSafeArea`）隐藏 Tab。

***

## Do's & Don'ts（当前现状惯例）

**Do**

- 品牌绿 `#107C10` 只用于动作 / 高亮 / tint，不在组件上用大面品牌色做装饰。

- 颜色一律走语义令牌（`XBXProfileTokens`、`AppThemePalette`）并随 dark/light 自适应；不硬编码单模式颜色。

- 状态色（success/warning/danger）只作用于值或图标，不作用于整行/标签底色。

- 按钮与交互对象用系统语义字体 + `glassEffect`，按压反馈统一缩放 + 亮度。

- 所有自定义位移动画防护 `reduceMotion`。

- 页面布局沿用 `pageInset=16`、卡片圆角 12–14、胶囊按钮等既有节奏。

**Don't**

- 不引入独立于上述 `AppThemePalette` / `XBXProfileTokens` 的平行色板；新颜色应复用现有令牌。

- 不绕过 `glassEffect` 自造按钮视觉。

- 不使用骨架屏假摩擦音暗示；等待态走系统 `ProgressView`。

- 媒体上层一律用 `onMedia` 白系，不加深色文字。

