# 手柄统一接入：Browser / Rust 实现与后台策略

日期：2026-10-27。状态：设计细化，产品代码尚未修改。基线：`78e82a5a`。

## 1. 当前决定与范围

采用统一的 GamepadBackend 抽象，Browser 与 Rust 实现同一能力合同；应用装配时选择一个实现。公共输入层统一处理设备选择、逻辑状态、UI/串流归属和释放。后台策略独立于具体后端，也独立于媒体运行时。

用户当前无法验证 Windows / ROG Xbox Ally X。本轮继续完成接口、策略和迁移设计；Windows 真机兼容性单独标为待验证。此前“先做 ROG 探针再推进迁移”和“以 hasFocus + visibility 为候选门禁”的实施安排，由本次决定替代。

**推荐当前 Windows 默认使用“后台保持输入”策略。** 它覆盖 FSE 使用场景，直接忽略前后台变化对输入的自动切换。后续获得有明确系统语义、可靠的应用前后台来源后，可使用同一抽象启用“跟随应用前后台”。

本文件细化[浏览器稳定版总体方案](/Users/guo.xu/Documents/code/games/xbxrc/docs/isu/2026-10-27-browser-stable-desktop-plan.md)。稳定版默认装配 BrowserGamepadBackend；RustGamepadBackend 保留相同接口和独立实现边界，其发行装配与 Rust 媒体计划分别决定。

## 2. 决策链：近期为何反复

下表来自实际 Git 提交、对应实现和仓库任务记录。提交中记载的现场原因按历史报告引用，本轮独立核实的是代码变化；ROG 当前版本的实机结果仍待取得。

| 时间 / 提交 | 当时的改动 | 随后暴露的边界 |
| --- | --- | --- |
| 5 月至 6 月，`311e48a9`、`db2b70ea` | 从 SDL 事件推进到主动快照，再加入 XInput fallback/direct poller，应对发现为空或旧句柄采样停顿 | 多来源、设备索引对应和设备去重共同影响最终样本 |
| 10-21，`3b0918cd` | 移除 FSE/Win32 专线，判定变为 `visible && !minimized && focused_from_event` | 后续记录指出：启动缺少 focus 事件时，粘性布尔值长期关闭 gate |
| 10-22，`77074b87` | 事件焦点、窗口焦点、Win32 前台取并集，增加主动拉前台 | 恢复逻辑继续依赖同一前台判断，仍可能保持关闭 |
| 10-22，`401a01fa` | 改为 `visible && !minimized`，移除多信号并集与抢焦点 | `visible` 仍作为关闭条件；采样 lifecycle 仍需要额外提升 |
| 10-22，`5d7be0f1` | 限制触屏提示触发 reopen | 历史调用链确认：一次触屏能够走到关闭并重开设备句柄 |
| 10-23，`43ef85bc` | 修正 XInput Y 轴到逻辑态的符号 | 不同输入来源的坐标合同需要统一，只在发包边界做协议转换 |
| 10-25，`4a170bf4` | Windows gate 只看最小化，查询缺失保持上次状态 | 任务仍记为实机复验待补；未最小化时后台串流输入属于已知行为代价 |
| 10-26，`78e82a5a` | SDL 单源、常驻 runtime、移除自愈、Rust 单一 InputRoute | RFC 明确记录跳过掌机基线，Phase 1–5 代码完成，FSE 实机验收待补；浏览器消费合同仍有错位 |

关键历史证据：

- [6 月 SDL/XInput 排查报告](/Users/guo.xu/Documents/code/games/xbxrc/docs/reports/2026-06-29-windows-sdl3-fse-gamepad-input-audit.md) 区分 SDL 已发现设备与发现为空两种故障，也指出旧日志对实际采样来源的证明能力有限。
- [10 月稳定化 RFC](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-10-26-gamepad-fse-input-stabilization.md:218) 将 Phase 0 标记为按决策跳过；[执行记录](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-10-26-gamepad-fse-input-stabilization.md:232) 说明在 macOS 环境直接选定 SDL 方案；[任务台账](/Users/guo.xu/Documents/code/games/xbxrc/docs/project-task.md:11) 同时记录 Done 与掌机验证待补。
- [现状分析](/Users/guo.xu/Documents/code/games/xbxrc/docs/isu/2026-10-27-desktop-implementation-complexity-audit.md) 已复现浏览器 owner 被后端 Ui 快照覆盖的组合问题。

