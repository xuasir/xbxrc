# 主机页缓存与更新验证证据

日期：2026-11-12。实施与本轮自动验证完成。汇总见 [results.json](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/results.json)，结论见 [Report](/Users/guo.xu/Documents/code/games/xbxrc/docs/reports/2026-11-12-home-host-cache.md)。

## 方法

production Vite 构建运行实际 App/router/KeepAlive/Home、几何导航与受控 Rust RPC fixture。Rust 主机缓存通过独立回归覆盖单调时钟时效、在途合并、取消、失败保留、账户隔离和 LRU。前端生命周期回归覆盖活动定时更新、失活暂停及认证代次。

[源码清单](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/preview-source.json)的 10 个文件在收口时与工作区、验证快照全部一致。fixture 中两台主机使用独立 console id、deviceId、serverId，账户 B 使用独立设备列表；全部操作由可见验证控件驱动。

## 行为证据

| 场景 | 结果 | 原始证据 |
| --- | --- | --- |
| 首次进入 | snapshot 1 次，HTTP 1 次 | [首屏](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/home-initial.json) |
| 主机↔设置 20 次 | snapshot / refresh / HTTP 增量全部 0 | [20 次往返](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/home-roundtrips-20.json) |
| 3 秒刷新期间切菜单再返回 | 共用原请求；书房选择保留，主操作可用，刷新按钮 busy | [慢刷新](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/home-slow-menu-return.json) |
| 首次加载期间往返 | snapshot / refresh / HTTP 增量全部 0 | [冷加载在途](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/home-cold-menu-return-pending.json) |
| 在设置页完成首次加载后返回 | 原内容直接呈现；此轮等待跨过 TTL，返回追加 1 次后台刷新 | [后台完成](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/home-cold-completed-return.json) |
| 刷新失败再往返 | 两台主机保留、主操作可用，新增请求 0 | [失败保留](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/home-refresh-failure.json) |
| 成功空列表再返回 | 正常 status 状态，新增请求 0 | [空列表](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/home-empty-cache.json) |
| scoped 通知与电源命令 | 旧账户通知新增请求 0；同账户 sessionReady 读取快照 1 次、HTTP 0 次；电源接受后快照、刷新、HTTP 各增加 1 次 | [事件](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/home-scoped-events-and-power.json) |
| 旧账户查询仍在途时切到 B | B 主机可见；A 迟到完成后继续显示 B，fixture 缓存 scope 仅 B | [切换中](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/home-account-switch-pending.json) · [迟到结果](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/home-account-switch-late-result.json) |
| 失活时推进 61 秒再返回 | 失活新增请求 0；返回 refresh / HTTP 各增加 1 次，保留内容与主操作 | [过期更新](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/home-expired-background-update.json) |
| 540×850 选择书房并切我的菜单再返回 | 书房选择保留，新增主机请求 0，横向溢出 0 | [窄窗返回](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/home-narrow-light.json) |

累计计数包含真实等待中的 60 秒更新和失败重试，各场景使用已保存的成对阶段增量。失败后的下次自动校验时间为 `max(freshUntil, retryAt)`，重试至少间隔 15 秒；有效缓存期间强刷失败可继续等待正常 TTL。

## 布局

1280×720 和 540×850 的深浅主题已观察实际画面，并核验文档宽度等于滚动宽度。验证控件作为 fixture 浮层出现在截图右下角。

- [浅色桌面](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/home-desktop-light.jpg)
- [深色桌面与账户隔离](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/home-desktop-dark.jpg)
- [浅色窄窗口](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/home-narrow-light.jpg)
- [深色窄窗口](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/home-narrow-dark.jpg)

## 检查与复现

Rust 主机缓存 8 项、data 63 项、workspace check、fmt、lint:fix、typecheck、build 通过。主机页新增 Vue 13 项通过；全量 114 文件、985 项通过、2 项既有失败。导航 217 处、0 violations。日志位于 [checks](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/checks)。两项全量失败分别为设置 `force_region_ip` 生效分类和串流 dev healthSamples 采样，详情见 [vue-all.txt](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/checks/vue-all.txt)。

从仓库执行 `create-fixture.mjs`、`prepare-preview.mjs`，在 `/private/tmp/xbxrc-home-host-cache-20261112` 使用 `node node_modules/vite/bin/vite.js build --config fixture.config.mjs` 构建。通过静态服务打开源码清单中的 browserUrl，使用主机验证操作控件回放请求、失败、账户切换与到期场景。本轮临时服务、标签页和 viewport override 已清理。

真实主机状态收敛、实际串流准备、原生窗口输入和物理手柄由用户实机验收；设备身份路由及安装库刷新合同沿用现有实现。
