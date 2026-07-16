# iOS Rust Auth Bridge 与原生个人中心 Report

## Summary

- Related RFC: [`docs/rfcs/2026-07-13-ios-rust-auth-profile-bridge.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-07-13-ios-rust-auth-profile-bridge.md)
- iOS 登录与个人中心初版已按 Rust 协议层、Swift 系统能力层边界完成。

## Delivered

- 新增 `xbox-ios-bridge` UniFFI crate，导出登录开始、登录完成、Token 刷新和当前用户 Profile。
- 新增 `ASWebAuthenticationSession` 登录、Keychain 会话存储、启动恢复和前台资料刷新。
- 新增登录态、资料态、Gamerscore/XUID 展示、下拉刷新和退出登录。
- Xcode 自动构建 Device/Simulator 对应 Rust 静态库并链接生成 Swift App。

## Changes

- `xbox-auth-flow` 增加 `include_streaming_tokens`，iOS 登录只生成 Web Token，Tauri 保持完整 xHome/xCloud Token 行为。
- OAuth callback 同时校验 scheme、host 和 OAuth state；Profile 解析集中在 Rust。
- refresh token、Web Token 和私有 JWK seed 使用 `WhenUnlockedThisDeviceOnly` Keychain 存储。
- AuthStore 使用可注入协议隔离 Xbox client、网页登录和会话存储，支持无系统副作用的状态测试。

## Validation

- `cargo fmt --check`
- `cargo test -p xbox-ios-bridge`：3 passed。
- `cargo test -p xbox-auth-flow`：通过。
- `cargo check -p xbxrc`：通过，桌面调用点兼容。
- `cargo build -p xbox-ios-bridge --target aarch64-apple-ios`：通过。
- `cargo build -p xbox-ios-bridge --target aarch64-apple-ios-sim`：通过。
- iOS 26.1 Device SDK 代码构建：App Swift/UniFFI/Rust 编译链接成功。
- iOS 26.1 Simulator SDK arm64 代码构建：App Swift/UniFFI/Rust 编译链接成功。
- `XBXRCTests` Device SDK `build-for-testing`：通过，覆盖会话编解码、无会话恢复、刷新恢复和交互登录持久化。
- `plutil -lint`：Info.plist 与 project.pbxproj 通过。
- `git diff --check`：通过。

## Risks

- 当前执行沙箱无法连接 CoreSimulatorService，完整 Asset Catalog 构建与 XCTest 运行需要在正常 Xcode 会话复验。
- 真实 Xbox 登录和 Profile 请求需要用户账户、联网环境与交互授权。
- Xbox 服务接口变化继续由 Rust crate 集中吸收。

## Follow-up

- 在 Xcode 模拟器或真机执行完整 XCTest 与真实账号登录验收。
- 在成就、游玩时长和游戏列表阶段继续扩展同一个 `xbox-ios-bridge`，保持 Swift 侧只消费业务 record。
- 串流入口增加按需生成 xHome/xCloud Token 的 Rust bridge 接口。
