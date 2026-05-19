---
name: release-desktop
description: >-
  发布 xbxrc 桌面应用到 GitHub Release（beta 或 stable）。根据用户给出的版本号 bump
  package.json/tauri.conf.json、推送 release/test 或打 v* tag 触发 CI。在用户说「发布、发版、
  release、beta、stable、版本号」时使用。
---

# 桌面应用发布（xbxrc）

仓库：[xuasir/xbxrc](https://github.com/xuasir/xbxrc)。用户**只需说明通道与版本号**（如「发 stable 0.2.0」「发 beta 0.2.0」），按本 skill 执行。

## 通道速查

| 通道 | 用户怎么说 | 配置文件版本 | Git 操作 | CI | 应用内更新源 |
|------|------------|--------------|----------|-----|----------------|
| **beta** | beta / 测试版 | `0.2.0`（仅基础 semver） | push **`release/test`** | `release-beta.yml` | `releases/download/beta/latest.json` |
| **stable** | stable / 正式版 | `0.2.0`（与 tag 一致） | tag **`v0.2.0`** + push | `release-stable.yml` | `releases/latest/download/latest.json` |

- Beta 实际构建版本：`{基础版本}-beta.{github.run_number}`（CI 自动，勿写入仓库）
- Stable：tag 为 `v0.2.0`，`package.json` / `tauri.conf.json` 必须为 `0.2.0`（**无 v**）
- `build-tauri.yml` 只打 artifact，**不会**发 Release

## 解析用户意图

从用户消息提取：

1. **通道**：含 `beta`/`测试`/`test` → beta；含 `stable`/`正式`/`稳定` → stable；否则 **AskQuestion** 二选一
2. **版本**：匹配 `X.Y.Z`（可带前缀 `v`，写入文件时去掉 `v`）

示例：

- 「发 beta 0.2.0」→ beta + `0.2.0`
- 「正式发布 v0.3.1」→ stable + `0.3.1`
- 「发 0.2.0」→ 缺通道，先问 beta 还是 stable

## 执行流程（必须由 agent 在终端实际执行）

### 前置检查

```bash
git status --short
git branch --show-current
```

- 工作区应干净，或先提交/暂存与发版无关的改动
- 确认 `origin` 指向 `xuasir/xbxrc`
- Secrets 已配置（`TAURI_SIGNING_PRIVATE_KEY` 等）— 仅提醒，勿要求用户贴私钥

### Step 1：统一版本号

```bash
node scripts/release-set-version.mjs <semver>
```

验证：

```bash
node -p "require('./package.json').version"
node -p "require('./src-tauri/tauri.conf.json').version"
```

两处输出必须相同，且等于 `<semver>`。

### Step 2a：发布 **beta**

1. 提交版本 bump（若尚未提交）：
   ```bash
   git add package.json src-tauri/tauri.conf.json
   git commit -m "chore(release): bump version to <semver> for beta"
   ```
2. 将提交送到 **`release/test`**：
   ```bash
   git push origin HEAD:release/test
   ```
   或 checkout `release/test` → merge → `git push origin release/test`
3. 用 `gh` 监视 CI：
   ```bash
   gh run list --workflow=release-beta.yml --limit 3
   gh run watch --exit-status
   ```
4. 完成后检查：
   ```bash
   gh release view beta
   ```
   确认存在 `latest.json` 与 dmg/nsis。**勿删除 tag `beta` 的 Release。**

### Step 2b：发布 **stable**

1. 提交版本 bump：
   ```bash
   git add package.json src-tauri/tauri.conf.json
   git commit -m "chore(release): release v<semver>"
   ```
2. 打 tag 并推送：
   ```bash
   git tag v<semver>
   git push origin HEAD
   git push origin v<semver>
   ```
3. 监视 CI：
   ```bash
   gh run list --workflow=release-stable.yml --limit 3
   gh run watch --exit-status
   ```
4. 验证：`gh release view v<semver>`

### 发版后说明（给用户）

- 设置 → 通用 → 应用更新：beta 选「测试版」，stable 选「稳定版」
- 本机需装旧版才能验证「检查更新」

## 禁止事项

- 不要修改 `pubkey` 或 GitHub Secrets
- 不要在仓库里提交 `-beta.N` 后缀版本
- 不要 force push，除非用户明确要求
- 不要删除 GitHub 上 tag **`beta`** 的 rolling Release

## 参考

- RFC：`docs/rfcs/2026-05-19-tauri-updater-github-release-integration.md`
- Workflows：`.github/workflows/release-beta.yml`、`.github/workflows/release-stable.yml`
