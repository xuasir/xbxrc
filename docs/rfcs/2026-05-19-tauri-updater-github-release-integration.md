# Tauri 应用内更新与 GitHub Release 集成 RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成（Phase 1 代码已落地，E2E 待首次 Release）
- Current State: in_progress
- Owner: Codex
- Last Updated: 2026-05-19

## Background

- 当前仓库已经具备 GitHub CI 构建基础，[`.github/workflows/build-tauri.yml`](/Users/guo.xu/Documents/code/games/xbxrc/.github/workflows/build-tauri.yml) 会在 `release/**` 分支上构建 `dmg` 与 `nsis` 安装包，并上传为 GitHub Actions artifact。
- 当前发布链路还缺少 release 级分发与应用内更新入口：
  - workflow `permissions` 仍为 `contents: read`
  - 构建产物只进入 Actions artifact，没有进入 GitHub Release
  - [`src-tauri/tauri.conf.json`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/tauri.conf.json) 尚未开启 updater artifact 生成
  - [`src-tauri/Cargo.toml`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/Cargo.toml) 与 [`src-tauri/src/lib.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/lib.rs) 尚未接入 `tauri-plugin-updater`
  - [`src-tauri/capabilities/default.json`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/capabilities/default.json) 尚未声明 updater 权限
- 当前桌面应用已经是 Tauri 2 主线：
  - [`package.json`](/Users/guo.xu/Documents/code/games/xbxrc/package.json) 使用 `@tauri-apps/api` 与 `@tauri-apps/cli` `^2`
  - [`src-tauri/Cargo.toml`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/Cargo.toml) 使用 `tauri = "2"`
- 目标需求是结合 GitHub 上的 CI 构建结果，引入基于 Tauri 官方 updater 的应用内更新检测、下载、安装与重启能力，并保持当前 Tauri + Vue 3 + TypeScript + Rust 技术边界稳定。
- 本 RFC 的方案基于 2026-05-19 核对的官方资料：
  - Tauri v2 updater 官方文档
  - Tauri 官方 GitHub 发布流水线文档
  - `tauri-action` 官方 README
  - GitHub Actions / Release 官方文档

## Goal

- 建立一条单一、可验证的桌面应用更新链路：`版本号 -> CI 构建 -> GitHub Release -> latest.json -> 应用内检查更新 -> 下载并安装 -> 重启生效`。
- 保持更新实现贴合现有架构：
  - 前端只承担更新状态展示与用户操作入口
  - Tauri/Rust 侧承担 updater 插件初始化、权限与打包配置
  - GitHub Actions 承担 release 产物与 updater 元数据发布
- 首版以“设置页内手动检查更新”为主路径，形成稳定闭环后再考虑静默检查、启动提示与多渠道发布。

## Scope

- In scope:
  - GitHub Release 发布工作流与 release 资产组织
  - Tauri updater 所需的签名密钥、打包配置、权限与插件初始化
  - 前端设置页中的“检查更新 / 下载并安装 / 重启”交互
  - 版本命名、渠道假设、验证路径与运维前置条件
  - 与 updater 集成直接相关的文档更新
- Out of scope:
  - 自建更新服务器、鉴权网关或私有 CDN
  - Linux 包管理分发策略
  - 强制更新、灰度分桶、分区域升级策略
  - 与 updater 无关的整体发布流程重写
  - 将成熟的原生分发逻辑迁移到 TypeScript

## Current Repository State

### 1. 构建产物已经具备更新所需的安装包形态

- 当前 workflow 已按平台输出：
  - macOS `dmg`
  - Windows `nsis`
- 这与 Tauri updater 支持的安装包路径一致，说明现有构建链路已经接近可用状态。

### 2. 当前缺少 updater 所需的三项核心资产

- 应用内 updater 插件与权限
- release 级静态元数据 `latest.json`
- updater 签名密钥与 CI secret 注入

### 3. 当前缺少用户侧入口

- 现有设置页尚无“检查更新”入口。
- 首版更适合将该能力放在设置页，保持可见、可控、可回退，避免一开始把启动期静默流程、提示策略、失败恢复一起叠上来。

## Proposed Direction

### A. 以 GitHub Release 作为首版更新源

- 采用 GitHub Release 承载安装包与 `latest.json`。
- updater endpoint 首版固定为：
  - `https://github.com/<owner>/<repo>/releases/latest/download/latest.json`
- 这样可以直接复用 Tauri 官方推荐链路，减少自定义服务面。

### B. 保留现有 build CI，新增 release CI

- 现有 [`build-tauri.yml`](/Users/guo.xu/Documents/code/games/xbxrc/.github/workflows/build-tauri.yml) 继续承担分支上的构建验收与 artifact 输出。
- 新增独立 release workflow，承担：
  - tag 触发或人工触发的正式发布
  - GitHub Release 创建
  - 安装包上传
  - updater 元数据上传
- 这样可以将“构建验证”和“对外分发”拆成两条清晰职责线，避免在当前 build workflow 上继续堆叠发布状态判断。

### C. 应用内更新采用“手动检查优先”的窄入口

- 首版将更新入口放入设置页。
- 用户操作顺序固定为：
  1. 检查更新
  2. 展示版本差异与更新说明链接
  3. 下载并安装
  4. 安装完成后重启
- 启动时静默检查属于二期增强项；首轮优先把手动链路做稳定。

### D. 公开 Release 作为首版分发假设

- 首版方案按“GitHub Release 资产可直接下载”设计。
- 私有仓库场景涉及 release asset 访问控制、`latest.json` 拉取鉴权与客户端凭证策略，复杂度明显更高。
- 因此首轮执行建议明确其一：
  - 使用公开 Release 资产
  - 使用公开静态更新元数据与可下载资产
- 私有分发路径单独起 RFC 更稳。

### E. `beta/stable` 双通道与发布触发分流

- updater 采用两条固定通道：
  - `stable`：面向正式用户
  - `beta`：面向测试用户
- CI 侧发布触发按两类事件分流：
  - `push release/test`：发布到 `beta`
  - `push tag v*`：发布到 `stable`
- 应用内更新通道与发布事件保持一一对应，避免“测试构建进入正式通道”或“正式构建覆盖测试通道”。
- `stable` 通道可直接使用 GitHub `releases/latest/download/latest.json`。
- `beta` 通道应使用独立 feed；原因是 GitHub `releases/latest` 适合作为稳定版入口，`beta` 需要单独维护自己的 `latest.json` 与安装包集合。
- 该结构与 Tauri updater 的运行时 endpoint 切换能力一致，适合作为首版正式设计，而不是后续增强项。

## Proposed Contract

### 1. Updater 打包与签名合同

- 在 [`src-tauri/tauri.conf.json`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/tauri.conf.json) 中开启 updater artifact 生成：

```json
{
  "bundle": {
    "active": true,
    "targets": "all",
    "createUpdaterArtifacts": true
  }
}
```

- 生成 updater 签名密钥：

```bash
pnpm tauri signer generate -w ~/.tauri/xbxrc.key
```

- CI secrets 固定为：
  - `TAURI_SIGNING_PRIVATE_KEY`
  - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
- `pubkey` 写入 Tauri 配置，私钥只进入 CI secret，不进入仓库。

### 2. Tauri 配置与权限合同

- 在 [`src-tauri/tauri.conf.json`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/tauri.conf.json) 中新增 updater 配置：

```json
{
  "plugins": {
    "updater": {
      "pubkey": "-----BEGIN PUBLIC KEY-----\n...\n-----END PUBLIC KEY-----",
      "endpoints": [
        "https://github.com/<owner>/<repo>/releases/latest/download/latest.json"
      ],
      "windows": {
        "installMode": "passive"
      }
    }
  }
}
```

- 在 [`src-tauri/capabilities/default.json`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/capabilities/default.json) 中加入：

```json
"updater:default"
```

- Rust 启动链路在 [`src-tauri/src/lib.rs`](/Users/guo.xu/Documents/code/games/xbxrc/src-tauri/src/lib.rs) 中初始化：
  - `tauri-plugin-updater`
  - `tauri-plugin-process`
- `process` 插件用于安装完成后的 `relaunch()`。

### 3. 更新通道与 endpoint 选择合同

- 应用内维护固定 channel：
  - `stable`
  - `beta`
- channel 建议持久化在 store 或设置项中，默认值为 `stable`。
- updater check 由 Rust 侧统一组装 endpoint，更适合收住：
  - channel 选择
  - 未来的 `allowDowngrades`
  - 未来的自定义 `target`
- endpoint 规划建议固定为：
  - `stable`: `https://github.com/<owner>/<repo>/releases/latest/download/latest.json`
  - `beta`: 独立 feed，例如 `https://updates.example.com/beta/latest.json`，或一个固定 `beta` release/tag 对应的 `latest.json`
- 运行时选择形态建议如下：

```rust
let endpoint = match channel.as_str() {
    "beta" => "https://updates.example.com/beta/latest.json",
    _ => "https://github.com/<owner>/<repo>/releases/latest/download/latest.json",
};

let update = app
    .updater_builder()
    .endpoints(vec![endpoint.to_string()])?
    .build()?
    .check()
    .await?;
```

- 用户在 `beta -> stable` 间切换时，可能出现版本回切；执行阶段需要结合 semver 规则评估是否开启 `allowDowngrades`。

### 4. 前端交互合同

- 建议新增单独 composable，例如：
  - `src/composables/useAppUpdater.ts`
- 设置页入口建议放在：
  - [`src/pages/Setting.vue`](/Users/guo.xu/Documents/code/games/xbxrc/src/pages/Setting.vue)
  - 或新的设置 section 组件
- 前端状态机保持窄而明确：
  - `idle`
  - `checking`
  - `available`
  - `downloading`
  - `installing`
  - `installed`
  - `error`
- 首版能力固定为：
  - 读取当前版本
  - 检查是否有新版本
  - 展示目标版本号
  - 下载与安装
  - 安装完成后重启
  - 显示当前更新通道并允许切换 `stable/beta`

建议 API 形态：

```ts
import { check } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'

const update = await check()

if (update) {
  await update.downloadAndInstall((event) => {
    // Started / Progress / Finished
  })
  await relaunch()
}
```

### 5. GitHub Release 工作流合同

- 新增发布工作流，建议拆为两条职责明确的 workflow：
  - `release-beta.yml`
  - `release-stable.yml`
- 新 workflow 需要：
  - `permissions.contents: write`
  - `GITHUB_TOKEN`
  - `TAURI_SIGNING_PRIVATE_KEY`
  - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
- 推荐使用 `tauri-apps/tauri-action` 负责：
  - 创建或更新 GitHub Release
  - 上传安装包
  - 上传 updater 元数据

触发规则建议固定为：

1. `beta` workflow
   - 触发：`push.branches: ['release/test']`
   - 目标通道：`beta`
   - release 属性：`prerelease: true`
   - 版本号建议：`0.x.y-beta.N`
   - 发布要求：上传 beta 安装包与 beta `latest.json`

2. `stable` workflow
   - 触发：`push.tags: ['v*']`
   - 目标通道：`stable`
   - release 属性：`prerelease: false`
   - 版本号建议：`v0.x.y`
   - 发布要求：上传 stable 安装包与 stable `latest.json`

工作流核心要求：

1. 版本号来源保持单一
   - `package.json`
   - `src-tauri/tauri.conf.json`
   - Git tag
   - 三者保持一致

2. 平台矩阵明确
   - Windows: `nsis`
   - macOS: 明确支持 `aarch64`、`x86_64` 或只支持其中之一

3. updater 元数据按通道进入对应发布位置
   - `stable` 的 `latest.json` 指向 stable 安装包
   - `beta` 的 `latest.json` 指向 beta 安装包

4. GitHub `latest` 只服务 `stable`
   - `stable` 通道可以直接消费 GitHub `releases/latest`
   - `beta` 通道固定消费独立 feed，不与 GitHub `latest` 复用

### 6. 平台签名与用户体验合同

- updater 签名只解决“更新包可信性”。
- 平台原生代码签名继续影响：
  - macOS Gatekeeper / notarization
  - Windows SmartScreen / 安装体验
- 因此发布链路需要同时考虑：
  - Tauri updater 的签名密钥
  - macOS 代码签名与 notarization
  - Windows 代码签名证书
- 首版可以先完成 updater 闭环，再补齐平台级签名自动化；正式对外发布前建议两者都到位。

## Delivery Phases

### Phase 1: 打通 updater 最小闭环

- 接入 `tauri-plugin-updater`
- 开启 `createUpdaterArtifacts`
- 新增 `beta/stable` 两条 release workflow
- 产出 stable `latest.json` 与 beta `latest.json`
- 设置页新增手动“检查更新”与通道切换入口

### Phase 2: 提升发布质量

- 明确 macOS 架构支持矩阵
- 完善 release naming、draft/release 策略
- 补平台代码签名与 notarization
- 完成跨平台升级 smoke test

### Phase 3: 提升使用体验

- 启动时静默检查
- 版本说明展示
- 更新失败后的重试与诊断
- 通道切换后的版本回切与提醒优化

## Plan

1. 明确分发假设：公开 Release、支持平台、版本命名、`beta/stable` 通道和发布触发方式。
2. 接入 Tauri updater 配置、权限与 Rust/TS 插件。
3. 新增设置页更新入口、通道切换与下载安装状态机。
4. 新增 `release/test -> beta`、`tag -> stable` 两条 GitHub Release workflow，并接入 updater 签名 secret。
5. 用一组 beta 版本与一组 stable 版本完成真实升级验证。

## Validation

- [ ] `cargo check -p xbxrc`
- [ ] `cargo fmt`
- [ ] `pnpm lint:fix`
- [ ] `pnpm build`
- [ ] `release/test` 触发的 beta workflow 成功发布 beta 安装包与 beta `latest.json`
- [ ] `tag v*` 触发的 stable workflow 成功发布 stable 安装包与 stable `latest.json`
- [ ] macOS 本地从旧版本检查到新版本并完成安装
- [ ] Windows 本地从旧版本检查到新版本并完成安装
- [ ] 设置页更新入口能覆盖“无更新 / 有更新 / 下载中 / 安装失败”四类状态
- [ ] 设置页通道切换后能命中正确 feed

## Risks

- GitHub Release 若继续作为私有资产使用，客户端拉取 `latest.json` 与安装包会受鉴权约束，首版复杂度会明显上升。
- macOS 若只在单一架构 runner 上构建，Intel 与 Apple Silicon 支持矩阵会不清晰，应用内更新也会受影响。
- `tauri-action` 与 `tauri-plugin-updater` 版本需要保持兼容，执行时应按同一代官方文档收口。
- 当前仓库已有 `build-tauri.yml`；若直接在原 workflow 上叠发布逻辑，构建验收与正式发布状态容易耦合。
- 平台级代码签名若滞后，更新包虽然可校验，最终安装体验仍可能受到系统拦截或警告影响。
- `beta -> stable` 通道切换可能形成版本回切，版本号策略与 `allowDowngrades` 需要配套设计。
- 若 beta feed 直接复用 stable `latest` 入口，测试构建会污染正式用户升级路径，因此 beta feed 需要保持独立。

## Open Decisions

- ~~GitHub Release 是否按公开资产发布。~~ → **公开**（已确认）
- macOS 是否同时支持 `aarch64` 与 `x86_64`（Phase 2）。
- ~~首版设置页入口是否只提供手动检查~~ → **仅手动检查**（Phase 1）
- ~~beta feed 落点~~ → **固定 rolling release tag `beta`**（已确认）

## Progress

- [x] Step 1: 完成现状调研，确认当前仓库尚未接入 updater 插件、权限与 release 发布链路。
- [x] Step 2: 完成官方方案核对，收敛为 `Tauri updater + GitHub Release + tauri-action` 主线。
- [x] Step 3: 明确推荐的双通道发布策略：`release/test -> beta`、`tag v* -> stable`，并确认 `beta` 需要独立 feed。
- [x] Step 4: 与仓库现有构建 workflow 对齐并完成 Phase 1 实施（updater 模块、设置页、release-beta/stable workflow）。
- [ ] Step 5: 完成跨平台升级验证与报告收口（Report 已写，E2E 待首次 GitHub Release）。

## Execution Notes

- Date: 2026-05-19 | Status: planned
- Update: 新建 RFC，收口“GitHub CI 构建结果进入 Tauri 应用内更新”的单一路径。
- Decision: 首版采用 `GitHub Release + latest.json + Tauri updater` 官方主线，设置页手动检查更新作为首个用户入口。
- Decision: 发布分流采用 `release/test -> beta`、`tag v* -> stable`；`stable` 直接消费 GitHub `latest`，`beta` 使用独立 feed。
- Risk/Blocker: 公开分发、平台架构矩阵、平台代码签名策略仍需在执行前确认。

## External References

- [Tauri v2 Updater 官方文档](https://v2.tauri.app/zh-cn/plugin/updater/)
- [Tauri JavaScript Updater API](https://v2.tauri.app/zh-cn/reference/javascript/updater/)
- [Tauri GitHub 发布流水线文档](https://v2.tauri.app/distribute/pipelines/github/)
- [tauri-action 官方 README](https://github.com/tauri-apps/tauri-action)
- [GitHub Actions Workflow Syntax](https://docs.github.com/en/actions/writing-workflows/workflow-syntax-for-github-actions)
- [GitHub Release Assets API](https://docs.github.com/en/rest/releases/assets?apiVersion=2022-11-28)
- [Tauri macOS Code Signing](https://v2.tauri.app/distribute/sign/macos/)
- [Tauri Windows Code Signing](https://v2.tauri.app/distribute/sign/windows/)