### 当前代码中的剩余复杂度

| 代码证据 | 新设计的处理 |
| --- | --- |
| [arbiter](/Users/guo.xu/Documents/code/games/xbxrc/src/shared/gamepad/business-input-arbiter.ts:53) 只读后端 route，[controller](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/stream/stream-input-route-controller.ts:214) 又乐观写 stream，[browser adapter](/Users/guo.xu/Documents/code/games/xbxrc/src/shared/gamepad/stream-input-consumer-adapters.ts:27) 启停为空 | 浏览器 owner 由前端当前业务事实唯一派生，原生 route 退出稳定输入链 |
| [wait-pad-neutral](/Users/guo.xu/Documents/code/games/xbxrc/src/shared/gamepad/wait-pad-neutral.ts:17) 要求所有槽位、所有轴/扳机严格等于零；等待本身无完成时限 | 仅跟踪活动设备交接时已按住的控件，各自归位后放行；其他控件立即可用 |
| [导航组合键](/Users/guo.xu/Documents/code/games/xbxrc/src/navigation/core/gamepad-listener.ts:131) 和 [InputService 组合键](/Users/guo.xu/Documents/code/games/xbxrc/src/player/app/input/InputService.ts:152) 分别维护状态 | 组合键在 owner 分发前识别一次，触发后持续消费到组合键全部松开 |
| [InputService](/Users/guo.xu/Documents/code/games/xbxrc/src/player/app/input/InputService.ts:79) 再监听 `stream-ui-input-mode`，保留 overlay suspend 和虚拟帧 bypass budget | 发送端只执行输入控制器给出的最终状态；软 Home 使用显式、有限的命令 |
| [GamepadDriver](/Users/guo.xu/Documents/code/games/xbxrc/src/player/infra/input/GamepadDriver.ts) 维护原生快照和 shadow 状态，虚拟按钮可屏蔽物理输入 | 活动物理设备保持唯一；键盘/软按钮经过明确命令或显式输入源选择 |

严格归零的风险来自仍有非零状态、其他槽位持续按住或初始快照一直缺失等条件；本轮没有将这些代码条件等同于已经复现的 ROG 现场故障。

## 3. 约束与选项

继续保留每设备单一来源、几何导航、统一逻辑坐标、单次协议 Y 转换、归属交接中性释放。采样由后端拥有，业务归属由公共输入层拥有，窗口状态由应用生命周期提供。

| 决策点 | 推荐 | 取舍 |
| --- | --- | --- |
| 接入方式 | 一个接口、两个后端、一次装配 | 业务消费同一数据；各后端保留自身硬件/API细节 |
| 后台处理 | 两种明确策略：跟随前后台 / 保持输入 | 可靠状态下可自动释放；状态可靠性未知时优先保留输入 |
| 当前 Windows 默认 | 保持输入 | 接受后台游戏可能继续响应手柄的行为 |
| 实施前提 | 先完成公共合同和本机可做的验证 | Windows/ROG结果单独记录，后续补证 |
| 后端选择 | 高级设置或构建装配，默认Browser；设置变更下次启动生效 | 在线故障按该后端错误处理，后端更换在应用生命周期边界执行 |

FSE运行状态与应用前后台是两个概念。查询“系统当前是否启用FSE”本身不足以证明应用拥有输入。当前Windows统一采用保持策略，配置在启动装配时明确确定，省去FSE运行时识别和多信号融合。

## 4. 统一能力合同

```text
BrowserGamepadBackend ─┐
                      ├─ GamepadBackend（运行时选择一个）
RustGamepadBackend ────┘       │
                       公共 InputController
                设备选择 + 后台策略 + UI/串流归属
                         │                │
                    几何导航        串流输入发送端
```

接口范围固定为设备/状态输出、启动/停止和基础触觉：

```ts
interface GamepadBackend {
  start(onSnapshot: (snapshot: GamepadSnapshot) => void): Promise<void>
  stop(): Promise<void>
  rumble(deviceId: string, effect: RumbleEffect): Promise<RumbleResult>
  stopRumble(deviceId: string): Promise<void>
}
```

