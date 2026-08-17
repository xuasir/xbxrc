# iOS Swift 协调层架构与生命周期稳健性优化 RFC

> 本 RFC 在现有 Tauri + Rust 共享领域主线和 iOS 原生运行时边界内，收敛 Swift 协调层职责并修复认证、缓存、设置和串流生命周期的一致性问题。

## Status

- Completion: 实施完成，运行时验收待完成
- Current State: validation-pending
- Owner: agent
- Last Updated: 2026-08-05
- Execution Gate: 用户已确认执行

## Background

iOS App 当前采用 SwiftUI + Swift Store/Actor + UniFFI Rust bridge。整体技术边界合理：Rust 负责 Xbox 认证、Web API、access lease、session flow 和共享协议，Swift 负责系统 UI、Keychain、UserDefaults、libwebrtc、Metal、GameController 和应用生命周期。

代码评审确认 Swift 协调层存在两类集中风险：一类是跨 `await` 状态提交缺少统一 owner generation/scope revision 校验，可能在退出、换号或并发 access 准备后恢复旧状态；另一类是 `AuthStore`、`CloudLibraryStore`、`XboxDataStore`、`StreamSessionActor` 等类型同时承担过多职责，状态事实由大量布尔、Task 和可选资源隐式组合。

## Goal

- 让凭据、账号 scope、缓存 snapshot 和串流资源都通过显式 owner revision 提交，旧异步结果无法复活已失效状态。
- 让退出登录的内存状态与 Keychain 持久状态保持一致，并为失败建立可恢复终态。
- 让设置具备版本、迁移、加载归一化和稳定默认值。
- 让串流 cleanup 先隔离显示输入，再关闭控制面与 access，消除 terminal 后晚到上屏窗口。
- 将 Swift 大型 Store/Actor 拆为单一生命周期 owner 加若干纯状态/协调组件，保持现有 UI、Rust 和 libwebrtc 合同稳定。

## Scope

### In Scope

- `AuthStore` 的 credential revision、access prepare single-flight、Keychain 提交和退出终态。
- `CloudLibraryStore` 与 `CloudCatalogSnapshotRepository` 的 scope revision、持久化任务所有权、CAS 风格提交和账号隔离。
- `XboxDataStore` 的 locale-aware surface key、账号作用域快照边界与状态拆分。
- `AppSettingsStore` 的版本化 snapshot、迁移、归一化和持久化适配。
- `StreamingFeatureStore`、`StreamSessionActor` 和 renderer 的 teardown/readiness/ICE 状态拆分。
- Swift/Rust input 和 rumble 的共享 golden fixture 门禁。
- focused XCTest、Rust bridge 门禁、Swift 6 typecheck、runtime trace gate 和真机/模拟器回归。

### Out Of Scope

- 改写 Xbox 协议、WebRTC transport、session flow 或 Rust 领域主线。
- 将 libwebrtc、Metal、GameController、Keychain 或 SwiftUI 迁移到 Rust。
- 引入新的客户端框架、第二套 bridge、持久化数据库或全量状态管理框架。
- UI 视觉重做、功能扩展和与稳健性无关的通用重构。

## Architecture Contract

### Rust Ownership

- Xbox auth/web API、token rotation、access lease、cloud/home session flow。
- session/negotiation/ICE/TURN plan、DataChannel bootstrap、终态分类和跨平台 wire authority。
- UniFFI 只导出粗粒度命令、稳定 DTO 和 typed error，持续避免逐帧调用。

### Swift Ownership

- SwiftUI 页面状态和 App/scene/orientation 生命周期。
- Keychain、UserDefaults、文件保护和本地缓存目录策略。
- libwebrtc、Metal、CVPixelBuffer、AVAudioSession、GameController 和 GCHaptics。
- 60Hz 输入编码热路径保留 Swift；Rust 生成共享 golden fixture，Swift 测试消费同一份权威字节。

### Swift Coordination Model

