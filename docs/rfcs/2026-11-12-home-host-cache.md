# 主机页缓存与更新 RFC

## Status

- Completion: 实施与自动验证完成；用户实机验收接续
- Current State: done
- Owner: 主窗口，单窗口执行
- Last Updated: 2026-11-12
- 执行授权：用户要求云游戏改造完成后处理主机页反复加载，落实缓存与更新；沿现有 data owner 和页面生命周期直接实施。

## Background

Home 每次激活调用 `getHosts`，离开页面使在途结果失效，返回再次请求。后台刷新还会让已有主机的主操作进入 busy。既有主机安装库需要刷新设备事实，串流发现与资格验证继续使用独立的 Rust 流程。

## Goal

已取得的主机列表立即呈现；60 秒内菜单热返回复用展示，过期在后台更新；离开期间允许当前账户请求完成缓存。手动刷新绕过时效并合并在途请求，失败保留最近成功内容，合法空列表作为成功缓存。

## Scope / Decisions

1. Rust `DataRuntimeState` 持有主机列表缓存，按 web 身份的账户 scope 隔离，采用单调时钟管理 60 秒时效与最多 8 个空闲账户槽位。保留正在刷新的槽位，取消领导请求时释放等待者。
2. 增加 `getHostsSnapshot` / `refreshHosts` typed RPC：快照可交付最近内容及明确时效；刷新查询 Smartglass 并在当前账户校验后提交。现有 `getHosts` 保持刷新设备事实的合同，并复用相同的在途请求。
3. Home 保留单实例展示状态与在途 Promise，激活复用有效缓存；活动期间按 Rust 时效安排更新，失败后自动重试至少间隔 15 秒，下次校验取 `max(freshUntil, retryAt)`；失活停止定时更新。认证事件清理展示和代次，隔离迟到结果。
4. 已有主机在后台更新时保持操作可用，选择与焦点锚点按主机身份保留；列表更新后按最新设备事实呈现。电源操作被接受、安装库取得新设备事实时，通过 scoped 变更事件触发 Home 校验。
5. 主机列表缓存只用于当前进程的展示与刷新，现有 xHome 跨区发现、串流准备、console identity 命令与 serverId 路由继续使用原流程。实际账户/主机及物理手柄由用户实机验收。

## Plan / Progress

- [x] M0：确认重复加载触发链、历史设备事实更新合同及现有 Rust owner。
- [x] M1：Rust 账户缓存、时效、在途合并、typed RPC 与变更通知。
- [x] M2：Home 缓存呈现、活动更新、手动刷新、认证/迟到结果隔离。
- [x] M3：回归、真实 Vue/浏览器菜单往返验证、报告与任务收口。

## Validation

- [x] 有效/过期/强刷/并发、合法空结果、失败保留、账户隔离、取消和更新中的失效回归。
- [x] 菜单往返 20 次复用列表，加载中离开/返回只产生一项在途请求，失活停止定时查询。
- [x] 更新期间主操作可用，主机选择保留，电源/设备事实通知按 scope 更新；认证切换隔离旧响应。
- [x] `cargo fmt` / data 63 项 / workspace check、`pnpm lint:fix` / typecheck / build、主机 Vue 13 项和导航 217 处通过。全量 Vue 985 通过、2 项既有失败，见 Report。
- [x] 深浅主题、桌面与窄窗口、空结果与失败状态的真实组件验证。
- [ ] 用户实机验收真实账户、主机电源状态收敛、安装库返回、实际串流和物理手柄。

## Results

20 次实际菜单往返新增 snapshot / refresh / HTTP 均为 0；加载中往返共用原请求，失活推进 61 秒期间新增请求为 0，返回追加一次后台更新。深浅主题 1280×720 与 540×850 布局通过，10 个核心源码与验证快照一致。详见 [Report](/Users/guo.xu/Documents/code/games/xbxrc/docs/reports/2026-11-12-home-host-cache.md) 与 [验证证据](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-12-home-host-cache/README.md)。

## Risks

- 电源状态可能在服务端稍后才收敛，接受命令后先失效缓存，再按正常更新读取真实状态。
- 时效内展示最近的设备事实，实际串流准备继续验证目标可达性与会话资格。
- 本任务的模拟 RPC 验证覆盖页面与数据合同；原生窗口和真实主机状态更新需要实机确认。
