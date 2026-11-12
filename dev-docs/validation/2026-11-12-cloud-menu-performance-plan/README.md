# 云游戏菜单性能方案验证证据

日期：2026-11-12。对应[优化方案](../../../docs/isu/2026-11-12-cloud-menu-performance-plan.md)。

## 真实详情 setup 生命周期回放

执行方式：

```sh
node dev-docs/validation/2026-11-12-cloud-menu-performance-plan/route-lifecycle-probe.mjs
```

[探针源码](route-lifecycle-probe.mjs)直接读取 `src/pages/CloudGameDetail.vue`，使用项目 Vue compiler-sfc 编译 setup，然后在真实 Vue renderer + KeepAlive 中执行。KeepAlive key 按商品路径设置，与 `src/App.vue` 的 `resolveRouteViewKey` 及 cloud-game 的 navPage 路由合同对应。

RPC、路由、个人游戏记录和焦点域使用受控输入，宿主渲染为空；详情的路由 watcher、事件订阅、偏好与等待组合函数沿用实际源码。组件 SHA-256、Vue 与 Node 版本记录于[结果 JSON](route-lifecycle-result.json)。

四步为打开详情 A → 回退目录 → 打开详情 B → 切换设置。结果确认：

- 各步全目录查询次数为 1、1、2、2，总计 6。
- 各步详情查询商品为 A、空字符串、B/B、空字符串/空字符串，总计 6，其中 3 次空 ID。
- 回退时活动详情为 0，目录订阅为 1；打开 B 后订阅为 2；切到设置时活动详情为 0，订阅仍为 2。
- 应用最终卸载后目录订阅为 0，说明释放点处在卸载生命周期。

探针断言对应本次方案形成时的基线行为。后续生命周期修复使用真实 hooks 回归目标：失活查询 0、空商品查询 0、活动详情单路查询和有界订阅。

## 证据边界

本回放测得详情实例带来的请求放大。XCloud 页面自身的激活查询、Rust 接口处理、IPC 字节、DOM/layout/paint 与原生窗口响应耗时，由方案阶段 0 单独测量。

## 源码核对

- 目录已有 Rust 内存/磁盘缓存和每商品时效，overlay 新鲜期 10 分钟。
- 商品资料已有每批 75 项、最多 4 路的预算，首批和余批分阶段取得。
- RemoteImage 已配置浏览器 lazy loading、async decoding 和候选回退。
- XCloud 横栏完整挂载，网格持续追加前缀；状态更新和页面重新激活会接受整份目录。
- 目录阶段 save 分别保存 overlay/source 状态，首批和余批商品各保存一次，共享 settings store 承接这些写入。
- 2026-11-11 的 2,000 条功能 fixture 每条官方横栏为 4 项，图片采用本地 data SVG，其数据分布适合交互验证；性能基线增加真实栏目规模、长描述、不同封面尺寸和长列表往返。
