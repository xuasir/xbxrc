# iOS“我的”Tab 信息架构合并方案

## Problem Framing

- iOS 当前把账户资料与应用设置分配在两个底部 Tab。账户页承载个人 Hero、活动与社交信息、成就概况、资料刷新和退出登录；设置页承载登录偏好、云游戏地区、诊断和版本信息。
- 两个入口都围绕“当前用户与应用偏好”展开，底部导航因此承担了过细的信息分类。合并后的目标是让“我的”成为稳定的个人中心，账号信息作为主内容，设置作为向下延伸的系统式入口。
- 资料刷新统一由系统下拉刷新接管。退出登录进入页面内容流末尾，并继续使用确认对话框保护会话操作。
- 详尽设置进入二级页面，主页只展示可扫描的名称、图标、当前值摘要和 disclosure indicator。

## Current Constraints

### Existing product contracts

- `ProfileView` 已有成熟的登录态 Hero、社交与活动信息、成就概况、错误降级和 `.refreshable` 刷新链路。
- 当前右上角菜单提供“刷新资料”和“退出登录”；菜单刷新同时更新 Profile 与 Xbox 游戏活动数据。
- `SettingsView` 已包含四类内容：登录偏好、云游戏地区、Runtime Trace 诊断、应用版本。
- 2026-07-16 设置 RFC 规定诊断入口在登录和退出状态都保持可达。合并后“我的”Tab 需要在未登录状态继续展示设置入口。
- 云游戏地区应用会触发设置持久化、Cloud Access 清理、认证续期与游戏库 scope 重建。UI 重组继续复用这一生命周期合同。
- Runtime Trace 导出使用系统分享页，Trace 清理继续使用 destructive confirmation。
- 2026-07-22 懒加载 RFC 规定远程数据页面只在当前 Tab 激活后请求；每个 surface 在当前 `ownerGeneration` 首次进入时后台刷新一次，Tab 往返、前后台切换和普通凭据续期保持零自动刷新。

### Architecture and delivery boundaries

- SwiftUI 负责 Tab、NavigationStack、页面状态与系统交互。
- `AuthStore`、`CloudLibraryStore`、`AppSettingsStore` 的职责与现有数据流保持稳定。
- Xbox 鉴权、Token、地区路由与目录协议继续位于 Rust 和现有 UniFFI bridge。
- Xcode 工程使用显式文件引用；页面改名或移动时需要同步 `project.pbxproj`。
- 当前工作区存在 iOS 串流与启动体验相关并行改动，实施阶段需要限制修改文件范围并保留现有变更。

## Options

### Option A：单一“设置”入口

- 主页面在成就概况下方只增加一行“设置”，点击后进入当前完整 `SettingsView`。
- 优点：修改量小，现有设置页可整体复用。
- 代价：主页信息密度偏低，用户无法直接看到地区、Trace profile 和版本摘要；二级页仍然是一个较长 Form。

### Option B：分类入口与对应二级页面

- 主页面在成就概况下方增加“云游戏”“登录偏好”“诊断”“关于”四行，每行展示当前值摘要并进入对应二级页面。
- 优点：结构贴近系统设置，扫描效率高，常用状态直接可见，各二级页拥有单一任务边界。
- 代价：需要拆分当前 `SettingsView`，并统一主页行样式、路由和共享状态。

### Option C：主页内联全部设置

- 现有设置控件全部放在成就概况下方，退出登录继续位于末尾。
- 优点：所有操作一步可达。
- 代价：账号主页长度和操作密度显著增加，Hero、成就与诊断工具形成视觉竞争，地区切换等长流程挤占主页面。

## Recommended Direction

采用 Option B。它完整表达“账号是主页面、设置是二级能力”的产品方向，也保留设置与诊断在未登录状态下的稳定可达性。

### Bottom navigation

底部 Tab 调整为四项：

1. 游戏库
2. 主机
3. 成就
4. 我的

“我的”继续使用 `person.crop.circle.fill`。`AppSection.profile` 收敛为 `AppSection.my`，独立 `settings` case 和 Settings Tab 从根 TabView 移除。

### My page information architecture

已登录状态从上到下排列：

