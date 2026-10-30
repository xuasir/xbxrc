# 统一手柄接入与设置改造 RFC

## Status

- Completion: 已完成（改造与本机验证；Windows/ROG实机单列待验）
- Current State: completed
- Owner: Codex（按用户要求单代理执行）
- Last Updated: 2026-10-30
- Execution authorization: 用户已确认设计，并通过目标“按方案推进手柄输入部分的改造”授权执行。

## Background

实施依据：[统一手柄与设置设计](/Users/guo.xu/Documents/code/games/xbxrc/docs/isu/2026-10-27-browser-gamepad-rog-fse-design.md)。当前浏览器播放器仍消费原生快照，前后端输入归属与overlay gate重叠，设置入口保留设备恢复与原始映射细节。

## Goal

1. GamepadBackend统一设备/标准状态/触觉接口，Browser和Rust两个实现，启动时选择一个，默认Browser。
2. 公共输入控制器唯一管理活动设备、后台策略、UI/串流归属、逐控件交接和释放。Windows默认keep-active；follow-app通过单一应用生命周期事实接入，未知状态允许输入。
3. 导航、串流、键盘模拟、震动和设备展示全部接入公共合同；移出原生route镜像、全槽归零等待、重复overlay门禁和触屏恢复动作。
4. 设置按设备与行为、操控、震动、测试与高级四组改造；按设计提供后台策略、游戏布局/死区/反转、键盘模拟、震动、测试、后端选择与恢复偏好。
5. 一次持久化和配置应用，游戏映射与UI标准导航独立，后端选择下次启动生效。旧 profile 存储原样保留，桌面退出旧布局读取与导入。

## Scope

- src/shared/gamepad、navigation、player输入/触觉、串流输入路由、AppShell、设置与配置合同。
- src-tauri及ohmygamepad必要适配：按后端选择启动采样、统一原生状态输出、应用生命周期事实、设置默认/校验/迁移。Rust媒体输入消费接公共输入合同，保留原媒体执行。
- 文档、合同/集成测试、本机类型/构建/格式化检查。
- 媒体引擎、WebGL/恢复策略瘦身和FFmpeg发行裁剪属于总体方案后续媒体任务。

## Plan / Progress

- [x] P1 统一逻辑状态、设置模型、后台策略、后端合同与Browser实现。
- [x] P2 Rust后端适配、单一装配与应用生命周期输入。
- [x] P3 公共InputController接导航、串流、键盘、触觉、设备展示；删除重复路由。
- [x] P4 四组设置、能力反馈、录入/测试/草稿、配置迁移。
- [x] P5 完整回归、前端类型/构建、Rust检查、文档与Report。

## Validation

- [x] 后端生命周期：完整初态、空洞/index重用、热插拔、停止后无迟到输出、触觉失败。
- [x] 路由：keep-active忽略前后台、follow-app明确后台释放、未知状态放行、菜单/设备/会话交接、逐控件归位。
- [x] 输入集成：UI保持标准映射、游戏映射只应用一次、持续状态/释放、键盘与物理输入源显式选择、组合键/软Home、中性同步。
- [x] 设置：有效旧值保留、保存失败、草稿取消、恢复范围、能力缺失、后端下次启动生效、测试/录入退出。
- [x] pnpm lint:fix、pnpm exec vue-tsc --noEmit、pnpm test、pnpm build。
- [x] cargo fmt及改动涉及的Rust测试/检查。
- [x] 更新架构说明与Report，逐项核对完整目标。

## Platform validation

用户当前无法验证Windows/ROG。该项单独记录待验证，公共实现与本机检查继续执行。keep-active保证应用路由持续允许消费，浏览器挂起/设备权限仍属于运行时边界。最终报告精确列出已验证范围与平台待验项。

## Execution Notes

- 2026-10-27：基线78e82a5a；工作树只有三份本任务设计文档。读取当前代码和task-run，开始P1。执行目标已明确授权，直接按已确认设计实施。

