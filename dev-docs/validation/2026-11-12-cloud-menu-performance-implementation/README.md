# 云游戏 P0–P2 验证证据

日期：2026-11-12。实施与本轮自动验证完成。汇总见 [results.json](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/results.json)，结论见 [Report](/Users/guo.xu/Documents/code/games/xbxrc/docs/reports/2026-11-12-cloud-menu-performance-implementation.md)。

## 方法与范围

- 生命周期回放执行实际 `XCloud` / `CloudGameDetail` setup、Vue watcher、KeepAlive hooks 和 App 路由 key，使用受控 RPC 与空渲染宿主统计查询和实例。目录缓存策略由 Rust 回归验证。
- 浏览器回放使用 production Vite 构建、实际 App/router/页面/几何导航和模拟 Rust RPC。固定目录包含最近、官方栏目、大量收藏及完整目录，覆盖 500、2,000、10,000 项。
- Node 模型实验在 Apple M2 Pro / Darwin arm64 / Node v24.21.0 执行，比较已保存旧模型和生产索引、查询、窗口模型，测量计算时间与 JSON 字节。
- [源码清单](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/preview-source.json)记录 8 个核心文件 SHA-256；收口时工作区、验证快照及记录全部一致。

## 核心结果

| 场景 | 结果 | 原始证据 |
| --- | --- | --- |
| 改造前四步详情回放 | 全目录 6 次、详情 6 次、空商品 3 次 | [基线](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/route-lifecycle-before.json) |
| 改造后 20 次目录/详情/设置往返 | 全目录 1 次、详情实例 1 个、空商品 0 次，同版本数组与索引复用 | [生命周期](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/route-lifecycle-result.json) |
| 500 / 2,000 / 10,000 项首屏，1280×720 | 每组 24 张卡片、87 个焦点候选 | [500](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/catalog-500-start.json) · [2,000](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/catalog-2000-start.json) · [10,000](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/catalog-10000-start.json) |
| 10,000 项目录第 2,000 项附近及尾部 | 每组 35 张卡片、120 个候选 | [深处](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/catalog-10000-index2000.json) · [尾部](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/catalog-10000-tail.json) |
| 网格连续下移 40 次 / 横栏右移 30 次 | 意图到焦点更新 P95 1.3ms / 2.9ms；30 / 34 张卡片 | [网格](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/catalog-10000-down40.json) · [横栏](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/catalog-rail-cross-window.json) |
| 540×850 缩放及回桌面 | 14 / 35 张卡片，57 / 120 个候选，商品锚点保留 | [窄窗](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/catalog-grid-narrow.json) · [恢复](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/catalog-grid-resized.json) |
| 菜单往返滚动恢复 | 95047.5px → 95047.5px | [菜单恢复](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/menu-scroll-restoration.json) |
| 详情截图与纵向滚动恢复 | 截图 614.5px → 614.5px；纵向 358.5px → 314.5px，焦点安全区显露调整 44px | [详情恢复](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/detail-scroll-restoration.json) |

设置页活动目录视图订阅为 0；进程共享目录 owner 保留一个订阅。生命周期 fixture 的单商品请求使用受控响应，详情缓存命中和过期行为由 Rust 回归覆盖。

10,000 项 Node 模型的旧空查询为 37–43ms，新空查询约 0.01ms；首次索引总计 61.75ms，最长时间片 6.10ms；75 项更新只归一化 75 项，耗时 5.67ms。全量事件 22,124,268 bytes，75 项 patch 165,642 bytes。见 [规模实验](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/scale-results.json)。

官方 CDN 响应核验：原图 2160×2160 / 1,118,479 bytes；128×128 / 7,623 bytes；384×384 / 51,475 bytes。见 [图片尺寸](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/image-sizing.json)。

## 场景与画面

浏览器已覆盖搜索和清空、分类、排序、刷新 patch、公开浏览、合法空目录、目录与详情失败、卡片主体/播放/收藏入口、LT/RT 跳栏、详情深链、截图横向滚动及大图预览。状态文件与 [browser-observations.json](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/browser-observations.json)保存各阶段观测。

- [浅色详情截图横栏](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/detail-screenshots-light.jpg)
- [深色详情截图横栏](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/detail-screenshots-dark.jpg)
- [窄窗口截图横栏](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/detail-screenshots-narrow-dark.jpg)
- [大图预览](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/detail-screenshot-preview-dark.jpg)

## 自动检查

本轮追加主机页改造后的最终共用检查：Rust data 63 项、workspace check、fmt、lint:fix、typecheck、build 通过；导航 217 处、0 violations。日志位于 [主机页 checks](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/checks)。本目录保留云改造阶段的 111 项 Vue/导航定向回归、11 项 catalog flow、2 项 trace 生产预算及 Rust 缓存交付回归日志。

最终 Vue 全量 114 文件、985 项通过、2 项失败。失败与 [改造前记录](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-11-cloud-catalog-ux-implementation/checks/vue-all-final.txt)一致：`force_region_ip` 生效分类预期 nextConnection、实际 nextAuthorization；串流 dev healthSamples 预期 0、实际 27。见 [最终日志](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/checks/vue-all.txt)。

## 验收边界

浏览器部分长任务观测与构建及自动化并行，结论限定于已观测的请求、DOM、局部导航、计算模型和事件字节。实机输入到可操作 P95、CPU/GPU、进程驻留、真实网络图片解码、磁盘保存耗时和物理手柄由用户在 Tauri production 构建中验收。
