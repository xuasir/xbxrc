# Streaming Native Video Dual Window Architecture RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: TBD
- Last Updated: 2026-04-09

## Background

- 当前 canonical 桌面串流窗口主线已经收敛为“单主窗口 + 主窗口内 native video host + Web UI 叠加”。
- 这一方案减少了窗口管理复杂度，但也把几类本来边界不同的职责重新压回了同一个主窗口：
  - 视频内容承载与展示策略
  - Web UI / HUD / 页面路由
  - 窗口透明化时序
  - 全屏、尺寸、位置、生命周期切换
- 在现状下，如果继续增强串流展示质量、黑边策略、首帧透明切换和后续平台差异处理，`main` 窗口会同时承担“应用壳”和“视频承载壳”两种职责，后续边界会继续混杂。
- 当前用户目标已经明确切换为双窗口设计：
  - 使用“独立视频窗口 + 主 WebView 窗口”的双窗口架构。
  - 两个窗口行为保持一致。
  - 只有在串流画面实际出现后，主 WebView 窗口和网页内容才透明化。
  - 视频窗口默认优先保比例显示，留白区域使用黑底。
- 同时用户已经明确约束：
  - 视频窗口只做纯展示，不承担输入交互。
  - 不新增独立的 `window_coordinator` 模块，窗口编排能力继续收口在现有 `src-tauri/src/mods/native_video/*` 主线下演进。

## Goal

- 将桌面串流窗口架构调整为 canonical 双窗口主线：
  - `main`：主 WebView 窗口，负责页面、HUD、交互与路由。
  - `native-video-*`：独立原生视频窗口，负责纯视频展示。
- 明确并固化以下行为：
  - 视频窗口不承载输入交互、不抢业务焦点。
  - 视频窗口与主窗口在显示/隐藏、位置、尺寸、全屏等行为上保持一致。
  - 只有在视频首帧已经稳定显示后，`main` 窗口和网页内容才进入透明叠加态。
  - 视频默认采用“保比例 + 黑底留白”的展示策略，而不是拉伸填满。
- 保持 Rust 侧对 native window 与 video pipeline 的 owner 边界，避免前端直接演化出双窗口原生控制逻辑。
- 为后续平台打磨、显示模式扩展和异常回滚留出稳定演进路径，而不是只做一次性的最小改造。

## Scope

- In scope:
  - `src-tauri/src/mods/native_video/*` 的职责扩展与模块内分层整理
  - `src-tauri/src/shell/*` 与窗口生命周期相关的接入点调整
  - `src-tauri/capabilities/default.json` 中双窗口所需 capability 的确认与必要收口
  - `src-tauri/src/mods/app_state/*`、`src-tauri/src/mods/streaming/*`、`src-tauri/src/mods/xbxengine/*` 与串流生命周期/首帧事件对接
  - `src/pages/XStreamMainView.vue`、`src/App.vue`、`src/styles/base.css` 等 Web UI 透明叠加态的契约调整
  - 双窗口状态机、窗口同步策略、透明化时序、黑底保比例展示策略、异常与回滚策略
  - 相关验证方案、文档与任务跟踪
- Out of scope:
  - 重写现有串流协议、RTC、解码主线
  - 重做输入系统或重新设计 gamepad/focus routing
  - 引入 Electron 或其他并行桌面 runtime
  - 为视频窗口增加交互式控件、路由页面或业务状态管理
  - 在本 RFC 中承诺一次性完成所有平台视觉打磨细节

## Problem Statement

### 1. 单窗口主线让视频承载与 UI 壳层边界混杂

- 当前 `main` 同时承担了应用壳、Web UI、native video host 三种职责。
- 这导致以下策略互相耦合：
  - 页面背景是否透明
  - 窗口本身是否透明
  - 视频展示是否 ready
  - 串流异常时窗口如何回退
- 继续在这个模型上叠加策略，会让窗口展示语义越来越依赖隐含时序，而不是清晰状态机。

