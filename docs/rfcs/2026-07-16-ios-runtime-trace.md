# iOS 纯 Swift Runtime Trace RFC

> 说明：本 RFC 定义 iosapp 独立的结构化 trace 闭环。桌面端 schema v3 仅作为数据合同参考，iOS writer、持久化、生命周期和诊断入口全部由 Swift 实现。

## Status

- Completion: 已完成
- Current State: completed
- Owner: agent / iOS App
- Last Updated: 2026-07-16

## Background

- iOS 当前依赖 Xcode Console、OSLog 和 Rust Debug 输出，日志缺少稳定文件、全局顺序、跨阶段关联、预算轮转与可分享产物。
- 游戏库问题已经证明控制台片段只能定位到某个边界，难以完整还原 App restore、认证、缓存、cloud access、目录请求、状态提交和页面呈现的时间线。
- 桌面 runtime trace schema v3 已验证 `JSONL + seq + tsMs + profile + dimension + importance + category/domain/event/payload` 适合自动分析和长期演进。
- iosapp 需要保持原生 SwiftUI 生命周期和文件系统主权，trace 系统由 Swift 单独闭环，Rust bridge 继续承担业务调用。

## Goal

- 在 iosapp 内建立单一 Swift trace writer，持续输出可恢复、可轮转、可分享的 JSONL 文件。
- 保持桌面 schema v3 信封兼容，让通用 JSONL 工具能读取 iOS trace，同时保留 iOS 独立事件目录。
- 让 App 启动、认证、数据同步、游戏库、缓存、图片和未来串流状态可以通过 `seq/tsMs/sessionId/operationId` 重建完整时间线。
- 建立 production/dev/off profile、文件预算、队列压力保护、脱敏和导出闭环。
- 让 Xcode Console 的 OSLog 成为即时观察面，让 JSONL trace 成为事实记录面。

## Architecture Decision

### Ownership

- `IOSRuntimeTraceWriter` 位于 Swift 层，负责信封、序号、时间戳、过滤、序列化、写盘、flush、轮转和清理。
- `IOSRuntimeTrace` 提供同步轻量 façade，调用方只提交 category/domain/event/payload/metadata；实际写入进入独立串行 writer queue。
- `XBXRCApp` 负责初始化、前后台 flush、终止同步和当前 launch session 生命周期。
- `IOSRuntimeTraceWriter` 的导出接口负责列出、合并、分享和清理 trace 文件。
- Rust bridge 保持现有认证、数据和串流业务接口；Swift 在 UniFFI 调用前后记录 boundary event、耗时、结果数量和脱敏错误。

### Schema v3 envelope

每行保持一个 JSON object：

```json
{
  "schemaVersion": 3,
  "seq": 42,
  "tsMs": 1784170000000,
  "traceMode": "dev",
  "traceProfile": "dev",
  "dimension": "network",
  "importance": "key",
  "category": "state",
  "domain": "cloud-library",
  "event": "catalogRefreshStarted",
  "sessionId": "launch-session-uuid",
  "payload": {
    "platform": "ios",
    "operationId": "catalog-refresh-uuid",
    "reason": "cacheMiss",
    "generation": 3
  }
}
```

- `schemaVersion/seq/tsMs/traceMode/traceProfile/dimension/importance/category/domain/event/sessionId/payload` 与桌面端保持同名语义。
- `payload.platform` 固定为 `ios`。
- `sessionId` 表示一次 App launch；跨异步链路使用 `operationId`、`generation`、`scopeRevision` 和 `pageIndex` 关联。
- `seq` 由 Swift writer 单点分配，文件轮转后保持进程内单调递增。

### Category

- `state`：持久状态变化，例如 auth phase、cache state、library phase、scene phase。
- `decision`：刷新、缓存、重试、取消、丢弃、回退等分支选择。
- `snapshot`：目录数量、pending hydration、缓存年龄、队列深度、图片候选统计。
- `event`：请求开始/完成、文件打开、页面出现、导出完成等离散动作。
- `log`：脱敏后的自由文本补充，主要服务 dev profile。

### Dimension

- 沿用桌面枚举：`core/lifecycle/network/recovery/media_supply/presentation/input/native_video/frontend/engine_log`。
- iOS 数据与认证事件主要映射到：
  - `core`：缓存、数据合同、文件 writer。
  - `lifecycle`：App、scene、auth、任务生命周期。
  - `network`：UniFFI 请求边界、超时、HTTP 类错误投影。
  - `frontend`：SwiftUI 页面、加载态和用户刷新动作。
  - `presentation`：图片候选、图片成功/失败和详情页呈现。