- 每个可变资源域只有一个生命周期 owner。
- 每次跨 actor/磁盘/网络 `await` 后，提交前验证 owner revision 和精确 scope。
- Task 必须由 owner 持有、取消并在清理边界按需等待；持久化写入携带 scope revision。
- Store 对外发布 UI 状态，纯状态机、持久化、access 和 cleanup 计划进入独立组件。

## Proposed Components

1. `AuthCredentialCoordinator`
   - 串行化 login refresh、cloud/home access preparation 和 sign-out。
   - 维护 credential revision，提供 compare-and-commit。
   - stale access 结果立即释放 handle；Keychain 删除失败进入显式可重试状态。
2. `CloudCatalogPersistenceCoordinator`
   - 持有 scope revision 和持久化任务。
   - 提供 load/save/clear 的条件提交；图片偏好只更新共享 base 数据。
   - 账号 overlay 文件名使用稳定指纹，并配置文件保护与 backup exclusion。
3. `XboxDataSurfaceState`
   - 将 host/library/achievement 的 phase、single-flight 和 activation key 从 Store 拆出。
   - achievement key 使用 `titleID + normalized locale`。
   - 明确主机、TitleHub 和成就的账号作用域磁盘快照合同。
4. `SettingsPersistence`
   - 使用 `SettingsSnapshotV1` 集中读写、归一化、迁移和默认值。
   - `AppSettingsStore` 保留 ObservableObject 和 UI binding。
5. `StreamingSessionState`
   - 用显式 readiness facts 替代分散布尔组合。
   - 拆出 `IceEpochCoordinator` 和 `SessionCleanupPlan`，`StreamSessionActor` 继续作为唯一 session owner。
   - cleanup 顺序固定为 renderer detach/drain、input/haptics、ICE/stats、peer/DataChannel、remote session、access。

## Plan

1. 修复认证 credential revision、access prepare 并发提交、sign-out/Keychain 一致性，并补延迟与失败测试。
2. 引入云目录 persistence coordinator，修复 load/save/image/clear 的跨 `await` 竞态和账号隔离。
3. 引入设置 snapshot/migration，修复 locale-aware achievement activation，并补浏览数据账号快照。
4. 重构串流 readiness、ICE epoch 和 cleanup plan，确保 renderer 在 terminal cleanup 起点 detach/drain。
5. 按职责拆分大型 Swift 文件，保持现有公共 Store API 和 SwiftUI 调用点稳定。
6. 建立跨 Swift/Rust golden fixture、完整 focused tests、构建/trace gate，并产出 Report。

## Validation

- [ ] delayed cloud/home access 与 sign-out/账号切换交错时，旧结果无法写入 Keychain 或 Store，access handle 全部释放。
- [ ] Keychain save/delete failure 产生一致、可重试的认证状态，重启后不会恢复已确认退出的账号。
- [ ] cache load/save/image update 与 clear/scope switch 任意交错时，旧 snapshot 无法回写内存或账号 overlay。
- [ ] 主机、TitleHub、成就快照按账号隔离；语言切换后 achievement surface 使用新 locale key。
- [x] 非法或旧版设置在加载时完成迁移、clamp 和回写，默认行为保持兼容。
- [ ] terminal 前 renderer 完成 detach/drain；terminal 后 `displayPresent`、frame submit 和旧 callback 均为零。
- [ ] `StreamSessionActor` 保持单 terminal、幂等 stop、stale callback rejection 和固定资源释放顺序。
- [ ] Swift/Rust input/rumble fixture 使用同一权威文件且字节一致。
- [ ] `cargo test -p xbox-ios-bridge`、streaming boundary、Swift parse、Swift 6 strict typecheck、focused XCTest 和 trace gate 通过。Rust、boundary、parse、生产子集 strict typecheck 与 XCTest strict typecheck 已通过；XCTest 运行和 iOS trace gate 待环境验收。
- [ ] fresh Simulator xCloud 与 Device xCloud/xHome 完成启动、播放、停止、返回竖屏和隐私扫描验收。

