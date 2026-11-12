# 官方震动时序与 Share 试验撤销 RFC

## Status

- Completion: 完成（震动时序与 Share 试验撤销）
- Current State: done
- Owner: 主窗口，单窗口执行
- Last Updated: 2026-11-12

## Background

- [前序核对报告](../reports/2026-11-12-rumble-compatibility-and-share-protocol.md)已修复 12/13 字节震动包兼容，并使用官方公开源码核实 delay、repeat、持续时间和 Share 预留字段。
- 用户明确授权“震动时序对齐官方，但是强度依旧可配置”及“先临时做一个键盘的 share 发送，我本机验证一下”。本轮直接执行该范围。
- 用户实机确认远端对 Share 输入无响应，明确要求删除该输入；最终交付保留震动改动并撤销 Share 试验接入。

## Goal

- 四路游戏震动按官方 delay、duration、repeat 调度，保留现有逐路强度配置。
- 移除临时 F9 Share 入口及其专用协议、状态、提示和 trace，恢复既有串流输入路径。

## Scope

- 公共 InputController 管理游戏震动调度和取消，Browser backend 提交原始持续时间。
- 移除仅为 Share 试验增加的 Browser 输入模型、Rust 信令/RPC 字段及前端状态链。
- 公共输入归属、原有输入发送与四路强度配置继续沿现有实现执行。
- Share 官方协议样本保留为历史证据；物理电机触觉由用户本机验收。

## Plan

1. 解析 delay/repeat，按首次 delay、相邻 duration+delay、追加 repeat 次调度；取消等待中的 Promise 与定时器。
2. 根据实机结论撤销 Share 专用快捷键、输入模型、协商字段、状态和测试。
3. 回归原有输入路径与震动，核对源码和构建产物，保留历史验证记录。

## Validation

- [x] 12/13 字节与旧 10-bit 格式解析、停止包、弱短脉冲回归。
- [x] 200/100/2 效果在 100/400/700ms 播放，5000ms 原值提交，四路配置保留。
- [x] 新效果、停止、退出、菜单接管、输入来源切换和断连取消待播效果。
- [x] Share 专用快捷键、状态、协议编码、通道、协商、提示和 trace 已撤销。
- [x] 原有输入路径回归及 Share 专用源码/构建标识清理通过。
- [x] pnpm lint:fix、typecheck、build；Rust 修改执行 cargo fmt 及相关测试。

## Risks

- 浏览器执行时钟和控制器硬件会影响物理触觉，自动验证覆盖提交时序和原始参数。
- Share 本机试验结论已记录为远端无响应；官方字段样本作为历史证据保留。
- 工作区已有暂存修改，按文件局部补丁保留现有变更与暂存状态。

## Progress

- [x] Step 1: 核实用户授权及前序协议证据。
- [x] Step 2: 震动实现与回归。
- [x] Step 3: Share 试验及按实机结论撤销。
- [x] Step 4: 验证、报告与本机操作说明。

## Execution Notes

- Date: 2026-11-12 | Status: done
- Decision: 按用户实机结论撤销 Share 试验接入，保留官方震动时序与四路强度配置。
- 首次实现验证: 774 项前端、Rust 协议234/trace5及生产成本门2通过，保留历史证据。
- Update: 用户实机确认远端对 Share 输入无响应，明确要求删除；撤销 F9、Share 状态、专用输入模型与协商字段，保留震动时序及强度配置。
- 最终验证: 撤销后 730 项前端、Rust 协议232/trace4、生产成本门2及 lint/typecheck/build/fmt/workspace check 通过；官方实际方法回放五组一致。
- Report: [交付报告](../reports/2026-11-12-official-rumble-timing-and-keyboard-share.md)。
- Evidence: [验证记录](../../dev-docs/validation/2026-11-12-official-rumble-share/README.md)。
