# 应用轻流动动效实施

## Status

- Completion: 三阶段代码完成，切页方向与可见幅度调优完成；实机验收待补齐
- Current State: validation-pending
- Owner: 主窗口直接执行，单窗口
- Last Updated: 2026-11-10
- Authorization: 用户要求继续已确认的[方案](../isu/2026-11-09-ui-motion-vitality-plan.md)，本轮承接实施。

## Goal / Scope

在现有 Vue 3 / TypeScript UI、KeepAlive、FocusScope、navigationEngine 和 InputController 上落实轻流动。共享主题 token 控制页面 280ms / 48px、子页 220ms / 24px、弹层进入 180ms / 退出 120ms、焦点邻近飞行 220ms / 跨区域淡入 100ms；背景 36 秒往返。一级页面按导航顺序前进 / 返回，一二级同层切换均跟随可见菜单排列：桌面竖排上下交接、窄屏横排左右交接；详情进入 / 返回使用水平层级过渡。确认 / 返回输入立即执行。

业务状态、几何导航、输入归属、媒体运行时沿用当前合同。页面与弹层通过共享 Vue Transition + WAAPI 呈现，旧画面离开前释放交互并移出布局；新画面立即挂载。输入事件先结束页面视觉位移，导航与命中在稳定位置执行。FocusFlowLayer 只消费引擎事件并绘制单一 2px 外环。

## Plan

1. 接续现有 token 与弹层编辑，统一主 / 子页共享过渡，补齐方向、快切、旧页面 inert 与 KeepAlive 恢复。
2. 全局 FocusFlowLayer：邻近移动尺寸 / 半径同步飞行；跨页 / Scope / 长距离淡入；滚动与布局变化即时吸附；连按从当前视觉位置重定向；静止及能力缺失沿用 CSS fallback。
3. 壳层静态环境光上增加低对比慢漂移。消费 InputController 窗口背景事实、document visibility、串流路由及录像播放事实；静止状态停止装饰动画，弹层打开暂停背景周期。

## Validation

- [x] 定向与详情返回、连续快切、真实 Vue KeepAlive、旧节点 inert、唯一有效页面。
- [x] 焦点单例、2px 边框、尺寸和圆角、跨页与弹层、快速重定向、滚动吸附与卸载释放。
- [x] 深浅色、系统偏好与后台事实的离线模拟、串流准备静止和实际录像播放 / 暂停恢复；窄屏结果记录于[验证目录](../../dev-docs/validation/2026-11-10-ui-motion-vitality/)。
- [x] 导航与播放器相关 119 项回归、174 个交互声明覆盖、pnpm lint:fix / typecheck / build。
- [ ] 用户实机：WKWebView 低功耗合成与供帧成本、物理手柄、实际串流。

## Risks / Decisions

- 并行过渡保持新页面立即可用，旧页面只作视觉残影。离开前 inert，完成后撤销临时定位和 inert，供 KeepAlive 再激活。
- 页面移动会改变 DOM 测量；所有方向 / 确认输入先结束移动动画。焦点层仅在过渡、滚动与布局变化时短暂追踪，停稳后释放 RAF。
- 根据用户对方向直觉和动画可见度的反馈，一级进出使用对称 48px 位移，二级使用对称 24px 位移；页面与焦点飞行采用 cubic-bezier(0.25, 0.6, 0.3, 1)，弹层保持原有收稳曲线。
- 焦点外环尺寸使用 left / top / width / height，保持均匀描边。全局角度累计，飞行期间连续流动，停稳后静止。
- 环境光采用静态渐变 + transform / opacity。实机供帧收益与成本记录为独立验收项。

## Progress

- [x] 页面与弹层：共享 UiMotionTransition 采用并行呈现，进入即时挂载；旧节点 inert / 清理焦点 / 移出布局，收尾恢复原样式。输入在已有调度链前结束位移，保持几何导航测量稳定。页面 280ms / 48px、子页 220ms / 24px，进出等距相反，同层切页方向随响应式菜单排列、详情进退保持横向；遮罩进入 180ms、面板延后 20ms / 160ms、退出 120ms。
- [x] 焦点飞行：单例 FocusFlowLayer 消费引擎 Focus / Scope / Targets 事件，邻近 220ms，同步宽高与圆角；跨区域 100ms 淡入，页面移动时持续定位并保持淡入；滚动吸附，连按从当前视觉位置重定向。渐变角度在短动画内按 8 秒速率累计、停稳静止，主题颜色直接继承 token，增强柔光默认关闭。
- [x] 背景与生命周期：36 秒往返、总位移 24×12px、透明度 .95–1。窗口背景消费 InputController 原始事实，页面隐藏消费 visibility，串流路由从准备阶段静止，录像消费 playing / playBusy。静止同步释放飞行，后台暂停保留环境光周期。
- [x] 生产前端与回归验证：[可重跑入口](../../dev-docs/validation/2026-11-10-ui-motion-vitality/index.html)，生产 main.ts / Router / KeepAlive / Scope / 输入 / 视频 / WAAPI 均实际装配；RPC 使用既有离线数据。
- [ ] 用户实机验收：物理手柄、原生窗口、WKWebView 低功耗浏览与串流供帧成本。验收补齐后生成最终 Report。
