# 串流运行条与诊断指标改造 RFC

## Status

- Completion: 已完成
- Current State: completed
- Owner: Codex（单窗口）
- Last Updated: 2026-11-03
- Authorization: 用户已授权按已确认方案实施，并要求应用 `xbx-design`。

## Background

[设计方案](../isu/2026-11-03-stream-metrics-low-interference-design.md)已确定顶部透明运行条与直接开启的诊断详情。当前体验面板有 12 项，浏览器诊断面板有 45 行，右上区域重叠且内部字段持续占用画面。

## Goal

- 运行条固定为源分辨率、呈现观察 FPS、视频 Mbps、RTT、5 秒丢包率，各密度保留五项，直接吸顶；状态单一表达，有效零值与过期采样独立表达。
- 诊断指标直接进入网络、显示、恢复、会话四组；面板保持右侧三分之一宽，字段分组、能力说明、当前会话分组记忆与诊断复制。
- 应用 XBX token、自动密度、三层信息层级和 Focusable 焦点环；诊断开关与关闭完整衔接既有输入路由。

## Scope

修改 Vue 面板、展示投影、页面临时 UI 状态、文案、语义样式 token 和旧展示设置入口。复用现有采样、统计窗口、runtime 和传输合同；保留旧 `performance_style` 配置读取兼容，页面统一采用运行条。

## Plan

1. 收敛运行投影与诊断字段分组，保持观测来源和窗口可追溯。
2. 实施 XBX 运行条、诊断详情与菜单 / 输入捕获切换。
3. 验证边界数值、状态、分组和输入归属；完成 lint、类型检查、构建及界面密度 / 主题检查。

## Validation

- [x] 5 项运行条、实时源尺寸、零值 / 缺值 / 过期值与单一状态测试
- [x] 4 组诊断、固定不可用行移至能力说明、迁移字段及复制字段边界
- [x] 诊断开关直接进入详情；关闭 / 返回 / 会话终止释放捕获
- [x] XBX tokens、FocusScope / Focusable、焦点环、standard / compact / narrow 与 light / dark
- [x] `pnpm lint:fix`、定向测试、`pnpm typecheck`、`pnpm build`、`git diff --check`

全量测试 48 文件、493 项通过。真实 Vue 组件验证覆盖 736px standard、compact 浅色主题与 320px narrow；运行条单行、详情适配、方向键焦点与长列表滚动、四组切换、复制反馈、Escape / 关闭按钮、重开分组记忆和过期状态均通过。Back 复用导航引擎派发 Escape 的既有合同。页面联动通过源码审核，菜单 → 详情连续捕获通过回归测试；真实串流与手柄实机验收由用户执行。

## Risks

浏览器 FPS 是视频推进或渲染提交观测；GPU / 实际 scanout 能力在详情明确说明。真实串流与长测按仓库规则由用户手动验收；本任务完成展示代码与静态 / 交互验证。

## Progress

- [x] 明确已授权的方案、实现边界和既有采样依据
- [x] 运行条与分组投影
- [x] 面板与输入路由
- [x] 验证与收口报告

收口结果见 [Report](../reports/2026-11-03-stream-metrics-low-interference.md)。轮询生命周期增加读取代次保护，已停用轮询的迟到成功 / 失败结果由原生命周期吸收。

2026-11-03 用户实机反馈后的布局修正：移除 compact / narrow 隐藏字段及 narrow 面板满宽规则。运行条顶距 0、底色 12% 实色；诊断宽度固定为容器三分之一、底色 55% 实色。真实 Vue 在 800 / 1024 / 1280px 下测得诊断宽度 266.66 / 341.33 / 426.66px，五项可见且运行条顶距 0。布局修正的 lint、typecheck、build 与 diff check 通过。
