# iOS 官方 TAK 兼容范围

本表记录地下城保存修复交付时的兼容范围。后续动态状态、连发、色板及图标进展见[能力补齐矩阵](../2026-11-25-ios-tak-completeness/support-matrix.md)。本轮目标是补齐用户截图暴露的保存阻断，以及已有手柄输出合同内的触控能力。

字段依据为微软 `xbox-game-streaming-tools` commit `99d5449bb054908876ceb867f948652f3b9cb41b` 的 [TAK v4.1 schema](https://raw.githubusercontent.com/microsoft/xbox-game-streaming-tools/99d5449bb054908876ceb867f948652f3b9cb41b/touch-adaptation-kit/schemas/layout/v4.1/layout.json)；[driving 原件](driving-original.json)和[fighting 原件](fighting-original.json)声明官方 `v3.1` schema。

| 能力 | 当前实现范围 | 代码依据 |
| --- | --- | --- |
| 版本识别 | 支持已知 1.0—4.1 版本的原有路径、官方 `v` 前缀路径与既有 `latest`；更高版本继续进入能力诊断 | `TouchLayoutNormalizer.supportsSchema` |
| 手柄输入 | button、joystick、directionalPad、throttle；本轮补 touchpad、六/八键 arcadeButtons、方向键 deadzone、单轴约束、两类响应曲线 | `TouchLayout`、`TouchControlRuntime`、`TouchLayoutGeometry` |
| 触摸板时序 | 原始采样时间计算速度，20ms 静止归零，重新移动即时响应，结束/取消/切换归零 | `TouchInputAccumulator`、`TouchControlRuntime.advance` |
| 图片 | 已解码 CGImage 复用；控件背景、faceImage、joystick/throttle knob 背景与前景、button pullIndicator 前景；已知 idle/moving/activated/toggled/pulled/disabled 状态素材切换 | `TouchLayoutSurface`、`TouchLayoutNormalizer.supportsImage` |
| 颜色与透明度 | 本轮补三/四/六/八位 hex 色、整体与组件 opacity、官方 AxisCap 和 PullIndicator 背景颜色 | `TouchControlDrawing.color`、`updateArtwork` |
| 资源引用 | Rust 有界展开包内 JSON Pointer 与静态 definitions；残留引用仍显示诊断 | `crates/xbox-touch-layouts/src/references.rs`、`TouchLayoutNormalizer.normalizeNode` |
| 保存门禁 | 缺失/损坏图片、未知 schema/字段/动作/图片位置继续阻止保存；面向用户合并重复原因，内部逐路径诊断保留 | `PreparedTouchLayout`、`SavedTouchLayoutsStore`、`TouchCapabilityReport` |

完整规范对齐还包含动态 Context/游戏状态引用、传感器宿主输入、鼠标/键盘输出、系统/布局命名颜色、渐变、官方完整图标库及其精确样式。当前手柄输出、输入宿主与绘制实现分别限定了这些范围；对应扩展需要沿既有公共合同逐项实施。

官方 `AxisCap` 与 `PullIndicatorBackground` 只定义颜色。本轮对这些位置的图片继续报告“此图片位置等待绘制支持”，并通过定向负向用例检查保存门禁。用户截图中的大量重复原因来自控件背景等同类图片字段在多个控件和样式中的重复出现；修复这类基础能力可同时消除多条诊断。

本轮 UI 冒烟使用官方格式夹具，覆盖第二项布局的卡片保存、预览拖动、保存退出和本机库。用户游戏包原件与实机串流的交互验收由设备侧完成。
