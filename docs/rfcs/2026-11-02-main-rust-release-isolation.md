# main / rust 开发与独立发行改造

## Status

- Current State: validation-pending
- Owner: 当前窗口直接执行，按用户要求使用中文且保持单窗口实施
- Authorization: 用户设置目标“按这个方案，把分支、发布、代码都做好改造”
- Date: 2026-11-02
- Design: [已确定方案](../isu/2026-11-02-stable-native-wip-release-split.md)

## Goal / Scope

`main` 只交付 Browser Gamepad API、浏览器 WebRTC 与 WebView 音视频及公共功能；`rust` 保存和开发原生媒体/手柄 WIP。`release/test` 由 main 的已验证提交发行 Browser beta，`release/native-wip` 由 rust 的已验证提交发行独立原生预览；正式稳定 tag 来源限定 main。

保存当前 P0 工作树为两条线的共同基线。原生代码、workspace 成员、SDL3/wgpu/FFmpeg/音频与 SDK 归 rust；main 保留 Tauri/Rust 认证、Xbox API、会话/信令、系统、配置、更新和日志。公共领域 DTO 按会话/信令职责整理，公共导航及现有浏览器功能继续保留。FFmpeg 和应用保持 macOS 26.0。

## Implementation

1. 固定基线并建立 rust 分支，保留旧功能分支历史；两条开发线分别提交可验证结果。
2. main 移除原生模块、AppState/service/RPC/trace 装配和前端原生入口/设置；清理 workspace 与依赖、SDK、包装脚本及发行资源。抽离公共构建身份与轻量领域合同。
3. rust 保留原生实现与两个独立后端开关，提供独立应用标识/名称、安装与配置日志身份、更新源及 WIP 版本；支持原生模块分别验证。
4. release 流水线核验开发来源、edition、正式 SemVer/预览版本和产物清单；CI/cache 按开发线与架构分开。稳定构建直接调用 Tauri，原生构建使用已固定 SDK。
5. 构建实际 macOS Browser/WIP 安装包，验证依赖闭包、签名、资源裁剪与更新清单。Windows CI 与安装产物按各自平台门禁验证。

## Verification

- [x] main 源码/workspace/Cargo 依赖树只含稳定能力，构建独立于 FFmpeg/SDL SDK
- [x] 前端 lint:fix、类型、测试、生产构建；Rust fmt、公共与 Tauri 单元测试
- [x] rust 原生编译/测试与运行库门禁，能力/开关/组合合同
- [x] 两种 app 身份、资源、更新 endpoint 和 manifest 对应，跨 edition 发布被门禁拒绝
- [x] 本地开发及 release 分支对应到验证源码，流水线语法、发行门禁与模拟产物验证
- [x] 实际本地 macOS arm64 安装包与可迁移依赖检查，记录尺寸和构建身份
- [ ] 远端开发线同步与 Windows/macOS CI 的实际安装产物取证
- [ ] 实机与长测按用户手动验收，独立记录平台/CI 证据范围

## Boundaries / Risks

当前任务改造分支、源码和发行机制。真实发版由版本/tag 或 release 分支推送触发；本轮发布准备先以可审查提交和产物取证。自动实机启动和串流操作按用户已确认约定保持暂停。

初次裁剪同步到 rust 时保留原生模块并适配公共边界；后续公共修复优先进入 main，再同步至 rust。平台与长测缺证项目继续保持待验状态。

## Progress

- 已保存 P0 基线 `37f632a`，rust 从此建立。公共协议与构建身份已拆分 `9abf17d`；公共发行/更新身份门禁 `10b771d` 已验证。
- main 原生源码、依赖、SDK、前端入口及设置完成裁剪。直接 Tauri 构建和 main/release/test/stable 的来源门禁已接入独立 CI/cache；共同 manifest、SHA/架构/版本/跨 edition 检查及 updater 运行时检查已落地。
- main：公共 Rust 147 项、Tauri 44 项、前端 455 项、lint/typecheck/生产构建、生命周期测试及 8 项发行门禁/两 edition 模拟通过；Browser Cargo 依赖树门禁通过。
- rust：原生媒体与手柄分别由 native-media / native-gamepad 控制，native-wip 聚合；四种特性组合的编译和依赖门禁通过。前端473项、引擎1277项（24 ignored）、Tauri212项、关闭原生Tauri48项、手柄core36项和Mac运行库探针8项通过。
- 两条线默认使用 target/browser / target/native-wip，与 CI 的 edition 目录一致。Browser 默认目录重建与 Native WIP 实际 app/DMG 已完成，签名、独立身份、当前架构资源与可迁移依赖闭包通过。应用与 FFmpeg 的 macOS 最低要求保持26.0。
- 本地源码及发行快照进入待验收状态；详细构建身份、尺寸和证据边界见[发行隔离验证](../../dev-docs/validation/2026-11-02-desktop-edition-isolation.md)。远端开发线同步与平台CI为下一步；实机和长测保持用户手动验收。
