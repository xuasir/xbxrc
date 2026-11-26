# iOS TAK 布局保存与手柄触屏引擎兼容

## Status

- Owner：主窗口，单窗口直接执行。
- Current State：completed。
- Last Updated：2026-11-25。
- 用户明确要求实现地下城第二个布局及相关触屏引擎能力。本轮按该执行授权推进。

## 背景与证据

用户反馈《我的世界：地下城 2》第二个布局保存显示“布局等待适配”。保存链先经过 Rust 资源准备，再由 Swift `PreparedTouchLayout` 执行能力检查；`GameTouchLayoutCard` 遇到待支持项后保存布尔状态并禁用保存，具体原因只在预览页出现。

微软 commit `99d5449bb054908876ceb867f948652f3b9cb41b` 的六份官方样例均声明 `/layout/v3.1/layout.json`，现有 `supportsSchema` 只接受无 `v` 的版本字段。该问题可直接复现纯手柄布局被误拒。样例原件已保存在前轮[证据](../../dev-docs/validation/2026-11-25-ios-touch-layout-download-followup/fixtures/official-sample-layouts.takx)。

现有引擎支持 button、joystick、directionalPad、throttle；touchpad 在归一化、几何与输入三层存在显式缺口；arcadeButtons、方向键 deadzone、responseCurve 仍被拒绝。官方 TAK v4.1 schema 与仓库冻结 TAK 输入实现作为字段与行为依据。

当前模拟器使用验收夹具，其布局资源库尚无用户游戏包。用户提供的两张实际截图确认 `Minecraft Dungeons II` 的 `gameplay_default` / `dialogues`、版本 `1.0.0.0` 均包含“此布局版本等待转换器支持”和重复的“此图片位置等待绘制支持”。实际保存阻断由 schema 识别及素材绘制位置两项门禁构成；本轮补齐官方 Background / Knob / PullIndicator 前景等素材位置，保留资源原件的必需校验。

## 范围与合同

1. 官方 schema 同时识别既有无前缀路径及微软 `v` 前缀路径，按已知版本执行兼容规则。
2. touchpad 输出沿现有 `IOSStreamGamepadState`：按位移/采样时间计算摇杆速度，支持 axis 对象/数组、action、renderAsButton；20ms 静止超时、释放、取消与切换统一归零。
3. arcadeButtons 支持六/八个子按钮、多指、按钮间组合与整排组合；绘制和输入共用子按钮几何，沿现有贡献合并保留独立释放。
4. joystick 按 axis 区分横向、纵向与双轴；directionalPad 使用声明的 deadzone 与 activationType。
5. 两类 responseCurve 在既有 deadzone → curve → sensitivity 管线中提供有限、奇对称和端点稳定的结果。冻结发布脚本中分母为零的异常按数学极限修正并记录边界。
6. 保存卡片显示实际能力诊断；能力门禁继续拒绝资源缺失、未知动作、未知字段与未实现的输出合同。
7. 官方背景、摇杆 knob 与 pullIndicator 前景图片进入同一资源绘制层；节流杆 cap 与 pullIndicator 背景按官方颜色合同绘制，按默认/idle/moving/activated/toggled/pulled/disabled 样式切换，颜色与 opacity 同步处理。绘制层复用已解码 CGImage，图片资源继续由既有 Rust/Swift 有界加载。

输入与绘制继续使用既有 UIKit/Core Animation、`TouchInputAccumulator`、`StreamInputSession`。公共 Rust 下载、存储、认证与串流协议保持各自既有职责。本轮只增加现有手柄输出合同内的触控能力。

用户追问完整性后，补充[官方能力范围](../../dev-docs/validation/2026-11-25-ios-dungeons-touch-engine/support-matrix.md)。当前完成范围为官方 TAK 的部分兼容；动态 Context、传感器、鼠标/键盘输出及完整官方样式属于后续合同与绘制能力扩展。

## 影响文件

- `TouchLayout.swift`：控件及轴/曲线模型。
- `TouchLayoutNormalizer.swift`：schema、控件字段与能力报告。
- `TouchLayoutGeometry.swift`：触摸板及格斗子按钮的共用命中几何。
- `TouchControlRuntime.swift`：触摸板速度、格斗组合、方向键与轴管线。
- `TouchInputAccumulator.swift` / `TouchLayoutSurface.swift`：源时间戳与20ms速度衰减、真实绘制和释放。
- `OfficialTouchLayoutsView.swift`：保存能力原因。
- 定向 XCTest 与一个真实入口 XCUITest。

## 最小验收

- 微软原件的 v3.1 常用手柄布局完成读取与能力检查；受支持布局可准备和保存。
- touchpad、曲线、方向键与 arcadeButtons 回放覆盖有效输出、静止超时、组合与独立释放。
- 一条卡片 → 第二个布局预览 → 保存 → 本机库的 UI 冒烟；失败提示保留具体原因。
- Simulator 编译、Swift 格式与必要边界检查完成；测试通过后直接收口。
- 最终截图、摘要、字段证据与失败诊断归档，确认进程结束后清理本轮临时根。

## 执行进度

- 首轮 Simulator 编译与17项定向验收通过：16项单元、1条第二布局触摸板预览/拖动/保存/本机列表 UI。Swift 本轮修改范围格式化、`pnpm lint:fix` 与串流边界检查通过。
- 用户截图补齐实际阻断证据后，完成背景/knob/PullIndicator 前景素材、颜色/透明度、样式切换及重复诊断整理；微软官方 AxisCap 和 PullIndicatorBackground 定义仅含颜色，继续对这些位置的资产图片及未知样式保持门禁。补验覆盖图片层复用与释放、静止后重新拖动、点击卡片保存及预览/本机列表。
- 最终定向 XCTest 9/9 通过，8项单元与1条UI；四张截图、12文件范围格式化、源码与构建一致性、保存负向门禁归档完成。临时641.4MiB产物已清理，磁盘可用空间增加615.0MiB，剩余70.86GiB。
- 本轮交付已完成，[Report](../reports/2026-11-25-ios-dungeons-touch-engine.md)与[证据](../../dev-docs/validation/2026-11-25-ios-dungeons-touch-engine/README.md)已收口；真实游戏包与实机串流接续设备侧验收。

## 本轮临时路径

- 临时根：`/private/tmp/xbxrc-ios-dungeons-touch-20261125-dpMHRj`。
- 证据目录：`dev-docs/validation/2026-11-25-ios-dungeons-touch-engine/`。
- `execution-manifest.json` 记录本轮源码基线及产物；前轮资源和共享构建缓存继续保留。
