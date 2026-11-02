# 浏览器稳定发行与原生 WIP 分支、构建裁剪方案

日期：2026-11-02。状态：用户已确定并授权实施；执行进度见 main/rust 发行隔离 RFC。

## Problem Framing

稳定发行以 Browser Gamepad API、浏览器 WebRTC 和 WebView 音视频直出为产品能力。Rust WebRTC、解码/呈现与 Rust Gamepad 归原生 WIP，通过指定 release 分支提供独立预览包。目标同时覆盖稳定包体积、构建工具依赖、设置交互、更新渠道和后续开发组织。

2026-11-02 用户确定开发分支命名为 `main` 与 `rust`，并分别通过 `release/test` 与 `release/native-wip` 打包；当前方案采用此方向。`main` 负责浏览器稳定交付，`rust` 负责原生 WIP；release 分支只承担发行。原生源码、依赖、SDK 与构建工具集中在 `rust`，Rust 媒体和手柄分别保留模块与可选构建特性，统一装配成一个 Native WIP 预览包。

## Current Constraints

- 保持 Tauri + Vue 3 + TypeScript + Rust。稳定版的 Rust 继续负责认证、Xbox API、会话、排队、HTTP 信令、keepalive、持久化、窗口、更新和日志。几何导航、公共输入路由与浏览器完整功能继续保留。
- 用户已确认 FFmpeg 保持 macOS 26.0 要求；本轮发行方案沿用应用最低系统版本 26.0。
- P0 已实现默认关闭的独立预览开关与原生按需运行；P0 的平台与长测继续手动验收。本轮讨论更新了原生能力的发行归属。
- 当前 `src-tauri/Cargo.toml` 无 edition/原生 feature 隔离，直接依赖 `xbxengine`、`ohmygamepad-core/sdl3`、`wgpu` 及平台图形库。`xbxengine` 进一步引入 `rtc*`、FFmpeg、cpal 和 opus。
- `tauri.conf.json` 的基础资源包含各平台 FFmpeg 资产及 Windows 工具；`pnpm tauri` 固定经过 FFmpeg SDK wrapper。stable/beta CI 都配置原生工具、验证 SDK 并构建相同依赖集合。
- `AppState`、shell 初始化/退出、RPC、streaming service、trace policy 和配置实时应用均引用原生 provider。`xbox-streaming` 对 `xbxengine-protocol` 的依赖主要是轻量领域合同，整理时按实际消费者保留。
- 前端静态导入 NativeGamepadBackend 和原生 runtime factory；宿主能力目前按桌面平台判定。稳定构建需要同时收紧前端入口与宿主编译能力。
- 当前更新渠道为 stable/beta，应用标识为 `com.xuasir.xbxrc`。独立 Native WIP 需要自己的应用标识、安装身份、配置/日志目录与更新 endpoint。
- 当前本地分支有 `main`、`release/test`、`feat/rtc`、`feat/gamepad` 和 `feat/rust`。旧功能分支分别停留在六月/四月，当前 P0 修改仍在工作树；新的 WIP 基线应承接本轮代码和证据。

依据：[P0 RFC](../rfcs/2026-11-02-browser-stable-p0.md)、[早期发行隔离探索](2026-10-27-browser-stable-desktop-plan.md)、当前 Cargo/脚本/shell/updater 源码。

## Options

| 方案 | 收益 | 代价 |
| --- | --- | --- |
| A. 同一分支通过两个 edition 发行 | 公共修改天然同步，构建隔离可以实现最小稳定包 | 原生 WIP 源码仍在稳定开发线，提交及评审节奏相互影响 |
| B. main/rust 开发线 + 各自 release 入口 | 稳定源码与依赖范围清晰，WIP 可持续实验，开发与发布职责分开；推荐采用 | 公共修改需要同步，初次拆分需要明确文件和模块归属 |
| C. Rust WebRTC 与 Rust Gamepad 各自长期开发/发行 | 单项实验可以独立推进 | 长期维护两套集成与预览组合，UI/公共路由修复需要多处同步 |

## Recommended Direction

### 分支和发布入口

| 分支/入口 | 职责 | 产物 |
| --- | --- | --- |
| `main` | 浏览器稳定产品、公共基础与正式 `vX.Y.Z` tag 来源 | Browser stable 最小包 |
| `rust` | Rust WebRTC/媒体与 Rust Gamepad WIP 开发、集成和验证 | 原生开发构建 |
| `release/test` | 从 main 已验证提交同步，打包浏览器发布候选 | Browser beta 最小包，继续使用 beta 渠道 |
| `release/native-wip` | 从 rust 已验证提交同步，打包原生预览 | Native WIP prerelease，独立 rolling release |
| 短期 `codex/native-media-*` / `codex/native-gamepad-*` 等 topic 分支 | 单项实现与评审 | 合入原生 WIP 开发线 |

两条开发线分别对应浏览器稳定与聚合原生 WIP，两条 release 分支仅承载各自已验证提交及发行 metadata。媒体与手柄在 `rust` 内的职责由模块、feature 和 topic 分支表达。

`main` 的源码、workspace、依赖图和发行清单仅包含浏览器交付所需组件。`xbxengine` 原生运行时、原生呈现、SDL3 手柄、原生前端适配/面板及 FFmpeg SDK 归 `rust`。稳定信令需要的轻量 DTO 归公共会话/信令模块，原生运行时 DTO 归原生实现；初次拆分时整理当前 `xbxengine-protocol` 的跨层引用。

公共修复优先在 `main` 完成，再同步到 `rust`；WIP 中发现的公共问题拆为独立提交回流 `main`。原生实现先在 `rust` 完成证据验收，成熟后按单项能力评估进入稳定发行。两条开发线复用同一公共 UI、输入路由、认证与会话合同，分歧集中在装配和原生实现。

