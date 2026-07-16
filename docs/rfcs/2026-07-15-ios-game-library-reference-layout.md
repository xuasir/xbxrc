# iOS 游戏库参考布局 RFC

## Status

- Completion: 已完成
- Current State: completed
- Owner: agent
- Last Updated: 2026-07-15

## Background

iOS 游戏库 Tab 当前固定显示空状态，已有 `XboxDataStore.games` 已提供 TitleHub 游戏历史、Artwork/Hero、最近游玩时间、游玩时长和成就进度。用户提供两张参考图，要求将游戏库改造成沉浸式内容首页：顶部使用通顶最近游玩轮播，下方按多个维度组织横向栏目，栏目标题进入对应全量列表页。

参考图一的关键结构是状态栏后方全宽 Hero、底部信息与分页指示、随后连续排列横向栏目。参考图二的关键结构是透明导航区、大号居中栏目标题与纵向宽卡列表。

本轮使用现有 `GameSummary` 数据完成页面信息架构和视觉交互，并保持未来 `CloudLibraryStore` 接入时可替换数据源。

## Goal

- 移除游戏库首页现有导航页头标题。
- 顶部实现延伸到状态栏后的最近游玩 Hero 轮播。
- 下方按最近、时长、成就和全部四个维度呈现游戏数据。
- 每个栏目标题可进入参考图二风格的全量列表页。
- 覆盖登录、加载、失败、空数据、局部错误、刷新与图片降级状态。
- 保持 iOS 26 SwiftUI、Liquid Glass、AppTheme、动态字体、VoiceOver 和 Reduce Motion 一致性。

## Scope

- In scope:
  - `iosapp/XBXRC/Features/Library/GameLibraryView.swift` 首页状态与导航。
  - 新增游戏库纯展示模型和分组排序逻辑。
  - 新增 Hero、Shelf、Poster、栏目列表和宽卡组件。
  - `iosapp/XBXRCTests/XBXRCTests.swift` 分组、排序、截断和完整列表测试。
  - Xcode 工程 Sources 登记。
  - iPhone 纵向布局、辅助功能与视觉截图验收。
- Follow-up scope:
  - xCloud 目录 Bridge、缓存和 `CloudLibraryStore` 数据源由共享目录任务承载。
  - 游戏卡片启动真实云串流由 StreamingRuntime 任务承载。
  - iPad 多栏与横屏专属信息架构进入后续适配任务。

## Layout Contract

### Home page

- 根页面隐藏 navigation title 和 navigation bar 背景，保留同一 `NavigationStack` 的 push 能力。
- 纵向 `ScrollView` 延伸到顶部安全区，Hero 从屏幕 `y=0` 开始。
- Hero 高度使用 `clamp(480pt, viewportHeight × 0.54, 540pt)`；窄屏按可用高度收缩，确保下方首个栏目标题可在首屏边缘出现。
- 最近游玩按 `lastPlayedAt` 降序取最多 5 项；日期缺失项按服务端原顺序补位。
- Hero 图片优先 `heroURL`，回退 `artworkURL`；全宽 `scaledToFill` 裁切。
- 顶部叠加轻暗遮罩保证状态栏可读，底部通过黑色渐变与 `AppTheme` 画布连续融合。
- 信息区位于 Hero 底部，左右 20pt，展示游戏名、“最近游玩时间 · 游玩时长”和成就进度摘要。
- 轮播支持横向分页、吸附、分页圆点和选择触觉反馈；常规模式每 6 秒切换，用户拖动后暂停自动切换；Reduce Motion 使用静态分页与短淡入淡出。
- 点击 Hero 进入现有游戏成就详情，后续可替换为统一游戏详情/串流入口。

### Shelves

- Hero 下方依次呈现以下栏目：
  1. `最近游玩`：按 `lastPlayedAt` 降序。
  2. `玩得最多`：具有 `playtimeMinutes` 的游戏按时长降序。
  3. `成就进度`：具有 `achievementProgress` 的游戏按百分比降序，完成度相同时按已获 Gamerscore 降序。
  4. `全部游戏`：按本地化名称排序。
- 每栏首页最多展示 8 项，目的列表页使用完整集合。
- 只有空集合栏目隐藏，全部游戏作为最终兜底栏目。
- 栏目间距 28–32pt，标题行水平 16pt，点击热区至少 44pt。
- 标题行展示栏目名、完整条数和 chevron，整行使用 `NavigationLink`。
- 横向 Poster 卡宽 112pt、高 168pt、圆角 14pt，首尾 inset 16pt，间距 12pt；标题最多两行，附加信息一行。
- `最近游玩` 可使用横向 250×148pt Hero 卡强化继续游玩语义，其余栏目使用 2:3 Poster。

### Collection page

- 目的页沿用 `AppThemeBackground`，导航栏背景透明，系统返回按钮保持 iOS 26 Liquid Glass 和边缘返回手势。
- 顶部内容区显示居中 40–44pt 粗体栏目标题，Accessibility 字号下允许缩放和换行。
- push 后隐藏 TabBar，返回首页时恢复。
- 列表使用 `LazyVStack`，水平 16pt，行间距 12pt。
- 宽卡最小高度 108pt、圆角 18pt；图片 128×80pt，优先 Hero、回退 Artwork。
- 文字区展示名称、最近游玩/时长与当前栏目对应的维度摘要。
- 成就栏目卡片显示 4pt 品牌绿进度条；普通栏目保持精简。
- 卡片使用单层 `GlassEffectContainer` 和 `.regular.interactive()`，描边 0.5pt，并提供 44pt 以上点击区域。
- 点击游戏进入现有游戏成就详情。

## Presentation Model

新增纯 Swift 展示模型，职责集中在分组、排序和元数据格式：