- 2026-10-27：P1–P4落地。Browser/Native适配、公共输入、四组设置接线完成；前端187项测试、Rust公共库36项、配置5项及输入队列2项已通过。随后完成界面验收、键盘交接回归和最终构建。

- 2026-10-27收口：189项前端测试、36项公共Rust测试、3项Rust输入端口测试及5项配置测试通过；lint、typecheck、build、cargo fmt/check通过。报告：[`统一手柄改造报告`](../reports/2026-10-27-unified-gamepad-input.md)。Windows/ROG实机仍按Platform validation独立待验。

## 已授权精简与样式修订（2026-10-27）

用户要求精简功能代码、删除部分向下兼容逻辑，并采用现有页面风格。本次沿用已确认的输入合同：

- 删除旧 profile 导入、相关 DTO/文案、额外配置读取与旧设置工具入口；Native 快照采用当前必填 deviceSamples 合同。
- 删除 InputService/RumbleService 无效配置参数、重复 ControlTransport 透传、空生命周期适配；震动报文解析直接返回效果列表，保留当前服务使用的两种报文格式与 Pad0 过滤。
- 从 SettingSectionList 提取现有 SettingRow 与分组样式供手柄页复用；开关沿用 SettingToggleRow，编辑器操作置于 SettingModalShell 固定底栏，颜色统一使用主题变量。
- 公共输入路由、keep-active 默认、设备/会话释放、capture 与几何导航继续沿用现有实现。旧存储数据及通用 Rust 库 API 保留。

## 三项复杂度收口（2026-10-29）

用户通过关联聊天明确的三项方案及“继续解决剩余的三个问题”授权执行。沿用现有统一输入边界，三项修复及本机验收于 2026-10-30 完成：

- [x] Native：现有 InputCore/SDL3 service 增加设备标准态处理模式，桌面退出槽位选择、profile 映射、activity 选择与绑定同步；桌面 RPC/事件只输出连接设备的 ID、名称、触觉能力与标准状态。通用库默认逻辑槽位模式继续服务库级调用。
- [x] 设置：提取面板会话生命周期，草稿在打开时初始化；录入与实时测试各持有独立期限和计时器；关闭、切换、断连与卸载复用统一释放入口，隔离迟到焦点恢复与触觉结果。
- [x] 遗留：移除无消费者的快照查询 RPC、旧 profile DTO、默认键与配置分组；历史存储按现有 patch 存储方式保留。键盘桥复用公共中性状态，配置在偏好变更时应用，提供完整监听清理。

验收：标准态模式原始值与热插拔发布、逻辑槽位模式回归；面板超时、切换、断连、卸载与重开隔离；键盘持键、配置变更与监听退出；前端 lint/typecheck/test/build，Rust fmt/test/check，diff 检查。Windows/ROG 实机范围沿用 Platform validation。


2026-10-30 收口证据：桌面启动启用 `StandardDevices`，核心跳过活动设备选择、游戏 profile/槽位采样和 ArcSwap 槽位发布，SDL3 service 跳过 sampling binding 同步。启动返回与事件共用紧凑投影，`devices` 中每个连接设备只包含 `deviceId/name/supportsRumble/state`，标准值及热插拔持续发布。通用库默认 `LogicalPads` 模式回归通过。

设置会话统一拥有面板与录入状态，10 秒录入和 20 秒测试各使用独立截止时间；关闭、切换、设备变化及卸载统一清理，保存、触觉和延迟焦点恢复校验当前会话身份。键盘配置只在初始化与成功保存后通知，持键时变更映射/来源先释放，真实松开后下一次按下使用新配置；安装与卸载覆盖全部 DOM 和配置监听。

验证记录详见[改造报告的 2026-10-30 补充验收](../reports/2026-10-27-unified-gamepad-input.md)。Windows/ROG 范围继续沿用 Platform validation。
