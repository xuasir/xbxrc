# main / rust 发行隔离验证

## Browser 稳定线

- 源码与构建提交：`0c89d1e`（main），包含 P0 基线、公共协议/构建身份、发布更新身份门禁和完整原生裁剪。
- 前端：455 项；公共 Rust：147 项；Tauri：44 项；lint/typecheck/生产构建、生命周期测试通过。
- 发布：8 项版型/分支/tag/版本/identity/hash负例门禁，两 edition 的 macOS/Windows模拟 `bundle -> manifest -> latest.json` 通过。CI YAML解析通过；Browser Cargo normal 依赖树裁剪门禁通过。
- 实际构建：CI模式 `pnpm tauri build --bundles app,dmg --config '{"bundle":{"createUpdaterArtifacts":false}}'`，CARGO_TARGET_DIR 使用绝对目录 `src-tauri/target/browser`。
- 安装身份 `com.xuasir.xbxrc`，arm64，最低 macOS 26.0；仅1个非系统Mach-O（应用自身），签名/依赖闭包/原生运行库及跨平台资源裁剪通过。
- app 文件合计 18,928,324 bytes；DMG 9,006,064 bytes。产物位于 `src-tauri/target/browser/release/bundle/{macos,dmg}`。
- 移动 app 到 `/private/tmp/xbxrc-p1-browser-relocated.app` 后再次通过签名与依赖闭包检查。
- 本地使用 ad-hoc 签名；此轮生成本地安装包。更新包签名与真实发布由 CI secrets 执行。

## Native WIP

正在适配并验证 native-media / native-gamepad 特性、资源、独立安装与更新身份，证据待追加。

## 证据边界

- GitHub 只读核对：远端默认分支已经是 main；rust 与 release/native-wip 尚未存在，release/test 保留旧提交。此轮先完成可审查代码与产物。
- 自动实机运行保持暂停。安装、登录、手柄、xHome/xCloud、退出回收、更新及长测由用户手动验收。
- 当前本地 macOS arm64 构建取证；Windows与macOS x86_64实际CI/安装证据分别记录。
