---
title: Gamepad battery UI and DTO extension
date: 2026-04-07
status: Draft
authors:
  - codex
---

## Status

- Completion: 未完成
- Current State: planned
- Owner: TBD
- Last Updated: 2026-04-09

## 背景

当前 `GamepadDeviceDto` 仅通过 `capabilities.battery: boolean` / `effectiveCapabilities.battery: boolean` 暗示“是否具备电量相关能力”，但**没有暴露具体电量百分比或充电状态**。这限制了前端在 Xbox 风格 UI 中展示更贴近主机体验的电量信息（例如手柄卡片右上角电池图标）。

## 目标

- 在不破坏既有调用者的前提下，为 `GamepadDeviceDto` 增加**可选的电量信息字段**。
- 让前端可以在有数据时展示电池图标和简单状态文案；无数据时保持当前行为不变。

## 提案

### DTO 扩展（向后兼容）

在 [`src/shared/gamepad/contract.ts`](/Users/guo.xu/Documents/code/games/xbxrc/src/shared/gamepad/contract.ts) 中为 `GamepadDeviceDto` 增加可选字段：

- `batteryLevel?: number | null` — 电量百分比，`0–100`。
- `batteryState?: 'charging' | 'discharging' | 'full' | 'not-present' | 'unknown'`。

Rust 侧各平台桥接层负责做最小映射，例如：

- Windows / XInput: 映射自 `XINPUT_BATTERY_INFORMATION`。
- macOS / GameController: 使用 `GCControllerBattery` 或系统电量 API。
- 其他平台若无可靠信号：保持 `batteryLevel = null`、`batteryState = 'unknown'`。

### 前端使用约定

- 仅当 `batteryLevel` 为 `number` 且在合理区间时展示电池图标与百分比，否则回退为简单的「电池信息」能力标签（已在 `GamepadProfileCard` 中存在）。
- `batteryState` 仅用于图标样式和辅助文案（例如充电闪电标记），不参与逻辑判断。

## 兼容性与迁移

- 所有新增字段为可选，旧版 Rust 宿主保持不填即可；前端在访问时需显式判空。
- DTO 版本不做硬分支，依赖运行时判空保证兼容老日志与旧快照。

## 后续工作

- Rust 侧实现具体采集与快照填充。
- 前端在 `GamepadProfileCard` 和 Setting 页的 gamepad 区追加电池图标展示逻辑（在本 RFC 范围内描述，不在当前实现批次强制完成）。+
