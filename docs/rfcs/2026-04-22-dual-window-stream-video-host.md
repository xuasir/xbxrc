# RFC: Dual-Window Stream Video Host

Completion: 未完成
State: in-progress
Owner: Codex
Created: 2026-04-22

## Background

当前串流页面采用单窗口融合方案：

- 主窗口 `main` 同时承载透明 Web UI 与 native video presenter。
- `stream-page-video` viewport 固定解析到主窗口宿主。
- Windows `native_video` presenter 通过 `run_on_main_thread` 在主窗口线程执行渲染 tick。

这一路径在视频链路异常时容易把“等待画面”和“窗口响应性”绑在一起。用户当前希望切换到参考 macOS 承载思路的双窗口方案：保留主窗口上的串流 UI，改由独立视频窗口承载 native video presenter。

## Goals

1. 将串流视频 presenter 从主窗口 `main` 迁移到独立视频窗口。
2. 保留现有串流页 Web UI、菜单、诊断面板、输入策略与主窗口路由。
3. 让 viewport target 根据串流场景解析到独立视频窗口，而不是硬编码主窗口。
4. 统一主窗口与视频窗口的生命周期、可见性与全屏行为，避免遗留孤儿窗口。

## Non-Goals

1. 不重写现有串流页面 UI。
2. 不修改 Rust-owned / browser runtime 的整体协议边界。
3. 不在本次改造中重做所有视频 presenter 实现。
4. 不引入 Electron、第二套 runtime 或新的前端技术栈。

## Scope

### In Scope

- `src-tauri/src/mods/native_video/*`
- `src-tauri/src/mods/app_state/*`
- `src-tauri/src/shell/*` 中与窗口初始化/关闭相关的部分
- `src/router/index.ts`
- `src/pages/*` 与 `src/styles/*` 中为双窗口视频宿主提供的最小前端支撑
- `docs/project-task.md`

### Out of Scope

- 串流恢复策略本身
- 视频编解码与 RTC 协商逻辑
- 非串流页面窗口行为

## Proposed Direction

### 1. Introduce a dedicated native video window

- 新增固定 label 的独立视频窗口，例如 `native-video-stream`。
- 该窗口仅作为 native video 宿主，页面内容保持最小化，避免重复渲染主应用 UI。
- 主窗口继续承载透明串流 UI 与交互覆盖层。

### 2. Route stream viewport away from the main window

- `stream-page-video` viewport 不再固定映射到 `main`。
- 在串流双窗口模式启用时，viewport target 应解析到独立视频窗口。
- presenter / host view / wgpu render loop 应绑定到该独立窗口。

### 3. Keep window lifecycle coordinated

- 串流开始时确保视频窗口存在并显示。
- 串流结束、页面退出、应用退出时关闭或隐藏视频窗口。
- 主窗口全屏切换时，同步视频窗口全屏状态与几何信息。

### 4. Keep current stream UI flow

- 主窗口上的 `Stream.vue` 保持现有 overlay/diagnostics/action sheet 主体结构。
- 前端只补充独立视频宿主页或最小 route，不重写串流控制器。

## Implementation Plan

1. 在 `native_video` 中抽取视频窗口 label/target 解析与窗口确保逻辑。
2. 新增独立视频窗口创建与关闭流程。
3. 将 `stream-page-video` viewport 目标切换到视频窗口。
4. 让全屏/退出/关闭流程同步操作主窗口与视频窗口。
5. 为独立视频窗口新增最小前端 route / page 支撑。
6. 运行构建或最小验证，确认窗口创建、attach viewport、关闭链路不回归。

## Validation

1. TypeScript 构建通过。
2. Rust / Tauri 构建通过。
3. 串流页启动后主窗口保留透明 UI，native video 绑定独立窗口。
4. 退出串流后不会残留独立视频窗口。
5. 全屏切换不会只作用于主窗口而遗漏视频窗口。

## Risks

1. 双窗口几何与焦点管理可能引入新的平台差异。
2. 若视频窗口承载页面不够轻量，可能出现多余 WebView 开销。
3. 现有 `main` 窗口假设较多，可能需要同步修正全屏与关闭逻辑。

## Progress

- [x] 完成现有单窗口宿主路径与日志行为分析。
- [x] 建立独立视频窗口与 viewport target 路由。
- [x] 接通前端最小宿主页与生命周期联动。
- [ ] 完成验证并回填结果。
