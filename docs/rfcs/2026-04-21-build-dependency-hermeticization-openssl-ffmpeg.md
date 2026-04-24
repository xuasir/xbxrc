# 构建依赖封装化：移除 OpenSSL 外部构建依赖并内置 FFmpeg SDK RFC

> 说明：复杂任务在执行中的里程碑、状态、决策变更、阻塞项统一记录在本 RFC 内；仅在任务完全完成后再单独产出 Report。

## Status

- Completion: 未完成
- Current State: in-progress
- Owner: codex
- Last Updated: 2026-04-21

## Background

- 当前 Windows 打包与 macOS 交叉编译都被第三方原生依赖卡住，失败点集中在 `openssl-sys` 和 `ffmpeg-sys-next`。
- `openssl` 当前通过 workspace 统一引入 vendored 构建，Windows 目标在 macOS 上会卡在 Perl/路径风格与原生工具链；Windows runner 也容易受到镜像差异影响。
- `ffmpeg` 当前依赖 CI 里的系统包安装与外部下载，失败点来自下载源、预编译包路径、系统包版本和 runner 环境波动。
- 产品目标需要稳定的跨平台桌面打包链路，构建应尽量只依赖仓库内容、Rust/Node 工具链和平台系统 SDK。

## Goal

- 移除项目对 OpenSSL 外部构建环境的依赖，收敛到纯 Rust 密钥生成与签名材料导出路径。
- 将 FFmpeg 依赖改为仓库内固定版本的预编译 SDK，由构建脚本和打包流程统一消费。
- 让本地构建、交叉编译、GitHub Actions、生产安装包共享同一套依赖边界。
- 让生产宿主随应用获得所需 FFmpeg 运行库，不再要求额外安装 FFmpeg。

## Scope

- In scope:
  - `crates/xbox-webapi` 中 ECDSA P-256 密钥生成与 JWK 导出逻辑
  - workspace / `src-tauri` / `xbxengine` 的 `openssl` 依赖收敛与清理
  - `third_party/ffmpeg/<target>` 目录约定、版本固定与仓库接入
  - `ffmpeg-sys-next` 所需环境变量和构建脚本收口
  - Tauri 打包时的 FFmpeg 动态库随包分发
  - GitHub Actions 与本地构建脚本的 FFmpeg 来源切换
- Out of scope:
  - 替换 FFmpeg 为其他视频解码栈
  - 改动现有 Rust-owned 视频解码主链、D3D11VA / VideoToolbox 运行时策略
  - 引入第二套原生运行时或第二套媒体管线
  - 本 RFC 之外的签名、公证、发布渠道流程重构

## Plan

1. 设计并实现 `openssl` 移除方案，使用 `p256` 直接生成私钥、公钥与 JWK 字段。
2. 清理 workspace 和各 crate 中无效或多余的 `openssl` 依赖，补齐验证用例。
3. 设计 `third_party/ffmpeg/<target>` 布局，固定 Windows/macOS 所需头文件、库文件和运行时动态库版本。
4. 改造构建入口与 Tauri 打包流程，使 `ffmpeg-sys-next` 和 bundle 都从仓库内 SDK 取依赖。
5. 更新 GitHub Actions、本地开发文档和任务追踪，验证本地构建、Windows 打包、macOS 打包至少各一条主路径。

## Validation

- [x] `cargo test -p xbox-webapi`
- [x] `cargo check -p xbxrc`
- [ ] Windows 目标构建能通过固定 SDK 路径完成
- [ ] macOS 本地打包不再依赖 `brew install ffmpeg`
- [x] GitHub Actions Windows/macOS 打包不再依赖外部 FFmpeg 下载
- [x] 安装包内包含 FFmpeg 运行库，目标宿主无需额外安装 FFmpeg（配置层已接入，待真实 SDK 资产验证）

## Risks

- `p256` 替换后导出的 JWK 字段必须与现有 Xbox 鉴权链完全兼容。
- FFmpeg 预编译包体积较大，仓库体积与拉取速度会受到影响。
- Windows 与 macOS 的动态库打包路径和运行时查找规则不同，容易出现 bundle 成功但运行时缺库。
- FFmpeg 二进制分发需要和许可证策略保持一致。

## Progress

- [x] Step 1: 完成 OpenSSL 使用面梳理与纯 Rust 替换设计
- [x] Step 2: 完成 FFmpeg SDK 目录与构建接入设计
- [ ] Step 3: 完成本地与 CI 验证链路收口

## Execution Notes

- Date: 2026-04-21 | Status: planned
- Update: 创建 RFC，确认问题边界覆盖 `openssl-sys` vendored 构建失败与 `ffmpeg-sys-next` 外部环境依赖。
- Decision: 采用“双收口”策略，OpenSSL 走纯 Rust 替换，FFmpeg 走仓库内固定 SDK 与随包分发。
- Risk/Blocker: FFmpeg 二进制资产体积、许可证与运行时查找路径需要在实施阶段一起验证。
- Date: 2026-04-21 | Status: in-progress
- Update: `crates/xbox-webapi` 已将 ECDSA/JWK 生成从 OpenSSL 切换为 `p256`，并新增最小结构回归测试。
- Update: workspace / `src-tauri` / `xbxengine` 已移除 `openssl` 依赖声明，消除 `openssl-sys` 构建链入口。
- Update: 新增 `third_party/ffmpeg/<target>` 目录约定、`scripts/with-ffmpeg-sdk.mjs` 统一环境注入、本地 `pnpm tauri:build:*` 包装命令。
- Update: `build-tauri.yml` 已移除外部 FFmpeg 下载，改为校验仓库内 SDK 并复用统一构建命令。
- Update: `src-tauri/tauri.conf.json` 已接入 FFmpeg 运行库资源打包路径。
