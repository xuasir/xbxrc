# iOS 数据驱动触控布局与本地输入测试 RFC

## Status

- Completion: 已完成
- Current State: completed
- Owner: 主窗口
- Last Updated: 2026-11-18
- 用户已确认实施，控件沿应用矢量风格与低遮挡透明度要求实现；高清 Xbox 图用于符号核对，可按需核对官方公开 JavaScript。

## Goal

提供离线全屏触控测试页：切换通用 Xbox 布局与 Forza Horizon 5 两套社区布局，渲染真实布局字段，采集多指事件并展示标准手柄输出。输出终点为本地 recorder。

## Evidence

- 当前 `iosapp/XBXRC/Platform/Streaming/TouchGamepadOverlay.swift` 使用固定 SwiftUI 排列，每个 UIKit 子视图保存一个 activeTouch。
- `StreamInputSession.setTouchButton` 直接设置或清除全局按键位；新测试模块按来源聚合贡献，输出复用 `IOSStreamGamepadState` 的字段合同。
- Forza 游戏索引 `2030093255` 的模板包含 `layout-1` 与 `layout-1-throttle-pedal`，原文位于 `dev-docs/validation/2026-11-12-tak-layout-and-engine/examples/forza-layouts.json`。
- 第一套需要 joystick、button、directionalPad、cluster、layer、pullAction；第二套增加 throttle 的 relative/sticky 与上下扳机映射。
- 布局、几何、输入语义依据仓库已冻结的 `tak-core-protocol.txt`、`tak-core-geometry.txt`、`tak-core-input.txt`。实际渲染由 UIKit/Core Animation 承担。

## Code Ownership

新增代码集中在 `iosapp/XBXRC/Features/TouchControls/`。

| 文件 | 职责 |
| --- | --- |
| `TouchLayout.swift` | Codable 布局、槽位/cluster 联合类型、action/axis 联合类型与结构校验 |
| `TouchLayoutNormalizer.swift` | 按 schema 版本展开默认值、归一化社区包装，输出统一布局模型与能力诊断 |
| `TouchLayoutCatalog.swift` | 本地资源读取、通用/Forza 身份、两套 Forza 布局选择及加载错误 |
| `TouchLayoutResolver.swift` | 激活 layer 的有序合并、null 继承、blank 覆盖与稳定 controlID |
| `TouchLayoutGeometry.swift` | 安全区/viewport 到 wheel、inner/outer/cluster/upper/lower 的 socket 与命中形状 |
| `TouchControlRuntime.swift` | button、joystick、directionalPad、throttle 状态机及 action/axis 映射 |
| `TouchInputAccumulator.swift` | 指针附着/转交、按来源聚合、事件与快照、统一取消 |
| `TouchLayoutSurface.swift` | 单个多指 UIView、CALayer 绘制与 SwiftUI UIViewRepresentable |
| `TouchLayoutPlaygroundView.swift` | 全屏测试页、布局选择、输入面板及本地事件记录 |

装配修改：`ControllerView.swift` 的触屏设置增加“布局与输入测试”入口，`XBXRC.xcodeproj/project.pbxproj` 登记源码、资源与测试。

## Layout Contract

1. 读取社区 `schema_version/layouts` 包装与直接 `{content: ...}` Layout；通用布局由现有 Xbox 控件集合写成相同格式。
2. 保留 `name/author`、原始数组索引、null 与 cluster 层次。Forza 资源原文随来源说明入包。
3. 支持 left/right/center 的 inner/outer，以及 upper.right、lower 三组槽位。inner 最多四项，outer 八个物理槽位；cluster 占连续两个物理槽位。
4. 基础布局 null 留位；layer null 继承基础值，blank 清空对应槽位；按 layer 激活顺序合并。
5. 支持单 action、action 数组、layer action，axis 对象/数组及 throttle 的 axisUp/axisDown。未知控件、action、axis 输出结构化兼容诊断。
6. 几何生成 socket 的中心、大小、角度、scale、collisionRadius。轮盘位置/大小由 viewport 与安全区约束；绘制和命中共用几何结果。
7. styles.default 与状态样式合并，Forza 使用的 steering、switchCamera、gasPedal、brakePedal、handbrake2 等语义图标映射到本地矢量图形。

## Standard Compatibility

- 官方 TAK 以版本化 Layout/Manifest/Context schema 定义控件、槽位、动作、资源及动态状态；社区以 schema_version/layouts 包装复用 TAK content，并允许部分运行时默认值。
- 两类输入经 normalizer 进入同一布局模型。schema 版本与控件 type 决定解释逻辑，TitleId 参与目录查询和布局选择。
- Forza 是通用能力的验收样本。默认样式补全、cluster、layer、pullAction、sticky 实现为公共规则，供所有声明相同能力的布局使用。
- 每次加载生成能力报告，保留 schema 版本、所需控件/行为、未知字段、未解析资源及对应 JSON 路径；加载兼容性由实际实现能力判断。
- 第一轮覆盖直接 Layout 与社区常用控件；完整官方包需要继续覆盖历史版本转换、definitions/$ref、Context/Patch、locale/DPI 与图片资源。
- 通用性验收加入 Halo 原件的能力扫描和官方最小 Layout 解析，用于识别 shared type/字段与待扩展能力；实际官方 TAKX 与动态 Context 样本进入后续兼容验证。