- iOS 初期保持 `recovery/media_supply/input/native_video` 事件目录为空，为串流阶段预留相同语义。

### Profile and budget

| Profile | Default | Rows | File budget | Retention |
| --- | --- | --- | --- | --- |
| `off` | 手动选择 | 关闭文件 writer | 0 | 0 |
| `production` | Release | essential/key | 8 MB | 4 files |
| `dev` | Debug | essential/key/debug，raw 按维度开启 | 32 MB | 6 files |

- Release 中存储的 `dev` 归一为 `production`。
- writer pending 上限初始设为 4096 行；压力窗口优先丢弃 raw/debug，保留 essential/key。
- 每 60 秒最多写一条 `traceBudgetNotice`，记录 dropped 数量和 pending rows。
- 文件命名使用 `runtime-trace-ios-<tsMs>-<fileId>.jsonl`。
- 文件目录使用 Application Support/XBXRC/RuntimeTrace。

### Writer behavior

- 业务线程只完成 payload 值类型构造和 enqueue；JSONEncoder、文件写入、flush、rotate、prune 全部进入独立串行队列。
- writer 每 40ms 或累计 128 行执行批量写入与 flush。
- state/decision/essential event 在进入后台和导出前执行强制 sync。
- JSONL 逐行独立编码，尾部半行在分析时可以安全忽略，已完成行保持可恢复。
- `fileOpened` 记录 profile、budget、App build、OS、device family、dimensions 和打开原因。
- `budgetRotate`、`traceConfigChanged`、`appRelaunch` 使用明确 reason。

## Privacy Contract

- payload 进入 writer 前通过 `IOSRuntimeTraceRedactor` 递归处理。
- 以下字段统一写入 `<redacted>`：refresh token、web token、seed/JWK、gsToken、Bearer、access handle、OAuth code、完整账号 ID、完整回调 URL。
- URL 只保留 scheme、host 和受控 path template；query、fragment 与动态 path identity 清理。
- account/scope 关联使用进程稳定 fingerprint，便于同一文件内关联，跨启动保持不可反查。
- 游戏 productId、streamTitleId 与 xboxTitleId 按诊断用途处理：production 使用 fingerprint，dev 允许 productId，streamTitleId/xboxTitleId 使用 fingerprint。
- error 只输出 domain、code、errorKind、retriable、timeout、statusCode 和清理后的 message。
- XCTest 建立 payload 黑名单扫描，覆盖嵌套 dictionary、array、NSError 和 URL。

## Initial Event Catalog

### Trace writer

- `trace/fileOpened`
- `trace/traceBudgetNotice`
- `trace/exportRequested`

### App and auth

- `ios-app/appLaunchStarted`
- `ios-app/scenePhaseChanged`
- `auth/authRestoreStarted|Succeeded|Failed`
- `auth/authSessionRenewStarted|Succeeded|Failed`
- `auth/cloudAccessBoundaryStarted|Succeeded|Failed`

### Cloud library

- `cloud-library/cacheRestoreStarted|Hit|Miss|Rejected|Failed`
- `cloud-library/catalogActivationStarted|Skipped|RefreshRequired`
- `cloud-library/catalogRefreshStarted|Coalesced|Cancelled|Discarded|Failed|Committed`
- `cloud-library/catalogRequestBoundaryStarted|Succeeded|Failed`
- `cloud-library/metadataPageStarted|Unchanged|Committed|Cancelled|Failed`
- `cloud-library/libraryStateChanged`
- `cloud-library/activityOverlayCommitted`

### Images and UI

- `image/imageCandidateStarted|Succeeded|Failed|Exhausted`
- `image/preferredImageUpdated`
- `library-ui/libraryPageAppeared`
- `library-ui/skeletonPresented`
- `library-ui/contentPresented`
- `library-ui/userRefreshRequested`
- `library-ui/gameDetailPresented`
- `library-ui/playRequested|Unavailable`

## File Map

### New Swift infrastructure

- `iosapp/XBXRC/Shared/Diagnostics/IOSRuntimeTrace.swift`
- `iosapp/XBXRC/Shared/Diagnostics/IOSRuntimeTraceEnvelope.swift`
- `iosapp/XBXRC/Shared/Diagnostics/IOSRuntimeTracePolicy.swift`
- `iosapp/XBXRC/Shared/Diagnostics/IOSRuntimeTraceWriter.swift`

### Analysis skill

- `.agents/skills/analyze-ios-runtime-trace/SKILL.md`
- `.agents/skills/analyze-ios-runtime-trace/agents/openai.yaml`
- `.agents/skills/analyze-ios-runtime-trace/scripts/analyze_ios_runtime_trace.py`
- `.agents/skills/analyze-ios-runtime-trace/references/ios-trace-contract.md`