### 2. 首帧前后的视觉语义不够清晰

- 用户明确要求：只有在串流画面真正出现时，主 WebView 窗口与网页内容才透明化。
- 这意味着“进入串流页面”和“视频首帧稳定可见”不能被视为同一时刻。
- 如果没有稳定的首帧状态机，容易出现：
  - 提前透明导致露出桌面或底层空白
  - 首帧失败后主窗口留在透明态
  - 停流/断流后主窗口未及时恢复不透明

### 3. 视频展示策略需要从“尽量填满”切换为“保比例优先”

- 当前用户要求默认保比例展示，并且留白必须是黑底。
- 这要求视频窗口本身成为“视觉基底”，不能再依赖 `main` WebView 的页面背景来兜底。
- 如果黑底仍属于网页层，就会与“主窗口透明化”直接冲突。

### 4. 双窗口行为一致不能依赖前端分散控制

- 用户要求两个窗口行为一致，但视频窗口不处理输入。
- 这类一致性更适合由 Rust 侧统一编排，不能把尺寸、位置、全屏、可见性拆给前端分别控制。
- 如果缺少统一 owner，很容易出现：
  - 两个窗口偶发错位
  - 全屏切换不同步
  - 一个窗口隐藏了，另一个仍残留
  - 平台差异修复时出现双份逻辑

## Non-Goals

- 不在本次 RFC 内把视频窗口扩展为可交互窗口。
- 不要求视频窗口接收鼠标、键盘、gamepad 或焦点路由。
- 不在本次 RFC 内解决所有历史窗口遗留能力清理，但要给出清理方向。
- 不承诺首版就支持所有缩放模式；首版 canonical 模式是 `contain + black background`。
- 不新增 `window_coordinator` 独立模块；相关编排能力在 `native_video` 内部完成职责扩展与必要重构。

## Current State Summary

- `tauri.conf` 目前只静态声明一个主窗口。
- `native_video` 当前逻辑已经保留 `native-video-*` capability 入口，但现行视口目标固定回 `MainWindow`。
- `main` 窗口在启动时会根据配置/参数设置 fullscreen，并在当前主线下直接承担视频宿主职责。
- macOS 已有“关闭主窗口即隐藏”的桌面语义，这会影响双窗口停流与退出期的窗口回收策略。
- 当前 Web UI 已经存在 `stream-ui-window` 相关透明样式线索，但它们服务的是单窗口叠加模式，需要重新定义触发条件。

## Proposed Architecture

### Window topology

- 保持 `main` 作为唯一主应用窗口：
  - 承载全部 Vue 页面、HUD、控制层、设置、导航与错误态。
  - 继续作为应用的焦点入口与输入入口。
- 恢复并固化独立视频窗口：
  - 标签建议沿用 `native-video-main` 或与会话绑定的 `native-video-{session_id}` 命名约定。
  - 窗口仅负责原生视频渲染与黑底基底展示。
  - 窗口不承载 Web UI、不加载业务页面。
- 渲染层级：
  - 下层：`native-video-*`
  - 上层：`main`
  - 串流开始前：`main` 保持正常不透明
  - 首帧 ready 后：`main` 进入原生透明 + Web 内容透明叠加态

### Ownership

- `native_video` 模块成为双窗口视频展示的唯一 owner，负责：
  - 视频窗口创建、复用、布局同步、显示/隐藏、关闭回收
  - 视频窗口展示策略（黑底、保比例、present path）
  - 首帧 ready / 透明化触发 / 停流恢复等生命周期编排
- `shell` 只提供：
  - 应用启动期主窗口基础初始化
  - 平台级 window event 接入点
  - 退出/重启等应用级生命周期
- `streaming` / `xbxengine` 负责：
  - 通知串流开始、停止、错误和帧级 ready 事件
  - 不直接编排双窗口原生细节