```text
我的
├── 账户 Hero
│   ├── 头像 / 显示名 / Gamertag / Gamerscore
│   ├── 在线状态 / 当前活动 / 最近游玩
│   └── 好友 / 关注 / 粉丝
├── 成就概况
├── 设置与支持
│   ├── 云游戏        当前地区 · 云访问状态
│   ├── 登录偏好      标准会话 / 无 Cookie 临时会话
│   ├── 诊断          Trace 记录级别
│   └── 关于          XBXRC 版本号
└── 退出登录
```

未登录状态从上到下排列：

```text
我的
├── 登录 Xbox 账户
├── 设置与支持
│   ├── 云游戏        当前地区 · 未登录
│   ├── 登录偏好      标准会话 / 无 Cookie 临时会话
│   ├── 诊断          Trace 记录级别
│   └── 关于          XBXRC 版本号
```

资料暂不可用状态保留“重新加载”操作，同时继续展示“设置与支持”。这样认证服务故障不会遮蔽地区调整、Trace 导出和版本信息。

### Primary page rows

- 使用一个系统设置式 grouped list surface，四行共享 52–56pt 最小高度、16pt 水平内边距和单一分隔线规则。
- 每行结构固定为：左侧 SF Symbol、主标题、右侧单行摘要、`chevron.right`。
- 图标使用稳定语义与克制颜色：云游戏 `network`、登录偏好 `person.badge.key`、诊断 `waveform.path.ecg`、关于 `info.circle`。
- 摘要反映即时状态：地区与云访问状态、登录会话模式、Trace profile、版本号。长文本使用单行截断，并提供完整 VoiceOver value。
- 四行作为一个整体 surface，页面 section 保持未嵌套卡片结构。视觉继续使用现有主题 canvas、0.5pt separator 和 Liquid Glass 语言。
- 退出登录使用独立 section 的红色 label 行，图标为 `rectangle.portrait.and.arrow.right`，位置固定为全部内容最后一项。

### Secondary pages

#### 云游戏

- 页面标题：“云游戏”。
- 内容：地区路由 Picker、云访问状态、应用地区设置按钮、作用说明、应用中进度与脱敏错误。
- 复用现有 `applyRegion()` 生命周期：保存 preset → 清理 cloud scope → 刷新认证 → 重新激活游戏库。

#### 登录偏好

- 页面标题：“登录偏好”。
- 内容：“使用无 Cookie 临时会话”开关及现有生效说明。
- 未登录与已登录状态均可修改；当前 Xbox 会话按既有语义继续有效，新设置在下一次登录生效。

#### 诊断

- 页面标题：“诊断”。
- 内容：Trace 记录级别、导出当前 Trace、导出全部 Trace、清理 Trace。
- 导出进度、分享页、错误 alert 和清理 confirmation 继续由该页面拥有。

#### 关于

- 页面标题：“关于”。
- 内容：应用名称、版本与构建号。
- 首期保持纯信息页，后续许可、隐私与反馈入口可沿这一边界扩展。

### Refresh and account actions

- 已登录主页面保留一个 `.refreshable` 入口，同时刷新 Profile 与 Xbox 游戏活动数据。
- 系统下拉刷新指示器成为唯一手动刷新反馈。右上角 ellipsis 菜单、菜单刷新按钮和自定义顶部刷新浮层一并移除。
- “我的”成为当前 Tab 时同时调用 `activateProfileOnce()` 与 `dataStore.activateLibraryOnce()`。两个 Store 各自按 surface + owner generation 去重，Profile Hero 与成就概况都能在首次进入时获得快照和一次后台刷新。
- Tab 预创建、Tab 往返和二级设置页进入不触发额外请求；地区应用继续作为显式网络动作。
- 退出登录行点击后展示现有“退出 Xbox 账户？”确认对话框；确认后调用 `authStore.signOut()`。
- 登出完成后停留在“我的”Tab，并原位切换为登录态空壳与设置入口，避免 Tab 跳转。

### View and module structure

推荐将页面语义从 Profile 扩展为 My：

```text
Features/My/
  MyView.swift
  MySettingsRows.swift

Features/Settings/
  CloudGamingSettingsView.swift
  LoginPreferencesView.swift
  DiagnosticsSettingsView.swift
  AboutView.swift
  AppSettingsStore.swift
```