## Risks

- 认证与缓存改造触及 App 启动和退出主链，必须分阶段保留既有公共 API 并优先补确定性测试。
- Store 拆分过程中可能改变 SwiftUI objectWillChange 时序；对外 `@Published` 语义保持稳定并使用 focused UI-state tests 锁定。
- renderer drain 涉及 Metal command buffer 与 WebRTC track 生命周期，需由 trace 同时验证 submit/present 和资源终态。
- 浏览数据磁盘快照会增加 schema 和隐私责任，文件采用账号指纹、数据保护、backup exclusion 和版本化 DTO。

## Progress

- [x] Step 0: 完成 iOS 架构、存储、设置、串流和 Rust/Swift 边界代码评审。
- [x] Step 1: 固化目标边界、阶段顺序和验证矩阵。
- [x] Step 2: 用户确认 RFC，开始认证和缓存 P1 修复。
- [x] Step 3: 完成设置、locale-aware 数据 surface 与串流结构优化；账号浏览数据磁盘快照因缺少稳定非敏感 owner key 转为后续合同任务。
- [ ] Step 4: 完成构建、测试、trace 验收和 Report。静态/单元编译门禁已完成，Xcode XCTest、fresh trace 与真机验收待完成。

## Execution Notes

- Date: 2026-08-05 | Status: planned
- Update: 代码评审确认 Rust/Swift 技术边界合理，Swift 协调层的异步一致性和大型 owner 拆分进入独立优化任务。
- Decision: 采用分阶段原地收敛，保持现有 SwiftUI API、单一 libwebrtc runtime 和共享 Rust 领域主线；先修一致性，再拆职责。
- Risk/Blocker: RFC 等待用户确认执行；完整 Device xCloud/xHome 验收依赖真实设备和账号环境。

- Date: 2026-08-05 | Status: in-progress
- Update: 用户确认执行 RFC；实施按认证、云缓存、设置/数据 surface、串流生命周期四条独立边界推进。
- Decision: 子任务保持文件所有权隔离，主窗口统一负责 Xcode 工程接线、共享测试文件、构建门禁和最终收口。
- Risk/Blocker: 完整 Device xCloud/xHome 验收继续依赖真实设备和账号环境。

- Date: 2026-08-05 | Status: validation-pending
- Update: 已落地 `AuthCredentialCoordinator`、`SettingsPersistence`、`CloudCatalogPersistenceCoordinator`、scope lifecycle、diagnostics 和 `StreamingSessionState`，并完成 Xcode Sources 接线与 UniFFI bindings 更新。认证 mutation 采用 revision 串行提交与 typed retryable error；退出失败使用显式恢复动作。云缓存 scope 切换会取消并等待图片/刷新持久化任务，stale save 按账号条件清理 overlay。成就 surface 使用 `titleID + normalized locale`；串流 cleanup 起点同步 detach video sink，Store stop 同步清空 video track。
- Validation: `cargo test -p xbox-auth-flow` 1 项、`cargo test -p xbox-ios-bridge` 49 项、streaming session boundary、全量 Swift parse、PBX plist、`git diff --check`、认证/云缓存/设置/XboxData/串流状态 Swift 6 strict typecheck、新增 focused XCTest iPhoneOS Swift 6 typecheck、streaming trace gate analyzer 14 项和 iOS trace analyzer 3 项通过。
- Decision: 账号浏览数据磁盘 snapshot 等待 Rust/Auth 输出稳定非敏感 owner fingerprint；Swift/Rust input/rumble golden fixture 作为独立 wire-contract 任务处理，避免本次协调层重构扩大协议边界。
- Risk/Blocker: `xcodebuild` 需要用户级 SwiftPM/CoreSimulator 目录，自动审批服务返回 503；当前工作区与临时目录未发现 iOS runtime trace，fresh Simulator/Device xCloud/xHome、terminal 后 present=0 和返回竖屏仍需运行时验收。
