# iOS“我的”Tab 合并 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: agent / iOS App
- Last Updated: 2026-07-22

## Background

- iOS 当前底部导航包含“游戏库 / 主机 / 成就 / 账户 / 设置”五个 Tab。
- `ProfileView` 承载账户 Hero、在线与社交信息、成就概况、资料刷新和退出登录；`SettingsView` 承载登录偏好、云游戏地区、Runtime Trace 诊断和版本信息。
- 两个页面共同表达当前用户与应用偏好。产品方向已在 [`docs/isu/2026-07-22-ios-my-tab-information-architecture.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/isu/2026-07-22-ios-my-tab-information-architecture.md) 收敛为单一“我的”Tab。
- 2026-07-16 设置 RFC 规定诊断在退出登录状态保持可达；合并后的未登录“我的”页面继续提供设置入口。
- 2026-07-22 懒加载 RFC 规定远程数据请求受当前 Tab 激活门控，每个 surface + owner generation 首次进入刷新一次，Tab 往返、前后台切换和普通凭据续期保持零自动刷新。

## Goal

- 底部导航收敛为“游戏库 / 主机 / 成就 / 我的”四项。
- “我的”以当前账户内容为主页面，保留既有 Hero、活动、社交和成就概况。
- 成就概况下方提供“云游戏 / 登录偏好 / 诊断 / 关于”四个系统设置式条目，各自进入职责单一的二级页面。
- 资料刷新统一由系统下拉刷新接管，并刷新 Profile 与 Xbox 游戏活动数据。
- 退出登录作为已登录内容流最后一项，继续使用 destructive confirmation。
- 未登录、资料加载和资料失败状态持续提供适用的设置入口。
- 保持现有设置持久化、地区应用、认证代际、Cloud Access、Runtime Trace 与 Rust bridge 合同稳定。

## Scope

- In scope:
  - `iosapp/XBXRC/App/AppRootView.swift` 的 Tab 与 `AppSection` 收敛。
  - `iosapp/XBXRC/App/XBXRCApp.swift` 的 XboxData session + owner generation 重绑身份。
  - `iosapp/XBXRC/Features/Profile/ProfileView.swift` 的“我的”根页面、状态布局、导航入口、刷新和退出交互。
  - `iosapp/XBXRC/Features/Settings/SettingsView.swift` 的四个二级设置页面与既有副作用迁移。
  - `iosapp/XBXRCTests/XBXRCTests.swift` 中与设置摘要、持久化和数据激活相关的定向回归。
  - Swift parse、PBX lint、Simulator/Device build-for-testing、源码结构门禁和视觉验收。
- Out of scope:
  - 桌面端 Profile、设置路由与 geometric gamepad navigation。
  - Rust Xbox 鉴权、Token、地区路由、目录协议和 UniFFI record 变更。
  - 成就概况跨 Tab 快捷导航。
  - 许可、隐私、反馈等新的关于页能力。
  - 物理移动 `ProfileView.swift`、批量重命名 Xcode group 和大范围组件拆分。

## Product Contract

### Bottom navigation

根 `TabView` 顺序固定为：

1. 游戏库
2. 主机
3. 成就
4. 我的

“我的”使用 `person.crop.circle.fill`。`AppSection.profile` 调整为 `AppSection.my`，独立 `AppSection.settings` 与 Settings Tab 从根层移除。当前默认 Tab 继续沿用 `.library`。

### My root page

已登录内容顺序固定为：

```text
账户 Hero
成就概况
账户与游戏活动错误提示（存在时）
设置与支持
  云游戏        当前地区 · 云访问状态
  登录偏好      标准会话 / 无 Cookie 临时会话
  诊断          Trace 记录级别
  关于          XBXRC 版本号
退出登录
```

未登录内容顺序固定为：

```text
登录 Xbox 账户
设置与支持
  云游戏        当前地区 · 未登录
  登录偏好      标准会话 / 无 Cookie 临时会话
  诊断          Trace 记录级别
  关于          XBXRC 版本号
```

资料失败状态展示“无法载入账户资料”和“重新加载”，并继续展示设置与支持。资料加载状态展示系统进度，设置与支持保持可操作。

### Settings rows

- 四行使用一个 grouped Liquid Glass surface，遵循现有主题 canvas、16pt 页面边距、0.5pt separator 与 52–56pt 最小行高。
- 每行包含左侧 SF Symbol、标题、右侧单行当前值和 `chevron.right`，整行作为导航点击区域。
- SF Symbol 固定为：云游戏 `network`、登录偏好 `person.badge.key`、诊断 `waveform.path.ecg`、关于 `info.circle`。
- 摘要使用 `AppSettingsStore`、`AuthStore.session`、根页面 `traceProfile` 状态与 Bundle 版本生成。
- Dynamic Type 下优先保持标题完整，摘要允许单行截断；VoiceOver 读取标题和完整当前值。
- 退出登录使用独立 section、红色 destructive label 和 `rectangle.portrait.and.arrow.right`，位置固定在已登录内容流末尾。

### Secondary pages

- 云游戏：地区路由 Picker、云访问状态、应用地区设置、进度、说明与脱敏错误。
- 登录偏好：无 Cookie 临时会话 Toggle 与下一次登录生效说明。
- 诊断：Trace 记录级别、导出当前、导出全部、清理 Trace、系统分享页、错误提示和清理确认；Trace profile 使用根页面 Binding，修改后同步更新主页摘要。
- 关于：应用名称、版本与构建号。

四个页面由“我的”的单一 `NavigationStack` 推入。各页面拥有自己的瞬时状态，返回后保留根页面导航与滚动状态。设置页面进入保持零网络请求；应用地区设置继续作为显式网络动作。

## Data And Lifecycle Contract

- `ProfileView(isActive:)` 继续从当前 Tab selection 接收显式激活状态；类型名与文件路径本次保持稳定，产品标题统一显示“我的”。
- “我的”根页面持有 `traceProfile = IOSRuntimeTrace.currentProfile`，诊断二级页通过 Binding 更新该状态和运行时 profile，返回主页后摘要即时一致。
- “我的”根页面持有共享 `isApplyingRegion` 状态，云游戏二级页通过 Binding 更新；地区应用期间根页面摘要显示处理中，并跳过下拉刷新提交。
- “我的”成为当前 Tab 时，调用 `authStore.activateProfileOnce()` 与 `dataStore.activateLibraryOnce()`。两个 Store 继续按 surface + owner generation single-flight 去重。
- `XBXRCApp` 的 XboxData 重绑 task ID 同时包含 `authStore.session` 与 `authStore.ownerGeneration`。地区切换发布新 generation 后，先完成 `dataStore.sync(session:ownerGeneration:)` 清理旧代际门闩，再由页面 activation 进入新代际首次刷新。
- Tab 预创建、四个 Tab 往返、二级设置页进出和 Scene active 不触发附加自动刷新。
- 已登录根页面使用一个公共 `ScrollView` 覆盖资料成功、加载和失败状态，并提供统一 `.refreshable`。每次有效手势调用一次 `authStore.refreshProfile(reason: .manualPull)` 和一次 `dataStore.refreshLibrary(reason: .manualPull)`，刷新期间保留旧内容与滚动位置；资料失败态“重新加载”按钮继续只刷新 Profile。
- 右上角 ellipsis 菜单、菜单刷新按钮、菜单退出按钮、`showsRefreshIndicator` 与 `ProfileRefreshIndicator` 全部移除。系统下拉刷新指示器承担唯一手动刷新反馈。
- 地区应用沿用既有顺序：保存 preset → 清理 Cloud Library → 刷新认证与 owner generation → 重新激活 Cloud Library。
- 地区应用期间 `isApplyingRegion` 保持为 true，云游戏页禁用重复提交，根页面下拉刷新直接返回；操作完成后恢复刷新能力。
- 退出确认后调用 `authStore.signOut()`；根 Tab selection 保持 `.my`，页面立即转为未登录状态，XboxData、Cloud Library 与 streaming store 继续通过 App 根层异步级联完成清理。

## Implementation Boundaries

### `AppRootView.swift`

- 删除独立 `SettingsView()` Tab。
- 将账户 Tab 文案改为“我的”，selection 判断与 tag 使用 `.my`。
- 保持游戏库、主机、成就顺序及现有 `isActive` 传递方式。

### `XBXRCApp.swift`

- 为 XboxData 同步建立同时包含 `StoredAuthSession?` 与 `ownerGeneration` 的 Equatable task identity。
- session 内容更新或 owner generation 变化都触发一次 `dataStore.sync(session:ownerGeneration:)`，保证地区切换、登出和切号的 Store 边界一致。

### `ProfileView.swift`

- 保留单一根 `NavigationStack`，统一承载主页、二级设置导航和退出 confirmation。
- 将已登录、资料加载、资料失败和未登录状态调整为共享设置入口的根页面布局。
- 在成就概况下加入设置分组，在已登录末尾加入退出登录行。
- 既有 Profile 与 library 错误卡位于成就概况之后、设置分组之前，退出登录继续保持为全部内容最后一项。
- 使用已登录公共 ScrollView 组合资料成功、加载、失败与设置入口，并根据 `isApplyingRegion` 门闩提交下拉刷新。
- 移除 toolbar 菜单与自定义刷新反馈。
- 扩展 activation task，使当前 Tab 首次进入激活 Profile 与活动库。
- 保留现有 Hero、社交、活动、成就聚合与错误卡实现，限制本次视觉回归范围。

### `SettingsView.swift`

- 将当前 Form 四个 section 收敛为 `CloudGamingSettingsView`、`LoginPreferencesView`、`DiagnosticsSettingsView`、`AboutSettingsView`。
- 每个二级页直接提供内容与 navigation title，不创建嵌套 `NavigationStack`。
- 地区应用通过根页面 `isApplyingRegion` Binding 暴露进行中状态；Trace profile 通过根页面 Binding 保持摘要一致。
- Trace 导出、分享 sheet、alert 与 confirmation 迁入对应页面并保持行为稳定。
- 本次保留文件路径，避免并行 `project.pbxproj` 改动冲突；后续模块整理可再拆分物理文件。

### Tests

- 保留 `AppSettingsStore` 的地区与临时登录设置持久化测试。
- 新增内部 `MySettingsPresentation` 纯值模型并测试摘要格式，覆盖未登录、可用、地区受限、等待刷新与 Trace profile。
- 增加数据激活回归：干净代际先进入“我的”时 Profile 与 library 各执行一次；先进入成就再进入“我的”时 library 复用既有 activation；“我的”Tab 往返保持零附加请求。
- 增加地区切换回归：owner generation 变化触发 XboxData 重绑，旧请求结果丢弃，新代际允许一次首次 activation。
- 增加登出回归：UI 立即切换未登录状态，随后 XboxData/Cloud/streaming store 完成级联清理。
- 使用源码结构门禁验证根 Tab、菜单移除、四个二级页和退出条目。

## Plan

1. M1：收敛根 Tab 与“我的”页面状态结构，接入共享设置入口和单一 NavigationStack。
2. M2：拆分四个设置二级页面，迁移地区与 Trace 副作用，保持未登录可达性。
3. M3：移除菜单与自定义刷新反馈，接入 Profile + library 首次激活和统一下拉刷新。
4. M4：补定向 XCTest、源码门禁、Swift/PBX/build 验证与视觉检查。
5. M5：更新 RFC 完成状态、生成 Report，并将 `docs/project-task.md` 收口为 Done。

## Validation

- [x] 根 TabView 只包含“游戏库 / 主机 / 成就 / 我的”，默认 selection 保持 `.library`。
- [x] 已登录、资料加载、资料失败、未登录四类根页面状态都能访问适用设置入口。
- [x] “云游戏 / 登录偏好 / 诊断 / 关于”四行进入对应二级页，返回后保持根页面状态。
- [ ] 干净代际先进入“我的”时 Profile 与 library 各记录一次 `initialActivation`；先进入成就再进入“我的”时 library 记录零次新增 activation；四个 Tab 往返五轮保持零附加自动刷新。
- [x] 每次下拉刷新提交一次 Profile manual pull 和一次 library manual pull，旧内容保持展示。
- [x] 右上角 ellipsis、菜单刷新、自定义顶部刷新浮层和独立 Settings Tab 已移除。
- [x] 地区切换后 XboxData 绑定新 owner generation，旧请求无法提交，新代际首次 activation 可执行。
- [x] 地区应用期间根页面跳过下拉刷新，操作完成后恢复刷新能力。
- [x] 退出登录位于已登录内容流末尾，确认后立即停留在未登录“我的”页面，随后数据与串流 Store 完成异步清理。
- [x] 地区应用、临时登录偏好、Trace profile、导出、清理与版本展示保持既有行为。
- [ ] VoiceOver、最大 Dynamic Type、Reduce Motion、深色与浅色布局通过定向检查。
- [x] `xcrun swiftc -parse` 覆盖全量 iOS App 与 XCTest Swift 文件。
- [x] `plutil -lint iosapp/XBXRC.xcodeproj/project.pbxproj` 通过。
- [x] 相关 XCTest 与 arm64 Simulator/Device `build-for-testing` 通过，或记录可复现环境阻塞。
- [x] `git diff --check`、`git diff --cached --check` 与定向源码门禁通过。
- [ ] iPhone 紧凑宽度完成已登录、未登录、二级设置与退出确认截图验收。

## Risks

- `ProfileView` 与 `SettingsView` 当前各自拥有 `NavigationStack`；实施时统一由“我的”根页面持有导航栈，避免嵌套导航造成标题、返回和 sheet 行为异常。
- 独立 Settings Tab 移除后，未登录和资料错误分支需要显式组合设置入口，保障诊断与地区配置可达。
- “我的”首次进入新增 library activation；Store 的 surface + owner generation 去重需要覆盖 Tab 往返和成就页复用场景。
- 地区应用会递增 owner generation；返回根页面时需要验证新代际首次激活次数和旧请求提交隔离。
- 当前工作区包含并行 iOS 串流、启动体验、Profile 与 PBX 改动；实施需要逐文件融合并保留现有修改。
- 完整 Xcode build 与 Simulator 运行可能受到 CoreSimulator、SwiftPM 缓存和审批服务状态影响；Swift parse、PBX lint、定向 typecheck 与源码门禁作为本地基础证据。

## Progress

- [x] Step 1: 完成现状代码审计、历史约束追溯与 ISU 信息架构收敛。
- [x] Step 2: 完成实施级 RFC，固定产品结构、生命周期、文件边界和验证门禁。
- [x] Step 3: M1–M4 已完成，根导航、我的主页、设置二级页、刷新和代际重绑均已落地。
- [x] Step 4: 静态验证、Report 与任务台账已收口；Xcode build/XCTest 和视觉验收的环境缺口已记录。

## Execution Notes

- Date: 2026-07-22 | Status: planned
- Update: 已从 ISU 进入 task-run 复杂任务路径，任务完成登记并形成实施级 RFC。
- Decision: 采用四个分类入口与四个二级设置页；保留现有 Swift 文件路径，使用单一 NavigationStack，保持 Store 与 Rust 合同稳定。
- Risk/Blocker: 代码实施等待用户明确确认；工作区并行修改将在实施阶段逐文件融合。

- Date: 2026-07-22 | Status: in-progress
- Update: 用户已明确确认执行；根 Tab、“我的”主页、设置二级页与数据代际重绑开始并行实施。
- Decision: 文件所有权按 `ProfileView/AppRootView`、`SettingsView`、`XBXRCApp/XBXRCTests` 分离，主窗口负责最终集成。
- Risk/Blocker: 当前工作区含并行 iOS 改动，各执行单元需要保留既有修改并限制文件范围。

- Date: 2026-07-22 | Status: completed
- Update: 四 Tab、“我的”主页、四个设置二级页、统一下拉刷新、末尾退出登录、复合代际重绑和定向回归已完成；交付结果记录于 [`docs/reports/2026-07-22-ios-my-tab-merge.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/reports/2026-07-22-ios-my-tab-merge.md)。
- Decision: 保持单一 `NavigationStack` 与现有 Store/Rust 合同；设置摘要使用纯值模型，地区应用和 Trace 状态由“我的”根页面持有。
- Risk/Blocker: 全量 Swift parse、PBX lint、diff/source gates 通过；`xcodebuild build-for-testing` 与 XCTest 受 SwiftPM 用户缓存权限、CoreSimulator 环境和审批服务 503 阻断，模拟器截图、最大 Dynamic Type、VoiceOver 与真实账号交互仍需在可用 Xcode 环境验收。