初次拆分先保存当前 P0 工作树及验收基线，然后建立两条分支的文件归属。`main` 的一次性裁剪提交同步到 `rust` 时显式保留原生模块并完成原生装配适配；后续公共同步从该已处理的分支边界继续。当前 `main` 继续承担稳定开发职责；`rust` 分支的建立、默认分支核对和 release 来源约束列入实施准备。

### 构建合同

构建产物仍记录 edition，建议值为 `browser` 与 `native-wip`，由开发/发行来源固定。各自构建入口装配 Cargo、Vite、Tauri 配置、包名、应用标识和 updater endpoint，构建清单记录 edition、实际 features、架构与源码身份；CI 核验分支来源与产物 edition 一致。

`main` 的 Cargo 清单和 workspace 移除原生专用依赖及成员，shell、AppState、RPC、trace、service 和退出路径只装配公共能力。`rust` 保留原生实现，并独立定义 `native-media`、`native-gamepad`，通过 `native-wip` 聚合，方便逐项构建与验证。媒体 feature 覆盖原生 WebRTC、FFmpeg/音频和呈现依赖，手柄 feature 覆盖 SDL3 与专用设备依赖。

稳定 shell 继续创建公共服务；streaming 的能力与装配读取编译能力事实。构建身份与公共诊断从媒体 provider 中抽离，trace 的原生 hook 随 feature 装配。轻量 DTO/公共输入协议按实际用途保留。

`main` 直接执行普通 Tauri 构建，资源仅包含稳定应用所需文件。`rust` 的 `native-media` 构建使用 FFmpeg wrapper，原生资源通过 overlay 追加当前平台/架构的运行依赖与许可证；包内保留实际运行必需文件。FFmpeg SDK、原生构建脚本和门禁随原生开发线维护。构建目录与 CI cache 按 edition/架构隔离。

### 产品与交互合同

`main` 的设置仅呈现浏览器能力和公共诊断，前端源码移除原生 backend/runtime factory、面板及配置入口，宿主能力固定表达浏览器发行实际拥有的能力。P0 的 Rust 媒体/原生手柄开关归入 `rust` 的 Native WIP 预览包；WIP 宿主根据自身编译 feature 返回能力，Rust RPC 保持对应的编译边界和启动校验。

Native WIP 采用两个独立后端开关，保持“媒体下次串流生效、手柄下次应用启动生效”的既有交互；默认保留 Browser 对照路径，方便分别验证两种原生能力和组合行为。

最小裁剪针对原生 WIP 及发行冗余。主机/云游戏、Browser 输入、几何导航、画质/SR、麦克风、截图、统计与公共恢复等现有浏览器能力按当前合同保留。

### 独立安装与更新

建议 Native WIP 使用产品名 `xbxrc Native WIP`、应用标识 `com.xuasir.xbxrc.native-wip`，对应独立的 Windows 安装/卸载身份及各平台数据目录。稳定与 WIP 可同时安装、分别登录并保留配置/日志。

Browser stable/beta 继续使用当前 stable/beta endpoints。Native WIP 使用独立 `native-wip` GitHub prerelease 和 `latest.json`，版本建议为 `<base>-native-wip.<run>`。更新服务按 edition 限定可用渠道，WIP 的更新源固定归原生预览。稳定发布验证正式 SemVer 与来源提交，WIP 发布使用自己的 tag/rolling release 名称。

### 验收合同

1. `main` 的 workspace 与源码清单仅包含稳定模块；稳定构建在 SDK/原生工具不可用的干净环境下完成，Cargo 依赖树证明原生媒体/手柄与专用图形、FFmpeg、音频依赖已退出。
2. 检查稳定 app/exe 的动态依赖、资源清单及前端 bundle，确认发行仅包含稳定 edition 的组件；按实际构建测量包大小与构建耗时。
3. `main` CI 编译/测试 Browser，`rust` CI 验证 Browser 对照、media-only、gamepad-only、组合配置。稳定发行执行 Browser 门禁，WIP 执行原生门禁及 SDK/签名闭包检查。
4. 交叉检查前后端 edition、RPC 能力与设置合同，以及安装身份和更新 manifest；来源错误/跨 edition 产物由发行校验拒绝。
5. 实机与长测沿用户已确认的手动验收约定；新稳定包与 WIP 包分别记录产物身份，复用 P0 的功能和资源回收检查。

## Open Questions

分支角色已按用户追加方向确定为 `main` / `rust` 开发、`release/test` / `release/native-wip` 发行。应用名与 updater rolling release 名称沿用本文建议，在实施 RFC 中固定。当前本地 `origin/HEAD` 指向 `release/test`；实施时核对 GitHub 默认分支与发布保护规则。

真实最小包体积、平台依赖闭包、干净机器构建/安装和更新行为需要实施后的产物证明；这些项目属于验收事实。

## Candidate Follow-On Tasks

| 顺序 | 任务 | 类型 |
| --- | --- | --- |
| P1-0 | 固定实施 RFC，保存 P0 基线，建立 main/rust 与公共同步边界 | complex |
| P1-1 | main 源码、workspace 与依赖裁剪；rust 原生集成适配 | complex |
| P1-2 | 两条开发线的前端入口、能力、设置与公共合同整理 | complex |
| P1-3 | 独立构建资源、wrapper、应用身份与更新渠道 | complex |
| P1-4 | 两条开发 CI、两条 release 来源校验与包内容/安装更新门禁 | complex |

实施按依赖隔离、界面装配、发行隔离、产物验收推进，执行记录见[实施 RFC](../rfcs/2026-11-02-main-rust-release-isolation.md)。
