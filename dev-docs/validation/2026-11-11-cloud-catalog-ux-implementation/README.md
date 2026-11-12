# 云目录与交互增强验证

验证日期：2026-11-11。使用真实 Vue 页面、统一导航引擎和模拟 Tauri RPC；目录含 2,000 个项目，图片为本地验证素材。

## 复现入口

仓库 Vite 服务下访问本目录 `index.html`，路由为 `#/xcloud` 或 `#/xcloud/games/P0000`。验证期间使用固定源码快照 `/tmp/xbxrc-cloud-ux-preview` 的 1434 端口，减少其他工作区编辑触发的页面重载。

URL 参数支持 `scenario=catalog/public/failure/empty/detail-failure`、`theme=light/dark`、`language=zh/en`。`#project-validation-results` 保存模拟 RPC 计数、时间线和当前几何导航可达图。固定快照只用于验证，正式代码位于仓库 `src`。

## 页面结果

| 场景 | 已验证结果 | 证据 |
| --- | --- | --- |
| 卡片操作 | 收藏图标保持目录路由且选中；主体进入详情；DOM 嵌套按钮为 0 | [目录](catalog-desktop.jpg)、[时间线](detail-browser-state.json) |
| 搜索与返回 | 全角 `ＨＡＬＯ infinite` 返回 2 项，直接名称优先；详情返回恢复搜索词和 P0000 焦点 | [时间线](detail-browser-state.json) |
| 详情 | Hero、工具栏、简介、语言与个人信息；截图下一张显示 2/2，Escape 关闭并恢复 | [桌面](detail-desktop.jpg)、[480px](detail-narrow.jpg) |
| 图片回退 | P0000 Tile 故障后 Poster 加载成功 | [详情时间线](detail-browser-state.json) |
| 搜索双框修复 | 输入内层普通 field-shell、无输入阴影；外层搜索栏统一拥有飞行焦点；浅色 480px 只显示整框轮廓 | [截图](search-single-focus-light.jpg)、[样式事实](search-focus-browser-state.json) |
| 公开播放 | 可浏览，点击播放进入 `#/login` | [登录引导](public-play-login.jpg) |
| 刷新失败 | 2,000 项目录保持可见，错误与局部来源降级可见，刷新终态释放 | [截图](catalog-failure-light.jpg)、[状态](failure-browser-state.json) |
| 空目录 | 合法 0 项目录保留暂缺收藏；刷新时保留搜索栏，终态焦点仍为 `xcloud.refresh`，显示目录更新时间 | [截图](catalog-empty-light.jpg)、[状态](empty-browser-state.json) |
| 详情失败 | 基础资料与个人记录保持可见，重试后详情请求数为 2，导航图无不可达节点，无横向溢出 | [截图](detail-failure-dark.jpg)、[状态](detail-failure-browser-state.json) |

## 自动检查

检查输出保存在 [checks](checks)。最终 lint、类型、构建、209 处导航覆盖及 diff check 通过；云目录 Vue 12 项、WebAPI 70 项、Cloud flow 10 项、Tauri data 43 项通过。最终全量 Vue 为 941 通过、2 失败，断言详情见 [全量日志](checks/vue-all-final.txt) 和 [Report](../../../docs/reports/2026-11-11-cloud-catalog-ux-implementation.md)。

此次浏览器验证消费模拟 RPC；真实 Xbox 账户、物理手柄、Tauri 原生窗口与实际云串流由用户实机验收。临时 viewport 已恢复。
