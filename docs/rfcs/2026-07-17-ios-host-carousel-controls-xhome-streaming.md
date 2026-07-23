# iOS 主机轮播控制与 xHome 串流 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 已完成
- Current State: completed
- Owner: agent
- Last Updated: 2026-07-17

## Background

- iOS 已具备 SmartGlass 主机列表、主机详情页和 cloud-only libwebrtc 串流运行时。
- 当前主机 Tab 位于游戏库之后，使用 Xbox 标志，主机采用纵向列表，页面缺少围绕当前主机的开机、关机与 xHome 串流操作。
- 桌面端已提供 Series X / Series S 主机图片、Console API 电源控制和 home target 会话协议，可沿现有 Rust 主线复用。

## Goal

- 主机 Tab 成为首位与默认页，使用小电脑图标。
- 主机页上部使用与成就页一致的圆弧轮播卡片，卡片背景复用桌面端主机图片。
- 轮播下方提供开机、关机、串流三个操作，全部在点击时解析当前选中主机。
- 主机卡片保留轮播选择职责，页面保持单层操作结构。
- 开关机通过 Xbox Console API 执行；串流通过现有 iOS libwebrtc 数据面和新增 home target 控制面启动。

## Scope

- In scope:
  - `AppRootView` Tab 顺序、默认选择与图标。
  - 提取共享圆弧轮播组件，成就页和主机页共同消费。
  - 移除旧主机详情页与卡片导航入口。
  - 桌面 Series X / Series S 图片进入 iOS Asset Catalog。
  - `xbox-ios-bridge` Console API 电源命令与 UniFFI 合同。
  - home streaming access、target-aware stream session、Swift Streaming Runtime target 合同。
  - 主机页当前选择、操作状态、错误反馈与测试。
- Out of scope:
  - 手柄、触控、麦克风与 xHome 特有高级设置。
  - 模拟器和真实账号运行验收，由用户执行。

## Plan

1. 提取成就圆弧轮播为共享组件并接回成就页。
2. 导入桌面主机图片，重做主机页轮播与当前选择操作栏。
3. 增加 Console API 开关机桥接与 Swift 数据状态。
4. 增加 home access 与 target-aware iOS streaming control，串流按钮启动当前主机。
5. 生成 bindings，执行 Rust、Swift、PBX、Asset Catalog 与差异验证。

## Validation

- [x] `cargo fmt -p xbox-ios-bridge -- --check`
- [x] `cargo test -p xbox-ios-bridge`
- [x] Swift strict-concurrency typecheck 或 Device build
- [x] PBX / Asset Catalog / `git diff --check`
- [x] 源码门禁证明 Tab 顺序、图标、轮播和三个按钮均绑定当前选择

## Risks

- xHome 会话存在主机唤醒和服务注册延迟；串流入口应在待机状态先执行唤醒，并沿用现有 session pending/retry 语义。
- 当前工作区包含同日 iOS streaming 修复，修改共享合同与生成 bindings 时必须保留其变更。
- 桌面图片为单倍方形源图，iOS 卡片需要适配裁切与深色背景，避免透明或棋盘格边缘破坏可读性。

## Progress

- [x] Step 1: 已完成现状、桌面资产、成就轮播与 cloud-only streaming 边界检查。
- [x] Step 2: 已完成主机轮播、桌面资产和当前选择操作栏。
- [x] Step 3: 已完成 Console API 电源控制与 home streaming target。
- [x] Step 4: 已移除旧主机详情页并完成 Device build、Rust、PBX、资源和差异验证。

## Execution Notes

- Date: 2026-07-17 | Status: in-progress
- Update: 需求从局部 UI 调整改判为跨 Rust/Swift streaming contract 任务，建立独立 RFC 后执行。
- Decision: 串流按钮使用真实 home target，不复用 cloud title ID 入口，不保留静态占位行为。
- Risk/Blocker: 模拟器和真实主机行为由用户验收，代码侧需完成可编译与合同测试。
- Date: 2026-07-17 | Status: completed
- Update: 最终设计使用单层主机页，主机卡片仅更新当前选择，旧主机详情页及 Xcode 工程引用已移除。
- Validation: `cargo fmt -p xbox-ios-bridge -- --check`、`cargo test -p xbox-ios-bridge` 26 项、iPhoneOS Device build、PBX/JSON/图片一致性、bindings API、源码门禁与全量差异检查通过。