```text
LibraryCollection
  id
  title
  kind
  games: [GameSummary]
  homeGames: prefix(8)

LibraryCollectionKind
  recent
  mostPlayed
  achievementProgress
  all
```

排序规则保持稳定：主排序字段相同时回退 `name.localizedStandardCompare`，再回退原始数组索引。首页截断只影响 Shelf，目的页始终保留完整集合。

未来 `CloudLibraryGame` 可映射到同一展示输入协议，UI 组件继续消费稳定的图片、标题、时间、时长和进度字段。

## State Contract

- 未登录：复用 `XboxLoginView`。
- 首次加载：Hero 区和两个 Shelf 使用固定尺寸骨架；Reduce Motion 下保持静态占位。
- 全量失败：显示主题画布上的错误空态与重新加载按钮。
- 空数据：显示“暂无游戏记录”空态。
- 局部错误：保留已加载内容，在 Hero 下方显示 `InlineDataErrorView` 风格提示并允许刷新。
- 下拉刷新：调用 `XboxDataStore.refreshLibrary()`，已有内容保持可见。
- 图片加载与失败：始终保持目标尺寸，使用 `quaternary + photo.fill` 占位，防止布局跳动。
- 会话切换：继续由 `XboxDataStore` 的 token generation 保护旧请求提交。

## Plan

1. 新增 `LibraryPresentation.swift`，实现栏目模型、稳定排序、首页截断和格式化。
2. 新增 `LibraryComponents.swift`，实现通顶 Hero 轮播、Shelf、Poster 卡、全量列表页和宽卡。
3. 重写 `GameLibraryView.swift`，接入登录/加载/失败/空/内容状态、刷新和导航。
4. 更新 Xcode Sources，追加纯展示逻辑 XCTest。
5. 完成 Swift parse、Device/Simulator build、XCTest、源码门禁与 `git diff --check`。
6. 在 iPhone 17 Pro 模拟器截取首页和栏目列表页，逐项对照参考图并迭代。

## Validation

- [x] 首页源码已移除“游戏库”导航页头；真实数据渲染由用户在模拟器验收。
- [x] Hero 容器忽略顶部安全区并覆盖完整 viewport 宽度。
- [x] 最近游玩可分页，分页指示与当前游戏同步，1 项数据保持稳定静态布局。
- [x] 页面至少展示最近与全部两个栏目，数据充分时展示四个栏目。
- [x] 每个栏目标题整行可点击，目的页条数等于该栏目完整集合。
- [x] 目的页实现大号居中标题、透明导航区、纵向宽卡列表结构。
- [x] 最近、时长、成就、名称排序与首页截断纯逻辑测试通过。
- [x] 登录、加载、失败、空数据、局部错误和刷新状态均有明确 UI。
- [x] Dynamic Type Accessibility Size 下列表切换为图上文下自适应布局。
- [x] VoiceOver 可读出栏目、游戏名、最近游玩、时长、成就进度与可执行动作。
- [x] Reduce Motion 停用自动轮播和按压动画。
- [x] Device 业务代码、完整 Simulator SDK、XCTest target 与 iPhone 17 Pro XCTest 通过。
- [x] `git diff --check` 通过，其他 iOS 页面和 Rust Bridge 保持现有行为。

## Risks

- Hero 图片比例和清晰度受 Xbox 服务图片质量影响；固定裁切与占位保持布局稳定。
- 自动轮播与用户手势可能产生选择竞争；交互暂停窗口和单一 selection 状态负责仲裁。
- 大字体会放大宽卡高度；列表允许自适应高度，Accessibility Size 使用图上文下布局。
- iOS 26 Liquid Glass 在大量列表行上存在合成成本；每个可见列表使用单层 `GlassEffectContainer`。
- 当前工作树包含未提交的 iOS 视觉抛光；本轮只增量修改 Library Feature、XCTest 和 Xcode Sources。

## Progress

- [x] Step 1: 已完成参考图、当前 UI、数据模型和历史约束审计。
- [x] Step 2: 已完成实施级布局、数据分组、状态和验收合同。
- [x] Step 3: 已完成首页、组件与展示模型。
- [x] Step 4: Swift parse、Device/Simulator build、XCTest 与源码门禁通过；用户接管真实账号模拟器视觉验收。
- [x] Step 5: Report 与任务追踪闭环。

## Execution Notes

- Date: 2026-07-15 | Status: planned
- Update: 已形成游戏库首页与栏目目的页的完整规格，确认现有 `GameSummary` 可支持本轮视觉与数据分组。
- Decision: 本轮复用 `XboxDataStore.games`，通过独立 Presentation 层隔离未来 xCloud 数据源替换；首页使用四个稳定栏目，目的页展示完整集合。
- Risk/Blocker: 实施按 task-run 复杂任务流程等待用户确认。
- Date: 2026-07-15 | Status: in-progress
- Update: 用户已确认执行，开始实现 Presentation、Hero/Shelf/List 组件与首页状态导航。
- Decision: 保持 Rust Bridge、AppRootView、AchievementsView 和 ProfileView 原状，只增量修改 Library Feature、XCTest 与 Xcode Sources。
- Risk/Blocker: 当前工作树包含同日未提交视觉改动，实施严格限制文件范围并采用增量工程配置编辑。
- Date: 2026-07-15 | Status: completed
- Update: 首页、Hero、四维度栏目、全量列表、状态、辅助功能与测试均已交付。
- Decision: 用户负责真实账号模拟器截图验收；实现阶段已通过完整 Simulator build 与 iPhone 17 Pro XCTest。
- Risk/Blocker: Hero 图片实际裁切与首屏密度等待用户视觉反馈后按截图微调。