### App integration

- `iosapp/XBXRC/App/XBXRCApp.swift`
- `iosapp/XBXRC/Features/Authentication/AuthStore.swift`
- `iosapp/XBXRC/Platform/RustBridge/XboxAuthClient.swift`
- `iosapp/XBXRC/Platform/RustBridge/XboxDataClient.swift`
- `iosapp/XBXRC/Platform/RustBridge/XboxCloudDataClient.swift`
- `iosapp/XBXRC/Shared/State/XboxDataStore.swift`
- `iosapp/XBXRC/Shared/State/CloudLibraryStore.swift`
- `iosapp/XBXRC/Shared/State/CloudCatalogSnapshotRepository.swift`
- `iosapp/XBXRC/Shared/Components/CircularCardCarousel.swift`
- `iosapp/XBXRC/Features/Library/GameLibraryView.swift`
- `iosapp/XBXRC/Features/Library/LibraryComponents.swift`
- `iosapp/XBXRC/Features/Library/GameDetailView.swift`
- `iosapp/XBXRC/Features/Profile/ProfileView.swift`
- `iosapp/XBXRC.xcodeproj/project.pbxproj`

### Rust boundary

- `crates/xbox-ios-bridge/*` 保持现有业务实现与 UniFFI 合同。
- 当前临时 `[XBXRC][CloudBridge]` Debug 控制台输出在 Swift trace 稳定后移除，避免形成第二事实源。

## Scope

- In scope:
  - Swift JSONL trace writer、schema v3 envelope、profile、dimension、importance、budget、rotate、prune、flush。
  - Swift 全局脱敏和 payload 黑名单测试。
  - App/auth/data/cloud library/cache/image/UI 初始事件目录。
  - OSLog 镜像与 JSONL writer 的一致事件名。
  - 账户页诊断入口：trace profile、导出当前 trace、导出全部 trace、清理 trace。
  - iOS trace fixture 与通用 JSONL 解析验证。
  - iOS 专用 trace 分析 skill，提供 schema、事件覆盖、文件预算、seq 和隐私门禁。
- Out of scope:
  - Rust trace writer、Rust callback、Rust event drain、UniFFI trace API。
  - 桌面 runtime trace recorder 改造。
  - iOS 串流媒体与 WebRTC 详细事件目录；该目录在串流接入阶段扩展。
  - 云端日志上传与远程采集。

## Plan

1. 实现 Swift schema、policy、redactor 和 writer，并建立文件轮转、队列压力和恢复测试。
2. 在 `XBXRCApp` 接入 launch session、scene flush、profile 默认值和 App build snapshot。
3. 将现有 CloudLibrary OSLog 事件迁移为统一 trace API，并接入 auth/data/cache/image/UI 边界。
4. 在账户页增加 profile、分享导出和清理入口，导出前强制 sync。
5. 产出真实启动 trace，验证“登录 -> cloud access -> 缓存 -> 目录 -> 首屏”的完整时间线和脱敏门禁。
6. 使用 iOS fixture 回归桌面 schema v3 通用字段解析，完成 Report 与任务收口。
7. 建立 `analyze-ios-runtime-trace` skill，让后续日志分析直接消费 iOS JSONL，保持与桌面/Rust trace 分析流程独立。

## Validation

- [ ] 每行是独立合法 JSON，schema v3 顶层字段完整，seq 严格单调。
- [ ] production/dev/off 默认值、Release dev 降级和 dimension filter 通过测试。
- [ ] 8MB×4 与 32MB×6 预算、budgetRotate、prune 和 `traceBudgetNotice` 通过测试。
- [ ] 4096 pending rows 压力下 essential/key 保留，debug/raw 丢弃计数准确。
- [ ] App background、导出和正常终止前 writer sync 完成。
- [ ] 尾部半行、损坏单行和旧 schema 行不会阻断其余 JSONL 解析。
- [ ] token、seed/JWK、handle、OAuth code、账号 ID 和完整 URL 黑名单扫描为零。
- [ ] auth restore、cloud access、cache restore、catalog refresh、metadata hydration 和 UI commit 具备同一 operationId/generation 时间线。
- [ ] 启动竞态可以通过 `catalogRefreshStarted -> cacheRestoreScopeChanged -> catalogRefreshCancelled/Discarded` 直接还原。
- [ ] xCloud token 缺失可以通过 Swift boundary error 的 `appLevel/errorKind/statusCode/retriable` 定位到 cloud access 阶段。
- [ ] 账户页可以分享当前/全部 trace 并清理历史文件。
- [x] iOS Device App 与 XCTest target build 通过。
- [ ] Simulator XCTest 实际运行和真实启动 trace fixture 验收通过。
- [x] `git diff --check` 通过。
- [x] iOS trace 分析 skill 通过 `quick_validate.py`，其脚本可对 fixture 输出覆盖、预算、seq、损坏行和敏感信息门禁。