- `MyView` 拥有根 `NavigationStack`、账号内容、设置入口、下拉刷新与退出确认。
- `isActive` 从 `AppSection.my` 显式传入 `MyView`，远程数据激活继续受当前 Tab 门控。
- 四个二级页拥有各自局部 UI 状态。地区应用与 Trace 导出等副作用继续留在对应二级页。
- `MySettingsRows` 只负责显示摘要和发出导航，不直接执行设置副作用。
- `AppSettingsStore` 的持久化 key 与公开设置合同保持稳定。
- 可先保留 `ProfileView.swift` 中现有私有 Hero/成就组件，再在独立重构中按职责拆文件，控制本次变更范围。

### State matrix

| 状态 | 账号区 | 成就概况 | 设置入口 | 退出登录 | 下拉刷新 |
| --- | --- | --- | --- | --- | --- |
| 已登录且资料可用 | Hero | 展示 | 全部可用 | 展示 | 可用 |
| 已登录且资料加载中 | 加载状态 | 等待数据 | 全部可用 | 展示 | 可用 |
| 已登录且资料失败 | 错误与重新加载 | 可展示缓存时继续展示 | 全部可用 | 展示 | 可用 |
| 未登录 | 登录入口 | 隐藏 | 全部可用 | 隐藏 | 隐藏 |
| 地区应用中 | 保持当前内容 | 保持当前内容 | 云游戏摘要显示处理中 | 展示 | 暂停并发手动刷新 |

### Accessibility and interaction acceptance

- VoiceOver 按“标题、当前值、按钮”读取设置行；装饰图标隐藏语义。
- Dynamic Type 下标题和摘要使用系统字体；摘要空间不足时优先保留标题并截断摘要。
- 每行触控区域至少 44×44pt，整行可点击。
- Reduce Motion 继续关闭非必要转场动画；NavigationStack 使用系统转场。
- 退出登录与 Trace 清理维持 destructive role 和确认流程。
- 下拉刷新期间防止资料与地区切换形成重复 session refresh；可通过禁用地区应用按钮或共享 single-flight 状态完成。

### Validation gates

- 源码检查确认根 TabView 只保留“游戏库 / 主机 / 成就 / 我的”。
- 已登录、资料加载、资料失败、未登录四类页面状态均能访问预期设置入口。
- 首次进入“我的”时 Profile 与活动库各触发一次 `initialActivation`；四个 Tab 往返五轮和二级设置页往返保持零额外自动刷新。
- 下拉刷新恰好触发一次 Profile 刷新和一次活动数据刷新；右上角菜单与自定义刷新浮层已移除。
- 四个主页面条目均能进入正确二级页，返回后保持“我的”页面滚动和导航状态。
- 地区应用、临时登录偏好、Trace profile、导出、清理和版本展示保持现有行为。
- 退出登录位于已登录内容流最后一项，确认后回到未登录“我的”页面。
- 通过 Swift parse、PBX lint、arm64 Simulator `build-for-testing`、相关 XCTest 与 `git diff --check`。
- 使用 iPhone 紧凑宽度、最大 Dynamic Type、深浅色各完成一次页面截图与交互验收。

## Open Questions

- 成就概况首期继续作为只读汇总。后续可评估点击后切换到“成就”Tab或进入筛选后的成就列表。
- “关于”首期只承载版本信息。许可、隐私与反馈入口进入范围时可直接添加到该二级页。
- 页面和模块是否在本次实现中统一改名为 `MyView`，由执行 RFC 根据 Xcode 工程改动范围决定；产品文案与 Tab case 本次应统一为“我的”。

## Candidate Follow-On Tasks

1. **复杂任务：iOS“我的”Tab 合并 RFC 与实现**
   - 通过 `task-run` 固化路由、状态矩阵和兼容边界。
   - 合并根 Tab、重组账户主页、拆分四个设置二级页、迁移退出登录与刷新交互。
   - 补齐未登录可达性、设置副作用、导航和构建验证。
2. **简单任务：iOS“我的”Tab 真机视觉验收**
   - 覆盖深浅色、Dynamic Type、VoiceOver、下拉刷新、地区应用、Trace 分享和退出登录。
3. **候选简单任务：成就概况快捷导航**
   - 基于真实使用反馈决定跳转目标与跨 Tab 路由合同。
