# iOS 地下城官方布局保存与触屏能力 Report

## Summary

已完成[本轮 RFC](../rfcs/2026-11-25-ios-dungeons-touch-engine.md)中的手柄触控兼容与保存修复。用户截图的保存阻断集中于官方 `v` 前缀 schema 识别及背景图片绘制位置；同类图片字段在各控件/样式中重复产生诊断。

## Delivered

- 官方已知 schema 支持 `v` 前缀；控件背景、摇杆帽与拉动提示前景直接复用已解码 CGImage，按默认、移动、按下、切换、拉动和禁用状态绘制；三/四/六/八位 hex 色与组件/整体透明度同步处理。
- touchpad 按采样时间输出摇杆速度，20ms 静止超时与释放归零；arcadeButtons 支持六/八键、多指、相邻组合和整排组合；补齐单轴摇杆、方向键 deadzone 与两类 responseCurve。
- 卡片和预览共享去重后的能力原因；资源缺失、非法字段、未知动作/样式/图片位置的保存门禁保留。AxisCap 和 PullIndicatorBackground 按微软官方颜色合同绘制。

## Validation

最终 iPhone 18 Pro / iOS 27.0 / Xcode 27 定向验收 **9 项通过，0 项失败**：8 项单元、1 条 UI。UI 直接点击第二官方布局的卡片保存，检查预览拖动、保存退出与本机记录。单元验收覆盖图片身份复用、状态切换、拉动分段与释放、颜色/透明度、未知图片位置门禁，以及静止后再次拖动。

12 个 Swift 文件的任务修改范围格式化与差异检查通过。前轮已完成 `pnpm lint:fix`、串流边界检查及17项定向验收，本轮仅补验新增图片层和受影响路径。

测试摘要、四张最终截图、源码哈希、任务差异、共享文件快照、能力范围与清理记录见[证据目录](../../dev-docs/validation/2026-11-25-ios-dungeons-touch-engine/README.md)。构建、测试及导出进程结束后清理本轮临时 DerivedData、SourcePackages 副本、xcresult、导出图片和工具缓存，磁盘可用空间增加约 **615 MiB**，清理后剩余 **70.86 GiB**。仓库已有共享缓存继续保留。

## Compatibility Boundaries

当前引擎是官方 TAK 的部分兼容实现。[能力范围](../../dev-docs/validation/2026-11-25-ios-dungeons-touch-engine/support-matrix.md)列明动态 Context、传感器宿主、鼠标/键盘输出、命名颜色、渐变及完整内置图标库等后续对齐项。

最终 UI 使用 Halo Infinite 入口下的官方格式夹具，其第二布局同时声明 `v3.1`、touchpad 和背景图片。真实 Minecraft Dungeons II 游戏包与实机串流操作需要在更新后的 iOS 客户端验收；建议优先重新保存用户截图中的官方布局，再检验游戏内多指操作与布局切换。
