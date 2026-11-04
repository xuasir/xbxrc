# PC 手柄连接子页、焦点和动效完善报告

- 日期：2026-11-04
- 状态：Completed
- 执行：主窗口，单窗口
- [RFC](../rfcs/2026-11-04-pc-controller-status-focus-motion.md)

## 交付

- 手柄一级页包含连接状态 / 手柄设置两个子页。连接页复用 InputController，恢复透明设备图、连接数量、当前设备、输入来源 / 用途、采样频率、标准映射、震动能力、设备列表及按钮 / 摇杆 / 扳机实时展示；配置全部保留在手柄设置子页。
- 云游戏标题、搜索和分类融合背景，透明头部与内容滚动区域分开；搜索改为线框放大镜，区域与轨道焦点显露使用平滑滚动。
- 配置选项以底色区分：深色已选白色 24% / 常态约 4.3%，浅色已选蓝灰 18% / 常态 5%。移除新增勾选标记，聚焦 / 悬停继续保留选中底色；设备、主机与二级分类沿用共享选中色阶。
- 焦点使用主题绿到青蓝的淡雅 2px 渐变细环与低强度光晕，深浅色分别控制色阶。共享 CSS mask 轮廓覆盖导航、设置选项、游戏和设备卡片；原生输入框保留同色阴影。游戏卡片外层允许轮廓显示，封面圆角裁剪集中到图片容器。
- 一级导航取消左侧绿线，选中以背景表达；二级分类保留绿线。Logo 常态、悬停、聚焦均为透明底。
- 主机页标题 24px，重点设备名称标准 26px / 紧凑 24px。重点卡片采用弱渐变背景、淡色边框、16px 毛玻璃和轻阴影，刷新保持 36px 图标按钮。
- 页面、子页、主机切换、设置分组、焦点进入、悬停 / 按下和滚动增加短动效，并响应减少动态效果偏好。手柄子页单根容器修复设置编辑器多根节点导致的返回空白；主机重点区保留最小高度。

## 验证

- `pnpm lint:fix`、`pnpm typecheck`、`pnpm build`、`pnpm test`、`git diff --check` 通过；56 个测试文件、580 项测试全部通过。增加减少动态效果与快速导航滚动回归。
- 真实 Vue 隔离预览核对空设备与双设备：第一设备左摇杆 X=0.40，第二设备震动不支持 / RT=70%，选择设备后身份与能力同步更新。
- 手柄设置更改右摇杆反转、应用、关闭后重开读取保存值，再恢复原值；返回来源行焦点正确。连接 / 设置连续往返显示完整，独立滚动恢复实测 481px。
- 已选“生产”和聚焦“开发”同时展示，底色与外环分别表达状态，DOM 无新增勾选图标。深浅色均核对渐变轮廓，Logo computed background 为透明。
- 云游戏透明头部滚动后 background 保持透明、blur 为 none；DOOM 搜索返回 12 款，无结果清空后恢复原位置；键盘模拟经几何导航连续六次 Right 到 GROUNDED 7，轨道 scrollLeft=235.5，焦点完整显露。
- 主机切换 Series X / Series S 后名称、图片和主按钮同步，重点区高度稳定。1280×720 标准、960×540 / 800×600 紧凑布局核对内容独立滚动、无水平溢出及底部操作提示。
- 隔离双设备夹具仅用于预览内存。完成后预览入口恢复生产 main.ts，临时 viewport 已清除，本轮浏览器标签已关闭，沿用的预览服务保持运行。

## 截图

- [手柄连接与设备信息（深色）](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-controller-connection-devices-dark.png)
- [手柄小窗口（深色）](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-controller-connection-small-dark.png)
- [已选底色与聚焦渐变（深色）](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-settings-selection-focus-dark.png)
- [已选底色与聚焦渐变（浅色）](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-settings-selection-focus-light.png)
- [云游戏轨道焦点（浅色）](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-cloud-navigation-focus-light.png)
- [主机材质（深色）](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-home-material-dark.png)
- [主机材质（浅色）](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-home-material-light.png)

## 实机边界

