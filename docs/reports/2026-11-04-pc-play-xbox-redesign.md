# PC Play Xbox 首版 UI 改造 Report

## Summary

- Related RFC: [PC Play Xbox 风格改造](../rfcs/2026-11-04-pc-play-xbox-redesign.md)
- Completion: 已完成；2026-11-04，主窗口单窗口执行。
- 已将确认的方案应用于真实 Vue 客户端：通顶环境光、透明分组导航与常驻底部提示、主机重点区、方形游戏卡片、头像账户弹窗和完整设置页。

## Delivered

- foundation → semantic → alias 统一字体、深浅色、品牌色、渐变玻璃、边框、圆角、阴影、间距及焦点效果。表单控件继承页面字体；导航和底栏默认透明，选中 / 悬停 / 聚焦使用局部材质。
- 左侧主机 / 云游戏与设置 / 手柄两组导航，右上角头像。底栏占据独立布局行，随页面、弹层、保存 / 错误状态与输入归属显示有效操作。
- 主机透明示意图、重点区与多设备选择。开机、待机、关闭和未知状态沿用既有串流 / 远程管理能力及刷新入口。
- 云游戏保留最近游玩 / 新入库 / 全部 / 搜索与现有目录结构。封面方形、contain、透明承载，保留 tile 优先与 poster 降级；轨道加入浏览按钮，滚动标题局部玻璃化。
- 账户使用头像下方约 320px 弹层，退出有确认；设置保持完整页面和五类真实配置，编辑 / 输入 / 控制器复用当前 FocusScope 与弹层。
- Tauri 主窗口 macOS titleBarStyle 设置 Overlay、hiddenTitle；原生 macOS 头部为系统窗口按钮留出空间。

## Changes

- 壳层局部 Vue 上下文汇总提示，按来源身份处理 KeepAlive 页面和嵌套弹层的退出与恢复；业务数据与认证继续由原有服务拥有。
- LB/RB 页面切换和 LT/RT 分类 / 区域切换检查最上层 scope。账户 / 编辑弹层限定 Tab，B/Escape 关闭后恢复来源；底栏在弹层期间保持可见。
- 几何导航继续使用现有引擎；可见 sticky 搜索 / 跳转控件保持目录位置，内容焦点避让标题。轨道只负责横向显露，页面纵向滚动保持原路径；触摸轨道与表单交互使用原生滚动。
- 搜索开始同步保存位置，清空后在列表完成更新与绘制时恢复；目录加载失败仍保留已获得的账户权限和现有数据。封面加载失败保留标题与透明底色。
- Vue 3 + TypeScript、Tauri + Rust、RPC、配置 schema、认证和串流领域合同继续沿用。HUD 几何布局与媒体 / 输入装配保持当前主线。

## Validation

- `pnpm lint:fix`、`pnpm typecheck`、`pnpm build`、`git diff --check` 通过。
- `pnpm test`：56 个测试文件、577 项测试通过。新增提示来源 / 嵌套弹层恢复与 sticky 搜索焦点回归；相关输入、编辑和焦点回归共 72 项通过。
- Tauri 已安装 CLI schema 核对 Overlay / hiddenTitle 配置合法。
- 真实 Vue 隔离数据预览：导入生产 App、router、i18n 与组件；临时 Tauri RPC / 事件替身位于 `/private/tmp/xbxrc-pc-preview`，配置保存只作用于预览内存。游戏封面为带 PREVIEW 标识的验证素材。
- 主机：1280×720、1366×768、1920×1080、1280×672、960×540 与 600×720，主动作和底部提示可见，横向溢出为 0；单 / 多设备、开机 / 待机 / 关闭 / 未知、空 / 加载失败已核对。
- 云游戏：上述宽度档位的封面保持方形、contain 与透明承载，底栏可见且横向溢出为 0；1366 宽度档预览另有 769px 高度读数。横向浏览产生有效滚动，渐进加载从 20 张增加到 35 张；缓存、搜索 / 空结果、清空恢复、poster 与图片失败已核对。
- 设置：960×540 编辑弹层、1280×720 完整页面与五类配置；保存结果和生效时机显示于底栏，关闭后恢复原设置项。
- 交互：LB/RB 页面切换、LT/RT 设置分类、账户 Tab 循环、退出确认取消 / 来源恢复、B/Escape、弹层肩键保护及控制器关闭恢复通过。预览使用现有键盘回退触发相同输入路径。
- 主机路由核对：待机设备使用 `stream-s` 作为 targetId、`console-s` 作为 consoleId，并传递 ConnectedStandby / remoteManagementEnabled=1；预览串流由隔离替身结束。
- 截图从完整应用画布导出，裁去预览容器多出的外侧留白；预览服务已停止，临时视口已恢复。

## Screenshots

实际 Vue 页面，使用隔离数据：

![主机首页](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a444-8ccf-7823-9983-35c93a252064/pc-redesign-home.jpg)

![账户弹层](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a444-8ccf-7823-9983-35c93a252064/pc-redesign-account.jpg)

![云游戏](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a444-8ccf-7823-9983-35c93a252064/pc-redesign-cloud.jpg)

![完整设置页面](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a444-8ccf-7823-9983-35c93a252064/pc-redesign-settings.jpg)

![浅色适配](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a444-8ccf-7823-9983-35c93a252064/pc-redesign-light.jpg)

## Risks

- 按仓库约定，真实 Tauri 窗口按钮 / 拖动与标题栏、物理手柄、认证退出、真实目录与串流由用户实机验收。
- 玻璃合成在原生 WebView 的成本、串流 submit / present 长尾及帧供给 delta 需要实机运行证据；本轮浏览器和单元测试验证布局与交互。