- 前端负责：
  - 根据 Rust 透出的“透明叠加态”与串流 UI 状态应用样式
  - 不直接控制独立视频窗口的原生行为

### Internal structure inside `native_video`

- 不新增 `window_coordinator` 模块，但允许在 `native_video` 内部按职责细分子模块，例如：
  - `window_host.rs`：视频窗口创建、查询、基础属性同步
  - `layout.rs`：窗口尺寸/位置/全屏/保比例布局计算
  - `overlay_state.rs`：首帧后透明叠加状态机
  - `presenters.rs`：present 路径与视频提交
  - `policy.rs`：平台能力、展示模式与策略选择
- 关键原则：
  - 双窗口协调逻辑属于 `native_video` 的“展示 owner”职责
  - 不引入新的顶层 owner，避免主线模块继续分叉

## Window Behavior Model

### `main` window responsibilities

- 始终是应用的焦点入口。
- 负责 UI 交互、页面导航、HUD、错误态和设置层。
- 非串流期为普通不透明窗口。
- 串流期在满足条件后切换到透明叠加态：
  - 原生窗口透明
  - 网页背景透明
  - 页面仅保留需要可见的 UI 元素

### `native-video` window responsibilities

- 始终是纯展示窗口：
  - 不接业务输入
  - 不承载页面路由
  - 不创建 Web UI
- 背景固定为黑色，作为视觉基底。
- 默认展示模式为保比例居中显示。
- 留白区域保持黑底，不透明化后也不会露出桌面。

### Behavioral consistency definition

- “两个窗口行为保持一致”在本 RFC 中定义为以下语义一致，而非每个原生属性完全相同：
  - 同时出现/同时隐藏
  - 同步移动与同步调整尺寸
  - 同步进入/退出全屏
  - 停流后同步回到非串流态
- 不要求视频窗口复制 `main` 的所有系统级可见属性，例如焦点、任务栏表现或交互能力。

## Stream Lifecycle State Machine

### States

1. `Idle`
   - 仅 `main` 正常显示
   - `main` 不透明
   - 视频窗口不存在或处于隐藏/销毁态
2. `PreparingWindow`
   - 请求开始串流
   - 创建或复用视频窗口
   - 同步到与 `main` 一致的位置/尺寸/全屏
   - 视频窗口先显示黑底占位
   - `main` 保持不透明
3. `AwaitingFirstFrame`
   - 解码/渲染链路已经启动，但首帧尚未稳定 present
   - 允许视频窗口可见，但不允许 `main` 提前透明
4. `OverlayActive`
   - 视频窗口已成功 present 首帧，且窗口布局稳定
   - `main` 切换到透明叠加态
   - Web UI 透明背景样式生效
5. `Streaming`
   - 双窗口稳定运行
   - 窗口同步、保比例展示、HUD 叠加持续有效
6. `Stopping`
   - 用户退出串流、会话失败或应用准备退出
   - 先撤销 `main` 透明态，再隐藏/销毁视频窗口
7. `Failed`
   - 创建窗口失败、首帧超时、同步失败或 renderer 致命错误
   - 回退为仅 `main` 不透明态
   - 记录诊断并避免遗留透明窗口

### Key transitions

- `Idle -> PreparingWindow`
  - 进入串流页面并收到 Rust 侧开始准备串流的明确意图
- `PreparingWindow -> AwaitingFirstFrame`
  - 视频窗口已创建且完成首轮布局
- `AwaitingFirstFrame -> OverlayActive`
  - 已收到首帧 present 成功事件
  - 若需要，增加一帧或短暂稳定门限，避免用“收到帧”替代“已显示帧”
- `OverlayActive -> Streaming`
  - 透明态生效完成，运行稳定
- `Streaming -> Stopping`
  - 正常停流、用户退出、会话结束、窗口关闭
- `PreparingWindow/AwaitingFirstFrame/OverlayActive/Streaming -> Failed`
  - 任一关键步骤失败且无法自愈
