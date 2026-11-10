# 轻流动正式前端验证

日期：2026-11-10。三个实施阶段已交付，用户实机验收保留在[实施 RFC](../../../docs/rfcs/2026-11-10-ui-motion-vitality.md)。

## 实际装配与检查

[验证入口](index.html)通过生产 `/src/main.ts` 加载完整 App、Vue Router、KeepAlive、FocusScope、navigationEngine、InputController、实际 HTMLVideoElement 和 WAAPI。Rust RPC 复用已有[离线数据](../2026-11-09-gamepad-focus-action-completion/project-fixture.js)。窗口背景事实由测试直接投影到 InputController，系统减少动态效果由入口模拟 MediaQueryList；原生窗口与真实系统偏好列入实机验收。原生输入外层使用短暂加入页面 Scope 的隔离 DOM 夹具。

可使用 `pnpm dev --port 1430 --host 127.0.0.1` 后访问入口，点击“运行动效回归”。测试修改仅在离线入口的内存中生效。结束恢复原生 animate 方法、前台状态及偏好，生产 main.ts 和业务配置保持生产装配。

| 尺寸 | 主题覆盖 | 浏览器检查 | 原始证据 |
| --- | --- | --- | --- |
| 1280×720 | 深色开始、浅色结束 | 35 项 | [宽屏](browser-wide.json) |
| 390×844 | 深色开始、浅色结束 | 35 项 | [窄屏](browser-narrow.json) |

每组覆盖：2px 单例外环与 4px 外扩、220ms 邻近飞行、不同尺寸同步、飞行中重新定向与 A 确认、停止后角度静止、48px / 280ms 一级等距交接与旧节点 inert（宽屏 Y 轴、窄屏 X 轴）、七次连续往返后的最新页面和 KeepAlive 身份、24px / 220ms 二级交接及对称返回（宽屏菜单竖排使用 Y 轴、窄屏横排使用 X 轴）、弹层面板延后 20ms / 160ms、跨层 100ms 原位淡入、120ms 退出与来源立即恢复、嵌套滚动吸附、原生输入安全外层、WAAPI 缺失 fallback、窗口背景 / 减少动态效果、详情水平前进与返回、串流准备静止、实际录像播放和暂停、浅色颜色 token 同步及零运行错误。

本次参数修订对应用户对切页直觉、动画可见度和飞行时长的反馈。页面与飞行共用 cubic-bezier(0.25, 0.6, 0.3, 1)；切页期间的方向 / 确认输入仍先结束视觉位移，焦点飞行支持途中重定向。

所有停稳采样的外环四边误差小于 0.6px；主要样本为 0px。停稳后 `ring.getAnimations().length` 为 0，角度再次采样保持相同值。每组浏览器运行错误为 0。页面切换期间新页立即激活，旧页 `inert`，KeepAlive 返回撤销临时定位与 inert。标题、底栏与导航保持既有布局。

![浅色宽屏](settings-light-wide.jpg)

![浅色窄屏](settings-light-narrow.jpg)

## 源码验证

- 13 个文件 / 119 项相关测试通过，包含真实 Vue 导航组件、来源恢复、busy / A 去重、录像操作、页面方向、输入先收稳与飞行层卸载释放。
- `pnpm test:navigation-coverage`：64 个 Vue 文件、174 处交互声明全部覆盖，violations 为空。
- `pnpm lint:fix`、`pnpm typecheck`、`pnpm build`、`git diff --check` 通过。
- [验证汇总与源码 SHA-256](validation-summary.json)用于对应本轮文件状态。

## 实机验收清单

1. 真实手柄连续方向输入、A / B 与 LB / RB / LT / RT；重点核对飞行途中确认、边缘目标、弹层开关、详情返回和分类滚动恢复。
2. macOS 原生窗口切出 / 切回、系统减少动态效果、串流准备 / 播放与录像播放 / 暂停；核对静止和恢复。窗口失焦按原始背景事实静止，输入 keep-active 偏好沿用原合同。
3. WKWebView 低功耗浏览与长列表：取样 220ms 外环 left / top / width / height 动画的帧率与合成成本，并核对加大页面位移后的观感。环境光仅在前台浏览运行，弹层、后台、串流和录像播放期间静止。
4. 同设备、同配置执行串流对照长测，结合 runtime trace 核对 media supply、steady supply、submit / present 长尾与帧供给 delta。记录网络与恢复健康度，作为动效静止策略的实机成本证据。

实机结果补齐后收口任务并形成最终 Report。