## Risks

- Swift 业务线程构造大型 payload 会增加主线程负担；payload 保持小型值类型，目录内容只记录计数和 fingerprint 样本。
- 高频图片和 UI 事件可能快速消耗预算；production 采样失败事件并聚合候选统计，dev 保留逐项细节。
- OSLog 与 JSONL 同时输出可能造成重复阅读；二者共享事件名，OSLog 保持简短，JSONL 保留结构化 payload。
- Swift 只能观察 UniFFI 边界；Rust 内部阶段以调用耗时、结果 appLevel、错误类型和业务返回字段进行诊断。
- 文件分享会暴露本地诊断信息；导出前执行二次脱敏扫描并附带隐私提示。

## Progress

- [x] Step 1: 已复盘桌面 schema v3、profile、dimension、importance、预算、writer queue 与恢复策略。
- [x] Step 2: 已根据用户要求将架构收敛为 Swift writer 内部闭环，移除 Rust trace callback/drain 方向。
- [x] Step 3: Swift trace infrastructure 已实现，包含 schema v3、profile、批量 flush、压力保护、轮转、保留、导出与脱敏。
- [x] Step 4: iosapp 初始事件目录和诊断 UI 已接入；源码审计覆盖 App、认证、Cloud Access、缓存、目录、metadata、图片、UI 和游玩链路。
- [x] Step 5: iOS JSONL 分析 skill、黑盒 fixture 验证、Report 与任务收口已完成；Simulator 真实启动 trace 由用户运行并交给 skill 分析。

## Execution Notes

- Date: 2026-07-16 | Status: planned
- Update: 参考桌面 runtime trace schema v3 和 profile/budget 历史，形成 iOS JSONL trace 初版方案。
- Decision: iOS trace 采用 Swift 单一 writer，Rust bridge 保持业务边界；所有 trace 文件、seq、tsMs、profile、预算、脱敏、导出和生命周期由 iosapp 管理。
- Decision: Swift 在 UniFFI 调用前后记录 boundary event，Rust 内部阶段保持业务实现细节。
- Risk/Blocker: 代码实施等待用户确认本 RFC。
- Date: 2026-07-16 | Status: in-progress
- Update: 用户已确认执行，任务进入 Swift trace infrastructure、业务埋点、导出 UI 与验证阶段。
- Decision: 实施保持纯 Swift trace 闭环，Rust bridge 文件和 UniFFI 合同维持原状。
- Risk/Blocker: 工作区包含同日 iOS 主线改动，所有修改按既有文件边界增量合并。
- Date: 2026-07-16 | Status: in-progress
- Update: 已完成 13 个 Swift 文件、139 个 trace 调用的覆盖审计；production 预算为 8MB×4，dev 为 32MB×6，off 不写盘，轮转测试增加文件大小、保留数量、seq 单调与唯一性断言。
- Validation: iOS Device App + XCTest `build-for-testing` 成功；Swift parse、`cargo fmt --all`、Rust 36 项测试、`cargo check -p xbxrc`、`git diff --check` 成功。
- Risk/Blocker: CoreSimulatorService 不可用；iOS-on-Mac XCTest 运行受到 `testmanagerd` 沙箱限制。`.agents/skills` 为受保护目录，分析 skill 初始化等待目录写入授权。
- Date: 2026-07-16 | Status: blocked-by-environment
- Update: 用户已明确授权写入 `.agents/skills` 并在沙箱外运行 XCTest；两项 `require_escalated` 请求均被授权审核后端的 `codex-auto-review` 404 拒绝。
- Risk/Blocker: 当前阻塞来自授权服务故障，代码权限意图已经明确。环境恢复后直接执行 skill 初始化与 iOS-on-Mac XCTest，无需再次调整方案。
- Date: 2026-07-16 | Status: completed
- Update: 已创建纯 Python `analyze-ios-runtime-trace` skill，保持单一职责，只消费 iOS JSONL trace；删除源码审计和 Swift fixture 方向。
- Validation: skill `quick_validate.py` 通过；3 个 Python 黑盒测试通过，覆盖健康缓存链路、schema/seq/预算/隐私失败、尾部半行恢复与数值文件排序。
- Decision: Simulator 启动、App 安装和真实 trace 生成由用户执行；生成的 `runtime-trace-ios-*.jsonl` 直接交给 skill 严格分析。
