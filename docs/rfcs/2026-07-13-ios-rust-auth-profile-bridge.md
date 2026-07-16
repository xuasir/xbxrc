# iOS Rust Auth Bridge 与原生个人中心 RFC

## Status

- Completion: 已完成
- Current State: completed
- Owner: agent
- Last Updated: 2026-07-13

## Background

原生 iOS 应用需要登录 Xbox 账户并展示个人资料。仓库已有可工作的 `xbox-auth-flow` 与 `xbox-webapi`，协议实现包含 OAuth、Sisu、XSTS、JWK 签名和 Token 刷新；iOS 同时需要使用系统网页登录、Keychain 与 SwiftUI 生命周期。

## Goal

- 复用 Rust Xbox Auth/Profile 协议实现，保持桌面与 iOS 协议行为一致。
- 使用 `ASWebAuthenticationSession` 完成原生交互登录。
- 在 Keychain 中持久化可恢复登录所需的最小凭据。
- 提供登录、恢复、刷新资料和退出登录完整个人中心初版。

## Scope

- In scope:
  - `xbox-auth-flow` 增加按调用方控制的 streaming token 生成策略。
  - 新增 `xbox-ios-bridge` UniFFI crate 与 Device/Simulator 静态库构建。
  - Swift 登录状态机、系统网页登录、Keychain 会话存储和个人中心。
  - OAuth callback destination 校验、Profile/XUID 解析和状态测试。
- Out of scope:
  - 成就、游玩时长和游戏列表 API。
  - xHome/xCloud streaming token 的按需获取入口。
  - libwebrtc RTC 数据面和原生视频渲染。
  - App Store 签名、分发与线上账号端到端验收。

## Architecture

```text
SwiftUI ProfileView
       |
    AuthStore
       |---------------- ASWebAuthenticationSession
       |---------------- KeychainSessionStore
       |
  XboxAuthClient
       |
  UniFFI generated binding
       |
 xbox-ios-bridge
       |---------------- xbox-auth-flow
       `---------------- xbox-webapi ProfileApi
```

Rust 导出 `start_login`、`complete_login`、`refresh_login` 和 `fetch_profile`。Swift 只传递 UniFFI record 与序列化的 pending/seed/Web Token，不解析或重写 Xbox 协议。

## Security Boundary

- OAuth pending/state 只驻留当前交互登录流程内存。
- refresh token、Web Token 与私有 JWK seed 存入 Keychain。
- Keychain accessibility 固定为 `WhenUnlockedThisDeviceOnly`，凭据不经 iCloud 同步。
- Rust 同时校验 callback scheme `ms-xal-000000004c20a908` 与 host `auth`。
- iOS 登录设置 `include_streaming_tokens=false`，只生成 Profile/API 所需 Web Token。
- Tauri 登录设置 `include_streaming_tokens=true`，保持桌面端 xHome/xCloud 行为。

## Plan

1. 扩展共享 Auth Flow 并建立 iOS UniFFI bridge。
2. 接入系统网页登录、Keychain 与原生登录状态机。
3. 完成个人中心资料、刷新和退出界面。
4. 接入 Xcode Rust build phase，补齐单元测试和双平台构建验证。

## Validation

- [x] `cargo fmt`
- [x] `cargo test -p xbox-ios-bridge`
- [x] `cargo test -p xbox-auth-flow`
- [x] `cargo build -p xbox-ios-bridge --target aarch64-apple-ios`
- [x] `cargo build -p xbox-ios-bridge --target aarch64-apple-ios-sim`
- [x] iOS 26.1 Device SDK 下完整 Swift/UniFFI/Rust 链接成功。
- [x] `XBXRCTests` target 在 iOS 26.1 Device SDK 下编译成功。

当前自动化环境无法连接 CoreSimulatorService，因此 XCTest 运行与包含 Asset Catalog 的完整构建保留到正常 Xcode 会话执行；真实 Xbox 账户登录属于交互式在线验收。

## Risks

- Xbox 私有接口可能随服务端策略变化，需要通过 Rust crate 集中适配。
- 当前 UniFFI 传递 JSON 字符串承载内部 Token 对象，后续接口扩展应继续保持 bridge record 粗粒度。
- 真实账号验收需要用户交互和在线服务；自动测试覆盖状态机与纯解析逻辑。

## Progress

- [x] Step 1: Rust bridge 与 Web Token-only 登录路径已实现。
- [x] Step 2: ASWebAuthenticationSession、Keychain 和 AuthStore 已实现。
- [x] Step 3: ProfileView 登录态、资料态、刷新和退出已实现。
- [x] Step 4: 双平台构建、测试 target 编译、差异检查与交付报告已完成。

## Execution Notes

- Date: 2026-07-13 | Status: completed
- Update: 完成 Rust/Swift/Xcode 主链，Device/Simulator SDK 代码构建和 XCTest target 编译通过。
- Decision: iOS 登录阶段只生成 Web Token，串流 Token 延迟到串流入口按需获取。
- Risk/Blocker: 当前执行沙箱无法连接 CoreSimulatorService，Asset Catalog 完整构建与 XCTest 运行保留为外部环境验收。
