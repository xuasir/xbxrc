# 云游戏刷新与标题分类栏验证

> 2026-11-12 整理：本页保留验证摘要；带 Git 链接的截图、夹具和中间输出按采样时提交读取。

日期：2026-11-12。主窗口直接完成本轮修复。汇总见 [results.json](results.json)，源码指纹见 [source.json](source.json)。

## 实现

- Rust 刷新阶段沿同一缓存范围保留最近游玩、新入库、热门、即将离库四种排名；当前商品名单、串流 ID 和权益继续使用本轮接口事实，栏目查询完成后按既有成功或降级规则更新。
- Vue 首屏等待索引和查询结果就绪后展示，刷新继续呈现已提交内容。分类、排序按钮位于搜索框前，搜索与刷新保持同组。
- 账户目录与更新状态统一位于标题描述行；游戏数量沿内容区域展示。局部来源状态同样进入标题描述行。
- 分类导航采用单行横向滚动、焦点安全留白及现有几何导航的滚动容器合同。

## 自动检查

| 检查 | 结果 | 证据 |
| --- | --- | --- |
| Rust 数据模块 | 65 项通过，覆盖栏目保持、商品移除、当前权益与合法空目录 | [日志](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-cloud-refresh-and-filters/checks/rust-data-unrestricted.txt) |
| 云目录模型与状态 | 7 文件、27 项通过 | [日志](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-cloud-refresh-and-filters/checks/vue-cloud.txt) |
| 实际 Vue setup / KeepAlive | 首次完整查询一起提交；20 次往返目录请求 1 次，数组与索引复用 | [回放](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-cloud-refresh-and-filters/route-lifecycle-result.json) |
| fmt / lint / typecheck / build | 通过 | [lint](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-cloud-refresh-and-filters/checks/lint.txt) · [typecheck](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-cloud-refresh-and-filters/checks/typecheck.txt) · [build](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-cloud-refresh-and-filters/checks/build.txt) |
| 导航覆盖 | 217 处交互、违规 0 | [日志](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-cloud-refresh-and-filters/checks/navigation.txt) |

初次全量前端回归为 1062 项通过、1 项既有设置断言失败：`force_region_ip` 预期 `nextConnection`，实际 `nextAuthorization`。原始输出见 [全量日志](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-cloud-refresh-and-filters/checks/vue-all.txt)。Rust 沙箱回放包含四项媒体下载测试的本地端口权限错误；允许本地端口绑定后，完整数据模块 65 项通过。

## 页面回放

使用当前 App、路由、实际 Vue 页面和几何导航，RPC 由已有 [离线 fixture](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/browser-fixture.js) 提供。[本轮测量入口](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-cloud-refresh-and-filters/preview-fixture.js)记录布局与栏目状态。

- 2000 款目录的首屏与刷新期间保留全部六个栏目；[刷新记录](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-cloud-refresh-and-filters/catalog-refresh-final.json)包含开始、进行中与终态。
- 中文分类、最近排序和 Halo 搜索返回 1 款；1280×720、960×720、390×844、320×720 场景完成布局核验。
- 320px 英文分类栏保持一排，可见宽 296px、内容宽 697px；连续五次现有导航右移后，横向位置到 401px，末尾分类完整显露。[手柄意图记录](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-cloud-refresh-and-filters/catalog-horizontal-focus.json)。
- 390px 中文场景的原生焦点定位会显露末尾分类。[焦点记录](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-cloud-refresh-and-filters/catalog-horizontal-keyboard.json)。

页面预览：[桌面](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-cloud-refresh-and-filters/catalog-desktop-dark.png) · [中文窄屏](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-cloud-refresh-and-filters/catalog-narrow-dark.png) · [窄屏横向末尾](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-cloud-refresh-and-filters/catalog-narrow-dark-scrolled.png) · [英文 320px](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-cloud-refresh-and-filters/catalog-small-light-en.png)。

旧离线 fixture 的主机快照 RPC 产生一条 mock 缺项错误；本轮云目录 RPC 与交互完成回放。真实 Tauri 目录数据、网络和物理手柄由用户在重启开发应用后验收。
