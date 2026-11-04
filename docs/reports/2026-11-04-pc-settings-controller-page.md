# PC 设置层次、手柄页面与窗口布局修正 Report

## Summary

- Related RFC: [设置层次、独立手柄页与窗口布局修正](../rfcs/2026-11-04-pc-settings-controller-page.md)
- Completion: 已完成；2026-11-04，主窗口单窗口执行。
- 六项反馈已应用到生产 Vue 组件与 Tauri 窗口配置，延续方正小圆角、透明分类标题和哑光状态。

## Delivered

- 分段单选的未选状态提供独立底色，关闭态开关提供轨道底色；深浅色各自使用语义 token。
- 更新版本与通道同排，通道选项、说明、检查结果与动作聚合为独立轻底卡片，卡片内边距与主间距为 16px。
- 标准密度的分组标题、选项名称、说明、生效提示分别为 18 / 16 / 13 / 12px；紧凑密度为 16 / 14 / 12 / 11px。标题字重和说明颜色形成层次，生效提示使用轻底标签。
- 手柄成为 `/controller` 一级页面，设备选择直接内联；后台输入策略、游戏键位、摇杆、键盘模拟、震动、输入测试和高级功能全部集中于手柄页。设置保留通用、串流体验、连接与主机、高级与诊断四类。
- 主机刷新入口为 36×36px 按钮、20px SVG 图标，保留可访问名称、提示和加载禁用状态。
- Tauri 主窗口使用 `titleBarStyle: Visible` 与 `hiddenTitle: true`，系统窗口按钮占用独立标题栏；前端移除 Overlay 专用左侧避让。

## Changes

- 顶栏选中状态、上下文标题、返回判断、LB/RB 和触摸切换共用主机 → 云游戏 → 设置 → 手柄的页面顺序。
- 手柄页复用现有 InputController、配置保存和 FocusScope 编辑会话；页面离开时清理选项弹层、录入捕获、输入测试与草稿会话。
- 预览发现键盘录入筛选后的数组包含嵌套响应式代理，保存前逐项构造绑定 DTO，修复 DataCloneError。保存仍使用现有 Rust 配置合同。
- 修复主按钮悬停背景覆盖导致浅色主题文字对比不足，主要操作悬停保留品牌底色，几何聚焦沿用哑光状态。
- Vue 3 + TypeScript、Tauri + Rust、认证、媒体、串流和几何导航继续沿用当前主线。

## Validation

- `pnpm lint:fix`、`pnpm typecheck`、`pnpm build`、`git diff --check` 通过。
- `pnpm test`：56 个测试文件、578 项测试通过，覆盖现有更新、输入控制、编辑会话和导航回归。
- 已安装 Tauri CLI schema 明确支持 `Visible`；配置默认装饰与隐藏标题保持兼容。
- 真实 Vue 隔离预览导入生产 App、router、i18n 与组件；`/private/tmp/xbxrc-pc-preview` 的 Tauri RPC 替身仅回写预览内存。
- 设置：核对未选 / 选中分段、关闭轨道、四类分类、透明固定标题、独立更新卡片；检查更新显示最新版本，切换稳定版的确认 / 取消与来源焦点恢复通过。
- 手柄：空设备提示、全部配置归属、页面选中状态通过；左摇杆上从 W 改为 Q，应用后重新打开仍为 Q，再恢复 W；应用后返回来源行。录入期间通过浏览器返回离开页面，再次用页面快捷键进入手柄和高级页面，捕获会话与弹层按生命周期释放。
- 输入路径：键盘模拟的 1 / 2 触发相同 LB/RB 页面路径，3 / 4 触发相同 LT/RT 分类路径；K 关闭编辑后恢复原入口。
- 1280×720 深色设置 / 手柄、960×540 深浅色设置 / 手柄 / 主机布局通过；960px 视口的文档宽度为 960px，底栏独立可见，手柄长内容可滚动。刷新按钮实测 36×36px，点击后完成刷新并保持可用。
- 临时浏览器视口已恢复，本轮验证标签已关闭；已有隔离预览服务继续保留。

## Screenshots

![设置层次与独立选项底色](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-settings-feedback-general-dark.png)

![紧凑更新卡片](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-settings-feedback-update-dark.png)

![独立手柄页](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-controller-feedback-dark.png)

![浅色矮窗口更新卡片](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-settings-feedback-update-960-light.png)

![主机刷新图标](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-home-feedback-refresh-960-light.png)

## Risks

- 标题栏配置需要重新启动 Tauri 才生效；系统窗口按钮、窗口拖动、全屏和原生 WebView 的最终布局由用户实机核验。
- 本轮自动验证覆盖隔离浏览器与单元测试；物理手柄设备切换、震动及真实串流继续按仓库约定由用户手动验收。

## Follow-up

重新启动 Tauri，在常用窗口尺寸核验独立系统标题栏，并使用物理手柄确认设备选择、肩键导航、键位 / 摇杆和震动设置。
