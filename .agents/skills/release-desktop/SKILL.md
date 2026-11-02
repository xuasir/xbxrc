---
name: release-desktop
description: 发布 xbxrc Browser stable/beta 或 Native WIP，按 main/rust 版型验证来源、版本、安装身份、资源和更新清单。
---

# 桌面应用发布

仓库：[xuasir/xbxrc](https://github.com/xuasir/xbxrc)。先读取 `app-edition.json`、当前分支及干净工作树，按照版型处理。

| edition | 开发来源 | 发行入口 | 实际版本 | 更新源 |
| --- | --- | --- | --- | --- |
| browser stable | main | vX.Y.Z tag | X.Y.Z | releases/latest/download/latest.json |
| browser beta | main | release/test | X.Y.Z-beta.<run> | releases/download/beta/latest.json |
| native-wip | rust | release/native-wip | X.Y.Z-native-wip.<run> | releases/download/native-wip/latest.json |

Browser 名称为 xbxrc，identifier 为 com.xuasir.xbxrc。Native WIP 名称为 xbxrc Native WIP，identifier 为 com.xuasir.xbxrc.native-wip；配置、日志与更新身份独立。应用和 FFmpeg 的 macOS 最低要求保持 26.0。

## 执行

1. 核对用户授权的通道和基础版本。用户请求发布时，缺失的通道/版本应澄清；代码与发布机制改造按授权范围先完成可审查结果。用户的实机暂停约定继续生效。
2. 在对应开发线上运行 `node scripts/release-set-version.mjs X.Y.Z`。仓库版本保持基础 X.Y.Z；CI 使用 `release-set-preview-version.mjs` 自动追加预览后缀。
3. 验证 `pnpm release:verify-pipeline`、`node scripts/check-release-edition.mjs <edition>`、前端与对应 Rust 检查、实际产物身份/签名/运行库。main 直接 Tauri 构建；rust 使用 FFmpeg wrapper 和 native-media/native-gamepad 聚合特性。
4. 提交并推送对应 main/rust 开发提交，确认发行提交已经进入对应远端开发分支。release/test 必须来自 origin/main；release/native-wip 必须来自 origin/rust；正式 vX.Y.Z tag 必须来自 main 且版本一致。
5. 仅在用户已授权真实发布时触发：Browser beta 推送已验证 main 提交到 release/test；Native WIP 推送已验证 rust 提交到 release/native-wip；stable 在 main 的对应提交打 vX.Y.Z 并推送 tag。推送 release 分支会立即触发真实发布。
6. 监视对应 GitHub Actions，检查平台 manifest 与 latest.json 的 edition、identifier、架构、源码 SHA、features、文件哈希和版本。Browser rolling tag 为 beta；Native WIP rolling tag 为 native-wip，均设为 prerelease 且 latest=false。

## 边界

- 保留 pubkey 和现有 GitHub signing secrets。
- 保留 beta/native-wip rolling Release。
- force push 需要用户明确授权。
- main/rust push 与 PR 自动构建只上传验证 artifact；release 分支与 stable tag 执行发布。
- 自动实机暂停时，安装、登录、串流、手柄、退出回收、更新与长测继续手动验收；构建、单元测试、隔离探针和只读包检查可以执行。

依据：[发行隔离 RFC](../../../docs/rfcs/2026-11-02-main-rust-release-isolation.md)、[开发与发行说明](../../../dev-docs/desktop-editions.md)。