上述为能力边界示意；GamepadSnapshot、RumbleEffect和RumbleResult在实施时复用并收窄现有合同。快照包括当前设备、连接身份、能力、标准化按键/轴/扳机状态与采样序号/时间。基础震动返回applied/unsupported/failed，结果如实表达。

合同要求：

- start幂等，首次输出包含当前完整设备状态，静止和全零状态均有效；初始化失败以明确错误结束该次启动。
- stop幂等，解除订阅、结束该后端采样与震动，旧运行实例的迟到输出在适配层失效。
- 后端负责物理到标准布局转换；设备选择、用户映射/deadzone和路由由公共层统一执行一次。Rust适配时审计现有core过滤，避免重复应用。
- Browser实现负责Gamepad API和自己的rAF调度；Rust实现复用原生采样与现有桥。对上层交付同一逻辑数据与能力。
- 后端专注输入设备，接口保持与UI菜单、FSE判定和媒体运行时解耦。采样启停仅属于后端生命周期。
- 同一时刻只装配一个后端。热插拔交给该后端正常设备生命周期，后端切换需先停止旧实例并释放输入。

应用装配示意：

```ts
const backend = createGamepadBackend(config.gamepadBackend)
const input = createInputController({
  backend,
  backgroundPolicy: config.gamepadBackgroundPolicy,
  navigation: navigationConsumer,
  stream: streamInputConsumer,
})
await input.start()
```

浏览器媒体可以消费任一已装配手柄后端。未来Rust媒体接入公共路由和逻辑帧合同，原生快路径按同一归属指令执行；采样来源与媒体执行方式各自选择，业务层保持单一决策者。

## 5. 前后台只决定是否消费

### 5.1 两个策略与一个规则

```ts
type BackgroundPolicy = 'follow-app' | 'keep-active'

// true=明确后台，false=明确前台；undefined=读取结果不可用。
function allowsInput(
  policy: BackgroundPolicy,
  isBackground: boolean | undefined,
): boolean {
  return policy === 'keep-active' || isBackground !== true
}
```

公共路由只做一次派生：

```text
允许消费=false                         → blocked
允许消费=true，UI占用或串流尚未就绪      → ui
允许消费=true，串流就绪且UI未占用        → stream
```

前后台是二态业务事实；undefined仅表示观测缺失，并按允许消费处理。可靠性是生命周期来源在平台接入时的设计保证，运行时直接使用该来源的结果；去掉健康度推断、评分、宽限窗口、重试次数和自动切换策略。

| 场景 | 跟随应用前后台 | 后台保持输入 |
| --- | --- | --- |
| 明确前台 | 正常消费 | 正常消费 |
| 明确后台 | 中性释放，停止UI repeat与震动，owner=blocked | 保持原owner和输入 |
| 状态读取失败/缺失 | 按允许消费处理 | 保持原owner和输入 |
| 前台恢复 | 用当前快照重新交接 | 保持正常处理 |
| 应用菜单打开/关闭 | 正常切换UI/stream | 正常切换UI/stream |
| 设备断开、会话退出、显式暂停 | 正常释放对应输入 | 正常释放对应输入 |

### 5.2 可靠来源与当前默认

“跟随应用前后台”消费一个应用活动来源。当前实现使用Tauri主窗口onFocusChanged和一次isFocused初态；它表达窗口焦点，Windows/FSE中的可靠性保留待验证。后续完善来源时须覆盖应用内子窗口、启动初态和成对变化。DOM hasFocus、visible与FSE标志的拼接退出手柄模块。

当前证据尚未确认Windows/FSE的可靠来源，因此Windows默认keep-active。此默认覆盖窗口化、全屏与FSE，用户可以在“设置 → 手柄 → 后台输入策略”中修改。其他平台也以已明确的生命周期合同决定默认，来源可靠性未知时同样选择保持。已保存的用户选择优先于平台默认。

后续采用可靠来源时，只替换应用生命周期提供方并明确启用follow-app。Browser/Rust后端和UI/串流消费者沿用现有接口。用户明确修改策略时立即计算一次归属；进入blocked执行统一释放。

### 5.3 保持策略的准确语义

keep-active保证应用自己的焦点、可见性与前后台策略始终允许消费；菜单、设备断开、停止串流和用户显式暂停仍正常生效。系统覆盖层与其他应用活跃期间，当前游戏可能继续接收手柄；这是保持策略的明确取舍。

