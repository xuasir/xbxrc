# 桌面开发与发行

| 开发线 | 媒体 / 输入 | 发行入口 | 安装身份 / 更新源 |
| --- | --- | --- | --- |
| main | Browser WebRTC、WebView 音视频、Browser Gamepad API | release/test → beta；vX.Y.Z → stable | com.xuasir.xbxrc；stable / beta |
| rust | Browser 对照与 Rust 媒体 / 手柄 WIP | release/native-wip → Native WIP prerelease | com.xuasir.xbxrc.native-wip；native-wip |

原生源码、依赖、SDK、设置和诊断随 rust 开发线维护。公共协议、认证、Xbox API、会话/信令、UI与几何导航在 main 修改后同步到 rust。稳定线裁剪提交与原生线装配提交形成一次性分叉边界，后续公共修复按提交同步。

main 使用 `pnpm tauri build`，rust 使用带 FFmpeg SDK 的原生入口。Cargo本地默认目录分别为 `target/browser` / `target/native-wip`，CI使用对应绝对目录；cache按edition和架构隔离；应用和 FFmpeg 保持 macOS 26.0。

仓库版本保持基础 `X.Y.Z`，预览流水线追加 `-beta.<run>` 或 `-native-wip.<run>`。来源门禁验证 release 提交属于对应开发线，正式 tag 为 main 的 `vX.Y.Z`。两条 release 分支由已验证的对应开发提交更新。

每个平台的 manifest 包含 edition、identifier、版本、架构、features、源码提交和文件 SHA-256。收集阶段拒绝多个候选产物；发布阶段核验两平台身份与版本，生成限定渠道的 latest.json；应用更新器继续核验身份、版本和资产 URL。

开发与发行检查：`pnpm release:verify-pipeline`、`node scripts/check-release-edition.mjs`、`node scripts/check-desktop-package.mjs aarch64`（macOS）或 `x86_64`（Windows）。正式发行需要对应源码提交的构建、签名和产物证据。

当前自动实机运行暂停。安装、登录、主机/云游戏串流、手柄、退出回收和更新行为由用户手动验收；本地构建和 CI 证据分别记录。