## Follow-up

运行 `pnpm tauri dev`，使用常用 16:9 窗口检查系统窗口按钮、物理肩键 / 扳机、实际主机与云游戏启动，以及串流运行条和诊断面板。当前工作区保留可审阅改动。

## Keyboard Hint Correction · 2026-11-04

- 键盘模拟保存生效后，底栏确认 / 返回 / 页面 / 分类提示使用实际 J / K / 1、2 / 3、4；关闭后恢复 A / B / LB、RB / LT、RT。提示读取原有输入控制器的配置快照，设置草稿应用后更新。
- 账户与设置屏幕键盘同步显示 J / K，表单输入提示标注 J。菜单提示复用固定键盘映射；游戏自定义绑定继续由原有游戏分支消费。输入测试独占期间标注实际 Enter 快捷键，录入期间保留有效 Esc 提示。
- `pnpm lint:fix`、`pnpm typecheck`、`pnpm build`、`git diff --check` 与 28 项相关回归通过；新增回归核对游戏映射调整后的菜单键位、输入来源切换及订阅清理。
- 真实 Vue 隔离预览核对源配置保存前后提示切换、账户 J / K、表单 J 打开屏幕键盘、K 取消与焦点恢复。预览修改只作用于内存，服务及临时标签页已关闭。

![键盘模拟录入提示](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a444-8ccf-7823-9983-35c93a252064/pc-keyboard-hints.png)

## 设置材质与列表续修 · 2026-11-04

- 承接用户最新反馈，对照 `https://play.xbox.com/` 的指南 → 设置 → 外观。参考页选项使用 8px 圆角、48px 行高、独立轻底块和约 16px 行间留白；导航选中为低透明度中性色，单选圆点表达当前配置。
- 设置项统一为独立轻底块和 16px 行间留白，分组标题与组间留白表达内容层次；清理分组外框、内部横线及单选弹窗横线。行内单选的标签、说明和选项共用一个轻底块。
- 共享选中 / 悬停材质改为中性哑光底色；选中控件的描边为透明，焦点阴影为 none。单选弹窗以右侧圆点表示已保存配置，以哑光底色表示导航焦点，两种状态独立显示。
- 设置页面继续承接通顶环境光；滚动标题采用稳定底色与统一 16px 圆角。弹层面板使用共享轻玻璃材质，最上层统一提供遮罩，下层保留草稿与焦点节点；面板阴影与页尾留白收紧，默认面板内边距为 24px、按钮间距为 8px。
- `pnpm lint:fix`、`pnpm typecheck`、`pnpm build`、`git diff --check` 通过；设置几何导航、编辑会话、键盘回退、壳层提示共 4 个测试文件、28 项回归通过。
- 真实 Vue 隔离预览核对 1280×720 深浅色列表、单选圆点 / 焦点独立状态、K / Escape 关闭及来源恢复、设置保存反馈、确认弹窗页尾。960×540 核对自定义中继编辑与嵌套屏幕键盘，底部操作可见、横向溢出为 0；取消录入后返回原输入框，确认放弃草稿后返回原设置项。
- 滚动至 `scrollTop=921.5` 后，标题保持 16px 圆角、backdrop-filter 为 none，底部提示可见。预览配置保存作用于隔离内存；复用原任务预览服务，本轮临时标签页已关闭、视口覆盖已恢复。真实 Tauri 窗口、物理手柄与串流按仓库约定由用户实机验收。

![独立选项与哑光状态](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-settings-spaced-dark.png)

![浅色设置层次](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-settings-spaced-light.png)

![紧凑确认页尾](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-settings-spaced-confirm.png)

## 方正轮廓与透明标题 · 2026-11-04

- 按用户最新反馈，将基础圆角收紧为 2 / 4 / 6 / 8px；控件和游戏封面为 4px，主机设备卡片为 6px，大面板和弹层为 8px。密度档位复用同一圆角真源，导航、搜索框、账户、手柄配置和串流操作 / 文本 / 音量弹层同步消费。
- 设置分类标题置于独立的透明标题区，正文在下方滚动；背景为 transparent、backdrop-filter 为 none。标题直接承接通顶环境光，滚动内容的边界自然形成焦点安全区，分类位置记录继续绑定实际滚动容器。
- 1280×720 深色预览核对主机重点区 8px、设备卡片 6px、游戏封面和搜索框 4px；设置单选弹层为 8px，选项及关闭按钮为 4px，K 关闭恢复原设置项。
- 设置内容滚动至 921.5px 时标题仍位于 80–147px，切换分类后恢复 921.5px；方向键可进入当前可见设置项。960×540 浅色预览的标题保持透明，横向溢出为 0；连续方向导航后的焦点位于 375.78–422.78px，完整处于 139–452px 正文安全区内，Escape 关闭单选弹层恢复来源。
- `pnpm lint:fix`、`pnpm typecheck`、`pnpm build`、`git diff --check` 与 28 项相关回归通过。预览使用原任务的隔离数据与生产 Vue 组件；临时标签页已关闭，视口覆盖已恢复。真实 Tauri 与物理手柄按仓库约定由用户验收。

![方正控件与透明分类标题](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-square-settings-dark.png)

![浅色矮窗口](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-square-settings-light-small.png)

![方正单选弹层](/Users/guo.xu/.codex/visualizations/2026/11/04/01a1a51d-0c9d-7680-9879-e9a5370ce923/pc-square-settings-select.png)
