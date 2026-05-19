# Tauri Updater + GitHub Release 集成 — 实施报告

## 摘要

已完成 Phase 1 最小闭环的代码与 CI 配置：Tauri updater 插件、双通道 endpoint、设置页手动更新 UI，以及 `release/test → beta` / `tag v* → stable` 两条 GitHub Release workflow。仓库为公开 Release 假设，GitHub 坐标固定为 [xuasir/xbxrc](https://github.com/xuasir/xbxrc)。

## 已完成

### Rust / Tauri

- `tauri-plugin-updater`、`tauri-plugin-process` 与 `createUpdaterArtifacts`
- [`src-tauri/tauri.conf.json`](../src-tauri/tauri.conf.json)：`pubkey`、stable fallback endpoint、Windows `passive` 安装
- [`src-tauri/src/mods/updater/`](../src-tauri/src/mods/updater/)：`stable` / `beta` 通道、endpoint 组装、`updater` RPC namespace
- 能力：[`capabilities/default.json`](../src-tauri/capabilities/default.json) 增加 `updater:default`、`process:allow-restart`

### 前端

- [`src/composables/useAppUpdater.ts`](../src/composables/useAppUpdater.ts) 状态机
- [`src/pages/settings/SettingAppUpdateSection.vue`](../src/pages/settings/SettingAppUpdateSection.vue) 挂载于设置 → 通用
- RPC / 事件合同：`updater.*`、`updater.progress`

### CI

- [`.github/workflows/release-beta.yml`](../.github/workflows/release-beta.yml)：`release/test` → rolling tag `beta`
- [`.github/workflows/release-stable.yml`](../.github/workflows/release-stable.yml)：`v*` tag → GitHub `latest`

### 运维（已由开发者完成）

- 签名密钥与 GitHub Secrets：`TAURI_SIGNING_PRIVATE_KEY`、`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`

## 验证

| 项 | 结果 |
|----|------|
| `cargo check -p xbxrc` | 通过 |
| `cargo fmt --all` | 已执行 |
| `pnpm build` | 通过 |
| 变更文件 `eslint --fix` | 通过 |
| Beta/Stable 真实升级 E2E | **待 CI 首次发布后人工验证** |

## 待人工 E2E 步骤

1. 推送 `release/test`，确认 [Releases](https://github.com/xuasir/xbxrc/releases) 出现/更新 **beta** 预发布及 `latest.json`
2. 本地安装当前旧版包 → 设置 → 通用 → 切 **测试版** → 检查更新 → 下载安装 → 重启
3. 对齐 `package.json` / `tauri.conf.json` 版本后打 `v0.x.y` tag，确认 stable `releases/latest` 更新
4. 切 **稳定版** 通道重复升级验证

## 已知限制（Phase 2+）

- macOS 仅 `macos-latest`（Apple Silicon）矩阵
- 平台代码签名 / notarization 未自动化
- 启动静默检查、失败重试诊断未做
