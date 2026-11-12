# 云游戏菜单 P0–P2 性能优化 Report

## Summary

云目录与详情的生命周期、窗口渲染、索引、版本交付、合并持久化和图片尺寸优化已完成；详情截图采用单行横向滚动，保留大图预览和返回位置。实现保持现有 Rust 数据权威、Vue UI 与几何导航。

- [实施 RFC](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-11-12-cloud-menu-performance-implementation.md)
- [原方案](/Users/guo.xu/Documents/code/games/xbxrc/docs/isu/2026-11-12-cloud-menu-performance-plan.md)
- [验证说明](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/README.md) · [结果汇总](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-cloud-menu-performance-implementation/results.json)

## Delivered

1. P0：详情复用单实例，商品身份与活动路由、认证代次绑定；失活撤销页面等待与订阅，迟到响应按 scope 隔离。目录热返回复用已接受内容、搜索与浏览锚点，通过轻量状态校验时效。
2. P1：网格与横栏挂载视口和邻近窗口，跨窗口先挂载目标，再交由现有几何引擎选焦点。Map/Set、预归一化字段与分片搜索压低反复扫描；同版本目录和索引复用。
3. P2：Rust 提供内容 revision、轻量状态与单商品摘要，正常变更通过至多 75 项 patch 交付；版本缺口去重同步快照。可见窗口、最近和收藏资料优先补齐；base/overlay/status 合并保存，成功完整刷新显式保存目标最多两次。
4. P2：scope LRU 8、详情容量 30；官方 CDN 按 CSS 尺寸和 DPR 档位取图，DPR 上限 3，保留原图与备用来源回退。卡片图标采用半透明表面。详情截图左右滚动，支持上一张/下一张、大图与 Escape 返回。

## Changes

- [目录投影](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/xcloud/catalog-store.ts)、[索引](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/xcloud/catalog-index.ts)和 [窗口](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/xcloud/CloudCatalogWindow.vue)承接共享内容、增量派生和有界渲染。[XCloud](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/XCloud.vue)只在活动视图提交内容，并在离开前保存滚动。
- [详情](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/CloudGameDetail.vue)保留固定工具栏和独立滚动区，按商品保存截图横向锚点；[导航引擎](/Users/guo.xu/Documents/code/games/xbxrc/src/navigation/core/engine.ts)提供跨窗口准备与候选缓存。
- [Rust 缓存](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/cache_repository.rs)、[目录发布](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/catalog_projection.rs)和 [cloud service](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/services/xcloud_service.rs)保留账户/区域/语言/市场隔离、权益、合法空结果与局部失败降级。公开目录 fallback 的热读取继续交付同一内容版本及终态。
- [共享保存](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/settings_store.rs)通过串行阻塞任务完成持久化；取消异步等待后，已开始的磁盘写入继续持有串行锁。[图片候选](/Users/guo.xu/Documents/code/games/xbxrc/src/components/common/remote-image-source.ts)集中选择受支持的官方尺寸。

## Validation

真实 Vue hooks 的 20 次往返只读取一份全目录，详情实例保持 1 个，空商品查询为 0；设置阶段页面目录/详情请求与活动视图订阅为 0。同版本 titles/index 对象复用，进程共享 owner 保留一个目录订阅。

| 浏览器场景 | 卡片 / 焦点候选 | 观测 |
| --- | ---: | --- |
| 500 / 2,000 / 10,000 项首屏 | 24 / 87 | 1280×720，规模受视口约束 |
| 10,000 项第 2,000 项附近及尾部 | 35 / 120 | 深浏览保持有界 |
| 网格连续下移 40 次 | 30 / 105 | 意图到焦点更新 P95 1.3ms |
| 收藏横栏右移 30 次 | 34 / 118 | 跨窗口 P95 2.9ms |
| 540×850 / 恢复桌面 | 14 / 57；35 / 120 | 商品锚点保留 |

Node 的 10,000 项模型中，旧空查询 37–43ms，新空查询约 0.01ms；首次索引最长时间片 6.10ms；75 项增量归一化 75 项，耗时 5.67ms。全量事件 22,124,268 bytes，75 项 patch 165,642 bytes。官方 CDN 原图 1,118,479 bytes，384 档 51,475 bytes。

菜单纵向滚动精确恢复到 95047.5px；详情截图横向精确恢复到 614.5px，纵向受焦点安全区显露调整 44px。搜索、分类、排序、patch、公开浏览、空目录、刷新与详情失败、三个操作入口、LT/RT、深链、大图及深浅/窄窗口回放通过。

最终共用检查：Rust data 63 项、catalog flow 11 项、定向 Vue/导航 111 项、trace 生产预算 2 项通过；fmt、workspace check、lint:fix、typecheck、build 通过，导航 217 处、0 violations。Vue 全量 985 项通过、2 项失败，与改造前相同，分别涉及设置授权地区生效分类与串流诊断采样；[最终日志](https://github.com/xuasir/xbxrc/blob/b864eb1733751b9a57ffdad65ee4bb496175bb71/dev-docs/validation/2026-11-12-home-host-cache/checks/vue-all.txt)保留原始错误。8 个核心源码 SHA-256 与浏览器验证快照一致。

## Risks / Follow-up

- 浏览器部分长任务采样与编译、自动化并行；本报告的性能结论覆盖请求、DOM、局部导航、计算模型和事件字节。
- 用户在同一 production 构建中验收首次进入、过期刷新、详情回退和切菜单的输入到可操作 P95，以及 CPU/GPU、进程驻留和图片解码。
- 用户实机验收物理手柄、正常退出/断电后的缓存和真实网络状态；依据同设备对照结果调整窗口与批次预算。
