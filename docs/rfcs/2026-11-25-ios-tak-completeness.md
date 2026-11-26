# iOS 官方与社区 TAK 能力补齐 RFC

## Status

- Owner：主窗口，单窗口直接执行。
- Current State：completed。
- 用户已查看前轮能力范围，并明确要求“补齐一下”。本轮按该授权完成剩余能力，保持现有 UIKit 输入/绘制与 Rust 数据/资源/协议边界。

## 目标与范围

1. 官方 Context.state、definitions 和命名颜色保留动态来源；接入 showlayout、showlayoutv2、showtitledefault、hide、patchstate。状态更新按官方 JSON Patch 行为逐项应用，属性变化释放已被替换或隐藏的输入。
2. 补齐手柄 turbo 连发，按毫秒间隔切换输出，复用显示时钟与输入释放出口。
3. 用户在上轮最后明确“鼠标、键盘和传感器暂时先不处理”；这些能力保留可定位的保存门禁，串流保持当前手柄/触屏能力声明。
4. 系统/布局命名颜色、渐变、描边、全量官方内置图标与合法样式字段进入现有绘制，已解码图片继续复用。
5. 官方与当前接入的社区包装复用同一能力扫描、状态和输入执行。缺失资源、无效值、未知合同继续提供可定位的诊断。

## 实施合同

- Context 和静态资源规则放入 `crates/xbox-touch-layouts`；由既有 UniFFI bridge 暴露，Swift 只消费完成校验的数据和状态更新结果。
- `StreamInputSession` 与 `XboxStreamDataChannels` 继续承接手柄/触屏输入；扩展既有 input payload，复用当前输入版本和可靠通道。
- Swift 负责多指、屏幕几何与 Core Animation 绘制。动态消息沿已存在 message DataChannel 交给同一个 `StreamingTouchLayoutController`。
- 本轮按公开冻结 TAK schema/源码与项目既有协议证据实施。完整服务端响应与游戏内交互由用户设备侧验收。

## 分阶段执行

1. 读取官方 schema、冻结源码和当前宿主；登记字段/消息/输出合同及源码基线。
2. 实现上下文资源、动态消息与状态释放。
3. 实现手柄连发和输入生命周期。
4. 实现剩余样式与图标，统一官方/社区能力检查。
5. 一轮定向 Rust、Swift 与官方/社区 UI 冒烟；归档必要证据后清理本轮临时产物，更新 Report 和能力矩阵。

## 最小验收

- Context 静态解析和动态 JSON Patch、循环/缺失引用、合法状态值与局部操作失败。
- 真实官方和社区样本归一化、保存门禁、动态隐藏控件的输入释放。
- 连发时序、短按边沿、重复贡献和抑制/切换/结束释放。
- 鼠标、键盘、传感器保存门禁；命名颜色、渐变、图标绘制可见。
- 官方与社区各一条正式入口 UI 冒烟；Swift 格式、Rust格式、前端 lint 与必要边界检查。

## 证据

- [前轮能力范围](../../dev-docs/validation/2026-11-25-ios-dungeons-touch-engine/support-matrix.md)。
- [官方触屏协议](../references/official-touchscreen-protocol.md)、[布局与 Context](../references/better-xcloud-touch-layout-protocol.md)、[TAK 控件宿主](../references/tak-babylon-touch-engine.md)。
- 本轮 `dev-docs/validation/2026-11-25-ios-tak-completeness/execution-manifest.json` 保存源码基线与临时产物清单。

## 收口

本轮范围已实现；Rust36项和iOS9项定向验收通过，格式/边界检查通过。[Report](../reports/2026-11-25-ios-tak-completeness.md)与[证据](../../dev-docs/validation/2026-11-25-ios-tak-completeness/README.md)同步归档。鼠标、键盘和传感器按用户最后指令暂缓。

临时产物清理完成，释放约781.2MiB；清理清单与保留项见证据目录cleanup.json。