- `Stopping/Failed -> Idle`
  - `main` 已恢复不透明，视频窗口已隐藏或销毁

## Transparency Strategy

### Design rules

- 透明化必须拆成两个层次：
  - 原生窗口透明：`main` 窗口本体透明
  - Web 内容透明：网页根容器透明
- 两者的触发条件必须一致，并且都依赖首帧 ready。
- 禁止在“进入串流页面”时立即透明。

### Trigger contract

- 推荐由 `native_video` 在 present 成功后产出一个明确的“首帧已显示”事件，而不是由前端猜测。
- 透明化触发条件建议同时满足：
  - 视频窗口已存在且可见
  - 视频窗口已完成布局同步
  - 视频 presenter 已确认至少一帧真正显示
- 停流或失败时，恢复顺序必须固定：
  1. 撤销网页透明样式
  2. 撤销 `main` 原生透明
  3. 隐藏或销毁视频窗口

### Timeout and rollback

- 需要定义首帧等待超时：
  - 若超时，必须保证 `main` 不透明
  - 视频窗口可隐藏并记录失败原因
- 若透明态切换中途失败：
  - 回到 `main` 不透明
  - 不允许遗留“主窗口透明但无视频内容”的中间态

## Video Presentation Strategy

### Canonical display mode

- 首版 canonical 展示模式固定为：
  - `contain`
  - `center`
  - `black background`
- 含义：
  - 优先保持视频原始宽高比
  - 尽可能完整显示画面
  - 不拉伸填满目标窗口
  - 留白区域以黑底补足

### Why black background belongs to video window

- 一旦 `main` 透明，网页背景不能再承担视频留白的视觉兜底。
- 黑底必须属于视频窗口自身，这样无论 UI 是否透明，都不会露出桌面或其他内容。

### Future extensibility

- 后续可扩展的展示模式：
  - `cover`
  - `stretch`
  - 平台特定“整数缩放优先”
- 但这些都属于展示策略扩展，不能影响首版 canonical 默认值。

## Window Synchronization Strategy

### Single source of truth

- `main` 是双窗口布局与窗口生命周期的单一事实源。
- 视频窗口只跟随，不反向驱动 `main`。
- 这样可以避免：
  - 双向同步环路
  - 平台事件互相回写造成的抖动
  - 多 owner 冲突

### Synchronized behaviors

- 需要同步的行为：
  - 外框位置
  - 外框尺寸
  - 显示/隐藏
  - 全屏进入/退出
  - 关闭/恢复期状态
- 不需要同步的行为：
  - 焦点
  - 输入接收
  - 业务标题/业务路由

### Event handling rules

- `main` 窗口事件作为触发源。
- `native_video` 内部接收这些事件并做幂等同步。
- 同步必须有去抖/去重策略，避免高频 resize/move 导致无意义重排。
- 全屏切换要有单独路径处理，不能仅依赖普通 resize 事件推导。

## Focus and Input Policy

- 视频窗口不承担输入交互，因此设计原则如下：
  - 不主动获取焦点
  - 不承载键鼠/gamepad 事件 owner
  - 所有业务输入仍走 `main`
- 这意味着：
  - 即使视频窗口位于下层，用户感知上的“应用交互窗口”仍然是 `main`
  - 窗口一致性只需要覆盖展示行为，不需要复制交互能力
- 平台实现允许有差异，但目标是“视频窗口不成为主输入 owner”。

## Platform Considerations

### macOS

- 当前已有 `main` close -> hide 的应用语义，双窗口方案需要保证：
  - 当 `main` 隐藏时，视频窗口不得独立残留
  - Reopen `main` 时，不应错误恢复到悬空视频窗口状态
- 透明窗口、阴影和层级关系需要重点验证：
  - 透明后的 `main` 是否仍保留期望的交互可用性
  - 视频窗口黑底与原生 presenter 是否稳定

### Windows