浏览器实际供给仍受WebView调度、页面挂起和Gamepad API权限约束。“保持输入”只承诺应用层持续允许输入；系统冻结期间的采样和发送能力由运行时决定。真实设备移除或API不可用按后端设备状态处理，恢复执行后读取当前状态，中性同步与业务交接沿公共合同进行。

此前读取的[W3C Gamepad规范§12.1](https://www.w3.org/TR/2025/WD-gamepad-20250710/#dom-navigator-getgamepads)还规定首次手柄手势前可能返回空数组，权限限制可产生SecurityError。设备暴露、实际采样与业务归属分别表达。采样计数和timestamp只作观测，静止输入保持正常状态。

### 5.4 设置入口与生效方式

位置固定为“设置 → 手柄 → 后台输入策略”，作为应用级选项，Browser与Rust后端共用；设备切换、媒体模式切换保持用户选择。

| 选项 | 设置页说明 | 配置值 |
| --- | --- | --- |
| 后台保持输入 | 切到后台时继续接收手柄输入。适合掌机FSE，后台游戏也可能响应手柄。 | keep-active |
| 跟随应用前后台 | 应用进入后台时暂停手柄输入，返回前台后恢复。 | follow-app |

使用现有配置服务持久化，字段建议为 `gamepad_background_policy`，值限定为上述两项。Windows默认keep-active；缺失或非法旧值按平台默认迁移。正常升级保留有效用户选择，重置手柄设置时恢复平台默认。

用户切换后保存配置，保存成功立即通知公共InputController重新计算归属；保存失败保留原策略并显示失败反馈。整个过程保持现有手柄后端实例和串流会话。进入blocked时执行统一中性释放、停止震动与UI连发；恢复允许消费时按当前菜单占用和串流状态交接。用户选择keep-active之后，系统焦点事件始终保持这项策略。

后台策略控件只呈现两种策略及其行为说明。平台来源的可靠性和状态缺失处理沿用前述合同，策略变化完全由用户操作或显式恢复默认触发。

### 5.5 手柄设置整体重设计

目标是让用户完成选择手柄、设置后台行为、调整操控、配置震动和测试设备五类操作。设置页统一消费公共输入状态与配置服务，Browser/Rust共享页面；后端差异通过真实能力表达。

#### 当前入口与调整依据

主设置页实际由[设置schema](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/settings/setting-page-schema.ts:165)和[SettingInputToolsSection](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/Setting.vue:702)装配，主项为采样率、震动和强度，再打开测试与映射工具。[SettingGamepadSection](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/settings/SettingGamepadSection.vue)另有设备操作与重复映射实现，当前主页面采用前述入口。实施时收敛同类逻辑，并按引用关系清理旧入口。

现有映射保存[取第一个profile编辑，再构造单元素profiles写回](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/settings/SettingInputToolsSection.vue:383)，还同时调用原生配置RPC和持久化RPC。新设计采用统一配置写入与公共输入层应用，完整保留旧配置记录，按迁移合同转换。

#### 页面结构

保留当前设置导航框架，手柄页展示为“设置 → 手柄”，组织为四组：

| 分组 | 首层内容 | 二级内容 |
| --- | --- | --- |
| 设备与行为 | 当前手柄、后台输入策略 | 多设备时打开选择列表 |
| 操控 | 游戏按键布局、摇杆设置、键盘模拟 | 映射编辑、左右死区与视角反转、键盘绑定 |
| 震动 | 游戏震动、游戏震动强度、界面震动 | 简单选项直接修改 |
| 测试与高级 | 测试手柄、高级 | 输入可视化与震动测试；接入方式、诊断、恢复默认 |

首层使用整行可聚焦控件和当前值摘要。复杂编辑进入现有样式的二级面板；单页保持单列以适应掌机。返回后焦点回到来源行，设备列表变化时优先保留当前项目；触屏与手柄导航使用同一份状态。

#### 项目、默认值与范围

| 项目 | 选项与新安装默认值 | 范围与生效方式 |
| --- | --- | --- |
| 当前手柄 | 显示名称、连接状态；仅一台可用设备时直接选择 | 本次连接身份，选中后保持锁定；显式选择立即交接 |
| 后台输入策略 | 后台保持输入 / 跟随应用前后台；Windows默认保持 | 全局，Browser/Rust共用；保存成功立即生效 |
| 游戏按键布局 | 标准（默认）/ 自定义 | 全局标准逻辑布局，只作用于游戏输出；编辑后应用 |
| 左摇杆死区 | 0–30%，步长1%，默认10% | 游戏输出；二级面板编辑并预览，应用后生效 |
| 右摇杆死区 | 0–30%，步长1%，默认10% | 同上；与左摇杆独立 |
| 反转右摇杆上下 | 关闭（默认）/ 开启 | 游戏视角偏好；协议Y轴转换仍按固定合同执行一次 |
| 键盘模拟 | 关闭（默认）/ 开启 | 开启后键盘作为当前游戏控制源；物理手柄保留UI导航用途 |
| 游戏震动 | 开启（默认）/ 关闭 | 全局游戏触觉；关闭立即停止正在执行的游戏震动 |
| 游戏震动强度 | 轻（默认）/ 标准 / 强 | 沿用realistic/enhanced/full实际预设；游戏震动开启时可编辑 |
| 界面震动 | 关闭（新安装默认）/ 开启 | 仅UI导航反馈；已保存的ui_haptics选择继续保留 |
| 手柄接入方式（高级） | 浏览器（默认）/ 原生 | 仅展示安装包包含的实现；保存后下次应用启动生效 |

默认死区沿当前0.1基线；游戏扳机死区沿现有0.03固定值，首版保持算法参数内聚。具体数值属于用户操控偏好，调整后的原始/有效状态可在测试面板比较。

后台策略、触觉、游戏布局和操控偏好为全局配置。标准逻辑布局让更换Browser/Rust或更换标准手柄时保持同样行为。当前设备身份只用于连接期间选择；浏览器index与可变id保持运行时用途。首版采用一套全局游戏布局，后续确有需要再扩展每设备配置管理。

UI导航使用标准布局和现有导航阈值，游戏分支应用自定义映射、死区与视角反转。这样用户修改游戏A/B或摇杆方向后，设置页仍按标准确认/返回方式操作。标准化之前的物理兼容映射属于后端适配职责；用户页面基于语义按键展示。

#### 按键与键盘编辑

游戏映射采用“选择游戏动作 → 按下目标控件 → 保存草稿”的流程，展示“A、B、LB、右摇杆按下”等名称。普通数字按钮之间允许重映射；LT/RT保留模拟量并支持互换；摇杆方向使用专门操控设置。系统专用键按平台能力处理，远端Home保留软按钮入口。

一个来源控件对应一个游戏动作。选择已占用来源时交换两项绑定，并在草稿中显示结果；“恢复标准布局”只恢复当前草稿。开始录入时吸收已有按下状态，使用下一次新边沿；录入阶段独占相应输入，完成后执行公共交接。

录入中所有可映射的手柄按钮都用于绑定，取消使用屏幕按钮或Esc，并提供10秒无新按键时结束录入的有限等待。结束录入保留编辑草稿；只有应用才写入配置。取消、关闭或设备断开时退出录入，设置导航恢复正常。

键盘模拟在独立二级面板启用和编辑，复用现有默认绑定。开启时明确显示“键盘正在控制游戏”；关闭后游戏控制源回到所选手柄。键盘与物理手柄按明确来源选择消费，文本输入框、映射录入和UI菜单使用各自输入范围。DOM键盘失去按键事件来源时清理已按下状态，遵循其输入源语义。

#### 测试、能力与诊断

“测试手柄”展示全部可用按钮、两根摇杆与两只扳机，分别标注标准输入和游戏设置后的效果。测试目标是当前明确选择的设备。无设备时保留通用设置，测试和录入给出“连接手柄并按任意键”的提示。

进入按钮测试阶段后，测试面板占用输入并暂停普通导航动作，避免B关闭页面或方向键移动焦点。测试轮次最多20秒自动返回可导航状态，也支持触屏按钮或Esc结束；所有退出路径释放capture和测试震动。计时只属于用户启动的测试动作。

震动测试只在当前设备支持时启用，执行一次200ms基础震动，采用当前强度预设，点击期间限制重复执行。显式测试可以独立于游戏/UI开关运行，完成后自动停止并保留原配置。unsupported与执行失败显示具体结果；API返回成功表示命令已提交，用户感受到震动属于实际验证。

能力缺失时保留用户触觉偏好，控件显示“当前手柄暂不支持”；切换到支持设备后恢复编辑。接入方式只有一个可用实现时在高级详情显示当前方式；双实现安装包提供选择，变更后明确显示“下次启动使用……”和当前正在运行的实现。

高级诊断只读显示后端、当前输入去向与原因、设备能力、实测采样频率、最近错误，并提供导出。采样频率按实际数据计算，静止状态只表示当前值保持。诊断使用公共层的已有状态，面板开关保持同一采样和路由行为。

#### 删除与归并

| 现有项目 | 新设计 |
| --- | --- |
| 250Hz等采样率选择 | 从常规设置删除；后端管理实际节拍，高级详情只读显示 |
| 重新扫描、恢复采样、主采样设备 | 删除前两项日常操作；设备选择统一称“当前手柄” |
| 串流转发开关/路由状态摘要 | 路由作为高级只读事实，设置只表达后台行为与输入偏好 |
| 原始Button索引、设备matcher、filter epsilon | 移入兼容适配或诊断；游戏映射使用逻辑动作 |
| 调试开关与单行轴值 | 合并为测试面板和高级诊断 |
| 常规页的界面震动入口 | 迁移到手柄页，继续使用同一ui_haptics配置；界面音效保留常规页 |
| 重复映射弹层与保存逻辑 | 合并一个编辑器与一次配置应用入口 |

#### 保存、重置与旧配置迁移

后台策略和震动开关/强度保存成功后即时生效。映射、摇杆与键盘配置使用面板草稿，预览限测试区域，点击“应用”后一次保存并生效；关闭未应用面板丢弃草稿。手柄接入方式在下次启动装配，切换设置期间保持当前会话和后端实例。

所有设置复用现有配置服务。公共InputController消费成功保存的配置；失败时保留原有效值并显示错误。设备选择属于运行时操作，切换前释放旧输入；设置编辑保持串流菜单capture，关闭后按统一交接规则恢复。

建议字段和迁移如下，字段名称以实施RFC最终合同为准：

| 字段 | 处理 |
| --- | --- |
| gamepad_background_policy | 已确定的两种后台策略；Windows缺失时默认keep-active |
| gamepad_backend | browser/native；默认browser，只允许构建包含的实现，启动时装配 |
| vibration / vibration_strength | 沿用现有字段及预设值，只更新用户文案 |
| ui_haptics | 沿用字段和已有用户值，新安装默认false；展示分组与存储分组分别处理 |
| gamepad_game_controls | 新的后端无关游戏布局、左右死区与右摇杆反转设置 |
| gamepad_control_source | gamepad/keyboard；默认gamepad，键盘开关映射到这一单一选择 |
| gamepad_keyboard_mapping | 保留有效已有绑定，关闭模拟时保留配置 |
| gamepad_device_profiles | 完整保留旧记录；能够确认语义的旧映射可供用户导入为全局游戏布局，其余保留兼容数据 |
| polling_rate | 退出Browser配置消费与主设置，旧值保留迁移记录；Rust实验参数按实际后端合同处理 |

迁移保持旧profile集合完整，游戏逻辑布局导入须明确说明适用于所有标准手柄。原始索引或设备匹配条件可继续供Rust兼容适配使用，标准化后的用户映射只执行一次。该规则用于避免把一种设备的旧索引直接套到其他后端或设备上。

高级页提供“恢复手柄偏好”，确认页列出后台策略、游戏操控、键盘模拟与震动将恢复默认；恢复后统一保存并交接。运行中的接入方式、现有设备连接和旧兼容记录保持原状。各编辑面板可独立恢复当前草稿，避免一次映射重置连带清空其他设置。

#### 实施与验证范围

优先改造setting-page-schema与现有InputTools入口，共用一个配置模型、映射编辑器和测试面板。当前GamepadBackend能力快照支持所有设备状态展示；用户设置经公共层应用。配置白名单、默认值、分组、前端schema、国际化文案与迁移同步更新，使用现有geometric navigation和设置控件。

新增设置验收覆盖：默认值/旧值保留、保存失败、草稿取消、全局布局跨后端一致、游戏映射与UI导航独立、测试/录入退出、设备断开、触觉unsupported、已选择后台策略保持、接入方式下次启动装配、恢复偏好范围。Windows/ROG物理行为继续单独待验，当前完成设计与可执行的合同验证规划。

## 6. 公共输入层保留的最少职责

1. **设备选择与标准状态。** 默认选择一个活动设备并保持锁定，切换由明确操作触发；UI使用标准逻辑布局，游戏分支统一应用用户映射/deadzone一次，串流协议仅在编码边界转换。
2. **路由与释放。** 后台策略、UI capture、串流就绪共同产生一个owner。离开stream补中性状态，离开ui清repeat，交接时已按住控件各自松开后恢复，其余控件立即可用。
3. **消费与触觉。** UI复用几何导航，串流复用输入编码；单一发送出口控制序号、缓冲和协议metadata。震动只到当前活动设备，并随对应业务退出停止。

前后台切换始终只影响消费，采样实例保持既有生命周期。触屏事件只触发实际UI操作；诊断日志记录事实；两者均与设备恢复职责解耦。

既有组合键和软Home属于消费层。本次接入设计移除之前提议的新增100ms组合窗口，保留现有交互范围，集中识别并单独验证重复触发与按键释放。后续快捷键UX调整作为独立决策处理。

## 7. 当前可做的实施与验证

按以下顺序推进：

| 任务 | 产物/验收 | 分类 |
| --- | --- | --- |
| 公共合同与路由 | GamepadBackend接口、后台策略、统一owner与幂等释放；模拟后端验证 | complex |
| Browser实现及业务接线 | 现有导航/播放器消费抽象；移除原生route镜像、空adapter与二次overlay gate | complex |
| Rust实现对齐 | 复用原生采样和现有桥，适配统一状态/触觉，去掉后端业务路由决策 | complex |
| 设置、发行装配与配置迁移 | 按四组设置重做页面，统一草稿/即时应用/重启生效规则；完整迁移旧配置；默认Browser与Windows keep-active，后端选择与媒体独立 | complex |
| 平台补验 | Windows/ROG场景结果独立登记，具备环境后完成 | validation |

现在可完成的合同验证：两种后台策略真值表；设置保存/重载/恢复默认与失败反馈；有效旧配置保留；策略切换保持后端与会话实例；缺失前后台事件与未知状态；触屏/focus/visible任意变化下keep-active保持owner；菜单仍正常capture；全零快照；活动设备锁定；释放与迟到快照；启动/停止幂等；基础震动unsupported；后端更换后消费者保持同一行为。模拟后端属于测试夹具。

同一份合同测试用于Browser和Rust适配器，平台相关采样另外验证。已有的原生/前端测试可继续作为回归，代码变化后运行相应格式化、类型检查与构建。Windows交叉编译只证明构建结果，ROG物理行为单独补验。

后续Windows测试按所选策略验收：keep-active检查FSE、触屏、覆盖层和任务切换均保持应用owner；follow-app检查权威来源的后台释放和前台恢复。两种模式分别解释后台游戏响应。系统挂起、设备断开、UI菜单和会话退出均检查中性状态和资源释放。

## 8. 待补证据与完成口径

待补证据只有平台行为：Browser在ROG上的设备暴露/采样/震动、WebView后台调度，以及未来follow-app所需的可靠Windows应用生命周期来源。它们与公共抽象及保持策略的设计分开推进。

历史提交分析与现有归零/Y轴测试结果保留为前轮证据。本轮交付是方案修订与文档核对。Windows/ROG兼容性保持“待验证”；公共合同和本机可执行任务按各自实际结果收口。

建议先落实统一GamepadBackend与公共InputController，稳定版装配Browser实现，Windows默认keep-active，再逐步对齐Rust实现。

## 2026-10-27 实施后修订

用户在改造后授权精简兼容逻辑并沿用现有设置风格。最终实现取消旧 profile 显式导入与兼容 DTO，历史存储记录保留；设置页共用既有选择行、开关、分组和弹层样式。本文前文的旧布局导入/转换方案由此次决定替代，实施与验证以[统一手柄 RFC](../rfcs/2026-10-27-unified-gamepad-input.md)及[报告](../reports/2026-10-27-unified-gamepad-input.md)为准。
