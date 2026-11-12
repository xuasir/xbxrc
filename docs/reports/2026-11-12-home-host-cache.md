# 主机页缓存与更新 Report

## Summary

主机页已采用缓存即显与后台更新：60 秒内菜单热返回复用已取得的列表；过期更新保留内容和操作，手动刷新合并在途请求，失败保留最近成功结果。加载期间离开页面后，当前账户请求继续填充缓存，返回使用原结果。

- [RFC](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-11-12-home-host-cache.md)
- [验证说明](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/README.md) · [结果汇总](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/results.json)

## Delivered

1. [Rust 主机缓存](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/mods/data/host_cache.rs)归属现有 `DataRuntimeState`，按账户 scope 隔离，以单调时钟管理 60 秒 TTL，LRU 回收空闲账户，保留在途槽位。取消领导请求释放等待者，成功空列表形成有效缓存，失败保留旧快照。
2. `getHostsSnapshot` 即时读取缓存，`refreshHosts` 强制查新并合并在途请求。既有 `getHosts` 继续查询设备事实，供主机安装库刷新使用；查询提交前核验当前账户，电源失效代次隔离旧响应。
3. [页面投影](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/home/hosts-store.ts)和 [生命周期](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/home/use-hosts.ts)保留展示与在途 Promise，活动时按时效校验，失活停止定时更新，认证切换及卸载清理代次。自动失败重试至少间隔 15 秒，下一次校验取 `max(freshUntil, retryAt)`；手动刷新可立即重试。
4. [Home](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/Home.vue)后台更新期间保持主操作可用，刷新按钮展示 busy，主机身份与选择保持。电源命令被接受及安装库取得新设备事实时，scoped `data.hostsChanged` 通知页面校验。

## Changes

缓存仅保存当前进程的主机列表事实；更新与时效由 Rust owner 管理，Vue 消费 typed [HostsSnapshot](/Users/guo.xu/Documents/code/games/xbxrc/src/shared/rpc/hosts.ts)。Home 返回沿用实例内容，认证就绪时可直接读取 Rust 有效缓存。

主机串流继续使用 `serverId` 身份，安装库和电源命令继续使用 console id / deviceId。xHome 跨区发现、串流准备、可达性与资格核验保持现有 Rust 流程。设备变更事件仅交付 scopeKey。

## Validation

| 场景 | 结果 |
| --- | --- |
| 实际 App/router/KeepAlive 主机↔设置 20 次 | snapshot / refresh / HTTP 增量全部 0 |
| 3 秒刷新期间离开/返回 | 共用原请求；书房选择、两台主机与可用主操作保留 |
| 首次请求期间离开/返回 | 新增主机请求 0；离开期间可完成缓存 |
| 刷新失败、成功空列表再返回 | 各自状态保留，新增请求 0 |
| 失活时推进 61 秒，随后返回 | 失活新增请求 0，返回增加 1 次后台刷新 |
| 旧账户通知 / 同账户 sessionReady / 电源接受 | HTTP 增量 0 / 0 / 1；匹配 scope 更新 |
| A 查询在途时切到 B，随后 A 完成 | 两阶段均显示 B，旧响应隔离 |
| 540×850 选择书房并切我的菜单再返回 | 选择保留，新增主机请求 0 |

在设置页完成首次加载后的独立回放跨过了 TTL，返回直接呈现缓存并追加一次后台刷新；该结果按实际等待记录。累计请求数包含每 60 秒活动更新和失败重试，表中结果使用成对阶段增量。

1280×720、540×850 深浅主题的实际页面已检查，横向溢出为 0。[浅色桌面截图](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/home-desktop-light.jpg)与 [窄窗口截图](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/home-narrow-dark.jpg)保留画面。

Rust 主机缓存 8 项、data 63 项、workspace check、fmt、lint:fix、typecheck、build 通过。主机页 Vue 新增 13 项通过；全量 985 项通过、2 项既有失败，分别涉及设置生效分类和串流 dev healthSamples；[checks](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/checks)保存日志。导航 217 处、0 violations。10 个核心源码 SHA-256 与验证快照一致。

## Risks / Follow-up

- 服务端电源状态需要时间收敛，命令接受后先失效列表，再通过后续查询更新实际状态。
- 列表在有效期内展示最近设备事实，实际串流准备继续验证目标可达性和会话资格。
- 用户实机验收真实账户的菜单往返、手动刷新、主机开关机/待机、安装库返回、实际串流和物理手柄。