- 需要确认视频窗口在任务栏、焦点和全屏切换上的用户感知是否符合预期。
- 如果平台上“不可聚焦窗口”存在限制，应退化为“不主动聚焦 + 每次恢复主焦点到 `main`”。

### Other platforms

- 不是当前主测试目标，但架构上不应写死平台假设。
- 如果某平台暂不支持透明叠加或原生 presenter，应允许退回单窗口渲染或禁用透明叠加，并在 capability/policy 上显式记录。

## Failure Handling

### Failure classes

- 视频窗口创建失败
- 视频窗口布局同步失败
- 首帧超时
- presenter 初始化失败
- 透明切换失败
- 停流回收失败

### Required recovery behavior

- 任一失败都不能破坏主应用可用性。
- 任一失败都必须收敛到：
  - `main` 不透明
  - 视频窗口隐藏/销毁
  - UI 可以继续展示错误态
- 错误信息需要进入 runtime trace / 结构化日志，避免仅靠肉眼复现。

## Observability Plan

- 需要新增或明确以下观测事件：
  - 视频窗口创建开始/成功/失败
  - 视频窗口与 `main` 的同步事件摘要
  - 首帧已 present
  - 透明态开始/完成/失败
  - 停流回收开始/完成/失败
  - 首帧等待超时
- 这些观测应进入现有 runtime trace 主线，而不是散落在前端 console。

## Implementation Plan

1. 在 `native_video` 内完成双窗口 owner 设计与内部职责拆分，恢复独立视频窗口主线。
2. 将视口目标从固定 `MainWindow` 改为支持独立 `native-video-*` 窗口，并完成窗口创建/复用/销毁策略。
3. 实现以 `main` 为 source of truth 的窗口同步：
   - 位置
   - 尺寸
   - 显示/隐藏
   - 全屏
4. 实现视频窗口默认 `contain + black background` 展示策略，并把黑底归属收口到视频窗口。
5. 在 `native_video` 与 `xbxengine`/`streaming` 之间补齐首帧 ready contract。
6. 将 `main` 原生透明态与 Web 内容透明态改成由 Rust 生命周期驱动，而不是页面进入即启用。
7. 补齐停流、失败、主窗口隐藏/恢复等异常回收路径。
8. 增加 runtime trace 与必要测试，并根据验证结果修正平台差异细节。

## Detailed Rollout Phases

### Phase A. 架构与 owner 收口

- 明确 `native_video` 内部状态对象：
  - 视频窗口句柄状态
  - 同步布局状态
  - overlay 透明态状态
  - presenter ready 状态
- 清理当前“固定路由到 `main`”的单窗口假设。

### Phase B. 双窗口创建与布局同步

- 创建视频窗口并完成与 `main` 的跟随同步。
- 确保窗口不会抢焦点、不会残留为独立业务窗口。

### Phase C. 首帧驱动透明叠加

- 以“实际 present 首帧成功”为触发条件切换透明态。
- 修复停流和失败回滚路径。

### Phase D. 展示质量与异常收口

- 固化 `contain + black background`。
- 补齐 resize/fullscreen/恢复期的布局稳定性。
- 处理平台差异、阴影、任务栏、隐藏恢复等边缘场景。

## Validation

- [ ] `main` 与视频窗口在进入串流后可同时出现，停流后同时收敛
- [ ] 视频窗口默认保比例显示，留白始终为黑底
- [ ] 进入串流页面但首帧未到前，`main` 仍保持不透明
- [ ] 首帧 ready 后，`main` 原生透明和网页透明样式同时生效
- [ ] 停流、断流、首帧超时后，`main` 恢复不透明且视频窗口不残留
- [ ] 拖动、缩放、全屏切换时双窗口保持一致
- [ ] 视频窗口不会成为输入交互 owner，不会抢业务焦点
- [ ] macOS close/hide/reopen 语义下不会遗留悬空视频窗口
- [ ] Windows 下任务栏、焦点与全屏切换行为符合预期
- [ ] runtime trace 能区分窗口创建失败、首帧超时、透明切换失败等关键问题