- 现有 InputDevice 合同提供 id / name / standard / rumble；电量与连接方式按未知展示。
- 保持 Vue 3 + TypeScript、Tauri + Rust、InputController 与现有几何导航主线。
- 物理手柄、原生系统标题栏与真实串流由用户实机验收。

## 圆角与云游戏刷新续修

- 一级导航复用共享 4px 控件圆角，移除局部重复声明；焦点轮廓按外扩 4px 增加半径并补足元素边框宽度，使内外弧线平行。主机 / 设备卡片、游戏卡片、圆形头像与小圆角控件分别提供自身半径，2px 边框的串流操作同步补偿。
- 云游戏刷新迁到搜索框右侧，使用与主机页一致的 36px 按钮 / 20px 图标；保留 aria-label、悬停文案、禁用状态与刷新旋转动效。搜索模式下刷新入口保持可用。
- lint / typecheck / build / diff check 通过。真实 Vue 预览核对深浅色、800×600 / 960×540，导航 4px / 外轮廓 8px，搜索框与刷新按钮间距 12px，刷新后保留 DOOM 搜索条件，布局无水平溢出。临时 viewport 已清除，本轮标签已关闭。
- [深色圆角与刷新布局](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-cloud-refresh-radius-dark.png) · [浅色矮窗口](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-cloud-refresh-radius-light.png)

## 焦点裁切、流动轮廓与诊断续修

- 应用主内容左右保留 8px；云游戏页头也保留对称 8px，修复嵌套页面容器对分类首项的裁切。串流菜单和诊断列表增加 12px padding / scroll-padding，音量与文本弹层允许滚动并预留 24px，聚焦显露继续复用几何导航。
- 串流菜单、音量与文本按钮取消聚焦放大，轮廓沿用控件圆角与边框补偿。统一主题绿 / 青蓝 conic-gradient，注册角度属性并以 8 秒匀速流动，减少动态效果偏好关闭动画。
- 设置输入、音量输入与串流文本输入复用外层 `.ui-focus-outline`。音量 / 文本原生 focus 同步现有 navigationEngine，输入内部取消重复 outline。实测设置输入与外层均高 48px、文本输入与外层均高 55px；音量输入获得原生焦点后仅保留一处高亮。
- 诊断背景从 55% 加深至 96%，宽度使用 340–480px 限制并受窗口剩余宽度约束。Tab 选中使用独立底色，聚焦使用渐变外环；按钮实体边框保持透明。指标行可获得键盘焦点，Tab / Shift+Tab 在最上层诊断 scope 内循环，Escape 遵循当前 scope。
- 真实组件隔离预览使用静态测试指标和本地封面。菜单最后一项自动滚动至 scrollTop=40.5px，外扩 4px 后完整位于滚动容器内；同一焦点角度连续采样 296.126deg / 168.016deg，确认动画持续运行。
- 800×600 诊断面板首尾指标外框完整，最后一行 scrollTop=192px。深浅色核对“显示”保持已选底色、“恢复”独立获得外环。960×540 主机卡片与云游戏轨道焦点完整；800×600 云游戏分类首项外环距页内容器仍有 4px 余量，页面无水平溢出。
- 最终 `pnpm lint:fix`、`pnpm typecheck`、`pnpm build`、`pnpm test`、`git diff --check` 通过，56 个测试文件 / 580 项测试通过。临时 viewport 已清除，本轮预览标签关闭，生产预览入口仍导入 main.ts，沿用的本地服务保留。
- 物理手柄、原生系统窗口与真实串流仍由用户实机验收。

- [菜单底部焦点完整](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-stream-action-focus-last-dark.png)
- [诊断已选底色与聚焦外环](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-stream-diagnostics-focus-dark.png)
- [诊断小窗口最后一行](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-stream-diagnostics-focus-small-dark.png) · [浅色小窗口](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-stream-diagnostics-focus-small-light.png)
- [音量单一焦点](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-stream-audio-focus-small-dark.png) · [文本输入外框](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-stream-text-focus-small-dark.png)
- [云游戏分类小窗口完整外框](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-cloud-category-focus-safe-800-dark.png) · [主机卡片焦点](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-home-focus-safe-small-dark.png)