## Input Contract

```text
UITouch 批次
  → 稳定 pointerID 与局部坐标
  → 命中/附着/转交
  → 控件状态机
  → 每来源贡献聚合
  → TouchInputFrame
  → 本地 recorder 与输入面板
```

- 指针保存 ID、phase、单调时间、position、delta、pressure 与当前 controlID。button 允许多个指针；joystick/throttle 保存操作指针身份。
- 普通 button 处理按下/释放及滑动转交；pullAction/toggle 使用持续附着。pullAction 按冻结源码的拖拉距离/方向规则增加贡献。
- joystick 区分 relative 与 absolute，处理 expand 半径、单位圆限制、actionThreshold 及 axis 映射；deadzone 按已核对的 TAK 输出偏移语义实现。
- directionalPad 处理中心死区与 exclusive/neighboring 方向；throttle 处理上下扳机、relative 与 sticky，上侧保持、下侧释放回零。
- 贡献按 `(controlID, pointerID, action)` 标识；toggle/sticky 使用控件持久贡献。按键按位 OR，模拟轴按来源相加后限幅，同键来源分别释放。
- layer 由来源引用计数激活；发生替换时取消退出控件的输入，保留有效控件与指针，重新命中采用原子批次。
- 每批输入形成有序事件与完整快照，短按保留 press/release 两条边沿。绘制更新按显示帧合并，事件采集随输入批次完成。
- 布局切换、旋转重排、页面关闭和进入后台统一取消指针、toggle/sticky/layer，发布 neutral 快照。

`TouchInputFrame` 字段：sequence、timestamp、pointer events、active controls/layers、`IOSStreamGamepadState`。测试页显示 buttonMask、双摇杆 XY、LT/RT、触点数、命中 controlID 与最近事件，并提供清空与导出本地 JSON。

## Delivery Order

1. 布局模型、资源、resolver、geometry：通用与两套 Forza 可切换，形成静态截图。
2. 统一触摸面与控件状态机：通用按键/摇杆/方向键、Forza 油门/刹车与组合操作形成可观测输入。
3. Forza layer、pullAction、throttle sticky 和取消语义完成；测试页接入控制器入口。
4. 定向单元/UI 验证与证据归档，记录模拟器及实机能力边界。

## Validation

- [x] 原始 Forza 两套 JSON 完整读取，cluster/null/layer 的槽位结果与冻结规则对照。
- [x] 手机/iPad、横竖屏控件布局可用，命中区域与绘制结果一致。
- [x] 纯 Swift 输入回放覆盖同时按键/摇杆/扳机、同键多来源释放、短按、rollover、layer/pullAction/sticky。
- [x] 取消、布局切换、旋转与后台后的状态为 neutral，所有活动指针和持续贡献释放。
- [x] XCUITest 从实际入口切换布局，真实按压/拖动后检查输入面板与截图。
- [x] 本地 simulator/device 编译、定向测试与 diff 检查完成，必要证据归档并清理本轮临时目录。

## Subsequent Scope

Halo rightstick 作为 touchpad、toggle 和复合 action 的下一组兼容用例；官方 TAKX/Context、远程资源获取、游戏身份自动选布局与真实串流输入接线分别推进。

## Progress

- [x] 核对引用聊天、现有 iOS 触控实现、Forza 原始模板与冻结 TAK 源码。
- [x] 提交按文件、字段、状态机和验收划分的代码计划。
- [x] 执行代码计划。

## Execution Notes

- 2026-11-18：主窗口开始直接实施；源码与临时产物按 `/private/tmp/xbxrc-ios-touch-layout-20261118/r1` 隔离，证据归档于 `dev-docs/validation/2026-11-18-ios-touch-layout-playground/`。
- 按用户最新要求，控件使用原生矢量、细描边与低饱和 ABXY 配色；高清手柄参考用于轮廓与符号核对。控件层透明底、默认 65% 透明度、实时调节，验证画面中心留空与实际透明通道。

- 2026-11-18 收口：通用/Forza 两套布局、官方最小 Layout、Halo 能力扫描完成；18 项回放、手机/iPad 入口 UI、设备目标编译、Swift format、工程格式和会话边界检查通过。最终截图与摘要见 [验证证据](../../dev-docs/validation/2026-11-18-ios-touch-layout-playground/README.md)，交付记录见 [Report](../reports/2026-11-18-ios-touch-layout-playground.md)。