## Risks

- 双窗口同步会引入新的时序复杂度，尤其是 resize/fullscreen 期间的抖动与错位。
- 透明态切换如果没有严格绑定首帧 present，容易回到“露底/黑屏/残留透明态”的老问题。
- 各平台对于透明窗口、阴影、focusable 和任务栏语义的支持不完全一致，需要平台特化策略。
- 如果 `native_video` 内部职责拆分不清晰，可能只是把“单窗口混杂”替换成“模块内新一轮混杂”。
- 如果前端仍然保留分散控制透明态的入口，会破坏 Rust owner 边界。

## Alternatives Considered

### Alternative A: 保持当前单窗口主线

- 优点：
  - 实现复杂度更低
  - 不需要双窗口同步
- 缺点：
  - 视频承载与 UI 壳层继续耦合
  - 首帧透明切换、黑底保比例与后续平台打磨会继续复杂化
- 结论：
  - 不满足本次目标，放弃。

### Alternative B: 新增独立 `window_coordinator` 模块

- 优点：
  - 窗口同步逻辑看起来更集中
- 缺点：
  - 会新增新的顶层 owner，与 `native_video` 现有职责发生交叉
  - 用户已明确要求不要走这条路线
- 结论：
  - 不采用。

### Alternative C: 前端驱动双窗口原生状态

- 优点：
  - 前端开发入口直观
- 缺点：
  - 双窗口布局、首帧、透明、停流回滚都需要跨层推断
  - 难以保证平台一致性
- 结论：
  - 不采用。

## Rollback Strategy

- 在实现期间保留可切换的窗口策略开关，允许在问题未收敛时退回当前单窗口主线。
- 回滚必须保证：
  - `main` 仍可独立承载串流 UI
  - `native_video-*` 不会残留
  - 前端透明样式不会在回滚后被误触发
- 回滚后的诊断信息仍需保留，以支持继续迭代双窗口方案。

## Open Questions

- 视频窗口 label 是否固定为单实例，还是按会话生成动态 label 更合适？
- 首帧 ready 是否需要“已 present 一帧”之外的稳定门限，例如最短持续时长或最少帧数？
- 全屏切换时应由 `main` 先切，还是先布局视频窗口再切 `main`，哪种感知更稳定？
- macOS / Windows 是否都需要对视频窗口禁用任务栏入口，还是只要求不暴露为独立业务窗口？
- 对于非主目标平台，是否明确退化为“仍创建视频窗口但不透明叠加”，还是直接回退单窗口？

## Progress

- [x] Step 1: 已明确目标、约束和不新增 `window_coordinator` 的前提。
- [x] Step 2: 已完成双窗口拓扑、状态机、透明时序和保比例黑底策略设计。
- [ ] Step 3: 待按 RFC 实施 `native_video` 内部重构与双窗口恢复。
- [ ] Step 4: 待补窗口同步、首帧 ready contract 与透明态切换主线。
- [ ] Step 5: 待补平台验证、runtime trace 和最终交付文档。

## Execution Notes

- Date: 2026-03-24 | Status: planned
- Update: 新增本 RFC，明确 canonical 串流窗口策略从当前单窗口宿主切换为“独立视频窗口 + 主 WebView 窗口”的双窗口架构。
- Decision: 双窗口同步与首帧透明编排能力继续收口在 `src-tauri/src/mods/native_video/*` 内，不新增独立 `window_coordinator` 顶层模块。
- Decision: 首版 canonical 展示模式固定为 `contain + black background`，只有在首帧真实显示后才允许 `main` 透明化。
- Risk/Blocker: 当前主线仍是单窗口宿主，实现时需要严格控制迁移阶段与回滚开关，避免把应用置于“主窗口透明但视频窗口未 ready”的中间态。
