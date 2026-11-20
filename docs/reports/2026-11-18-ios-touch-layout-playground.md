# iOS 数据驱动触控布局与本地输入测试 Report

## Summary

完成离线触控渲染与输入采集闭环，入口为“控制器 → 触屏手柄 → 布局与输入测试”。[RFC](../rfcs/2026-11-18-ios-touch-layout-playground.md)与[证据](../../dev-docs/validation/2026-11-18-ios-touch-layout-playground/README.md)已同步。

## Delivered

- 社区包装与直接 TAK Layout 经统一模型解释；通用 Xbox、Forza 按键、Forza 踏板及官方格式最小示例可切换。Halo 原件进入能力扫描。
- button、joystick、directionalPad、throttle 共用输入引擎，覆盖 cluster/null/blank、layer、组合动作、pullAction、relative/absolute、toggle、sticky 与滑动转交。
- 单个 UIKit 多指面维护稳定指针 ID。按键按来源 OR，轴相加后限幅，短按保存双边沿；切布局、重排、后台与关闭统一取消。
- CALayer 原生绘制透明表面、细描边和低饱和 ABXY；语义图标沿 SF Symbols/原生路径绘制，默认 65%、可调 20–100%。绘制按显示帧合并，输入随触摸批次记录。
- 实时按键/摇杆/扳机/触点、事件浮层、清空、能力报告和本地 JSON 导出完成。记录上限 2048 帧，导出保留丢弃计数、布局身份和状态边沿。

## Validation

18 项定向单元用例在手机/iPad 均通过；覆盖原始 JSON、跨尺寸命中与边界、多个来源释放、短按、同时摇杆/按键/扳机、层引用与替换、拖拉、保持油门、toggle、独占指针、取消、有限坐标和透明通道。

手机/iPad 的 XCUITest 从控制器入口打开页面、切换三套布局、横竖屏按压/拖动，验证 sticky 的 RT 保持和清空归零。最终手机用例还验证展开事件浮层后 RT 保持。必要原生截图与测试摘要已归档。

iOS 设备目标关闭签名编译通过；Swift format、工程 plist、会话边界及 diff 检查通过。Forza/Halo 与冻结原件逐字节相同。

## Boundaries

- 本轮输出进入本地 recorder，真实串流接线沿后续任务推进。
- 完整 TAKX、动态 Context/引用、图片与语言/DPI 资源、触摸板速度映射由能力报告识别后继续扩展。默认轮盘使用应用几何，完整玩家自定义和非圆碰撞区进入后续兼容范围。
- 多指并发由纯 Swift 回放验证；模拟器 UI 验证单指真实事件。实机多指手感、触控尺寸与遮挡接续用户手动验收。

## Cleanup

本轮临时根目录为 `/private/tmp/xbxrc-ios-touch-layout-20261118/r1`。测试进程结束、必要摘要/截图归档后清理本轮 xcresult、重复导出、DerivedData、日志、格式配置与参考图片；释放量及路径核验记录见证据目录的 `cleanup.json`。复用的 WebRTC 与 Rust 缓存保留。

推荐接续实机多指与低遮挡验收，再沿同一引擎扩展 Halo 触摸板能力。

2026-11-18 补充：按压高亮、轻缩放、短按视觉保留 140ms 与应用触觉/声音开关接入完成；最终 22 项输入/绘制回归及入口 UI 测试通过。[反馈修正证据](../../dev-docs/validation/2026-11-18-ios-touch-feedback/README.md)。
