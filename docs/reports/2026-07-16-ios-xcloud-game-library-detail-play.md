# iOS xCloud 游戏库数据对齐、游戏详情与 Play 入口 Report

## Summary

- Related RFC: [`docs/rfcs/2026-07-16-ios-xcloud-game-library-detail-play.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-07-16-ios-xcloud-game-library-detail-play.md)
- iOS 游戏库已切换到与 Tauri Game Pass/xCloud 栏目一致的官方可串流目录主线，并交付全屏游戏详情页、圆弧数据卡与固定 Play 入口。
- 同构骨架、分层缓存、显式刷新、图片多级回退和 1000 项目录渐进呈现已纳入实现与验证。

## Delivered

- 新增共享 Rust `xbox-cloud-catalog-flow`，统一 xCloud live titles、MRU、Game Pass SIGL 与 products hydration 的目录组装和三身份合同。
- Tauri `XcloudService` 回接共享核心，保持 RPC、事件、SWR、TTL、single-flight 与现有前端字段兼容。
- iOS bridge 新增 cloud access 内存 handle、目录首批加载和 75 项 metadata 分页 hydration。
- iOS 新增 `CloudLibraryStore` 与版本化快照仓库，base 跨账号复用 7 天，overlay 使用 10 分钟 fresh 与 24 小时可渲染窗口。
- 游戏库 Hero、Shelf 与完整栏目列表统一进入 `GameDetailView`；详情页包含通顶主题图、圆弧数据卡、描述和固定游玩按钮。
- Hero、Poster、Tile、账号 Artwork 与主题占位组成图片回退链，最近成功图片 URL 写入 base 缓存。

## Changes

- 热缓存直接展示；stale 缓存保持稳定，用户下拉刷新和显式重试承担强制更新入口；同内容快照跳过重复发布。
- 首轮只等待 75 项 metadata，剩余目录按 75 项增量合并；首页 Hero 5 项、Shelf 8 项、完整栏目每次扩展 24 项。
- 加载骨架复用 `LibraryLayoutMetrics` 的 Hero、栏目标题和卡片尺寸，保持真实内容切换时的几何稳定。
- `productId` 负责目录与缓存，`streamTitleId` 负责 Play，`xboxTitleId` 负责合并 TitleHub 活动、时长和成就。
- 缓存测试覆盖 base/overlay 完整往返与账号退出清 overlay、保留共享 base。

## Validation

- `cargo test -p xbox-webapi -p xbox-cloud-catalog-flow -p xbox-ios-bridge`：36 项通过。
- `cargo check -p xbxrc`：通过。
- `cargo fmt --all -- --check`：通过。
- iOS Device `build-for-testing`：App 与 XCTest target 完成 Swift 6、UniFFI、Rust 静态库编译链接，结果 `TEST BUILD SUCCEEDED`。
- XCTest 源码覆盖缓存 TTL、图片候选顺序、1000 项目录窗口、刷新 single-flight、磁盘 base/overlay round-trip 和退出清理边界。
- 本机 Tauri `settings.json` 匿名抽样：目录 1888 项；名称、productId、streamTitleId、xboxTitleId、分类、输入能力和 entitlement 覆盖 1888/1888；Tile 1885/1888，Poster 1887/1888；前 20 项图片地址协议结构全部有效。
- 工作树 `git diff --check`：通过。

## Risks

- CoreSimulatorService 当前不可用，Simulator XCTest 实际运行与完整 Asset Catalog 构建保留到正常 Xcode 会话。
- 当前网络审批服务故障阻止 20 项图片 URL 在线 HTTP 可达性验证；本地字段覆盖、URL 规范化与多级回退已通过。
- Play 入口已使用 `streamTitleId` 并校验 entitlement；真实 xCloud session provisioning、libwebrtc 媒体、音频、画面和手柄闭环属于独立串流阶段。

## Follow-up

- CoreSimulatorService 恢复后执行 Simulator XCTest 和完整 Asset Catalog 构建，并采集游戏库首屏与详情页真机截图。
- 使用同一账号、region、market 与 language 对比 Tauri/iOS product 集合，并执行至少 20 项图片 HTTP 可达性抽样。
- 串流阶段将详情页 Play 回调接入 `StreamingRuntime` 的 cloud launch request 与连接状态机。
