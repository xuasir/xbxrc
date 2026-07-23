# iOS 数据请求、缓存与重载策略审计 Report

## Summary

- Related RFC: [`docs/rfcs/2026-07-22-ios-data-cache-reload-audit.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-07-22-ios-data-cache-reload-audit.md)
- 本轮完成 iOS 主机、游戏库、成就、账户、设置、详情页和共享图片组件的代码级请求/缓存/生命周期审计，定位用户描述的重复请求与图片重新进入 loading 状态的主要触发链。

## Delivered

- 建立七类请求矩阵：主机、游戏库目录、TitleHub/时长、单游戏成就、账户资料、设置、远程图片。
- 固定十条目标行为合同，覆盖 Tab 往返、Scene active、Token 续期、single-flight、stale-while-revalidate 和图片复用。
- 整理后续 XCTest、Runtime Trace 和真机复现门禁，保持 Tauri + Vue 3 + TypeScript + Rust 技术边界不变。

## Changes

- 代码级根因 1：Scene 每次进入 active 都调用 Profile refresh，缺少 freshness window 和 single-flight。
- 代码级根因 2：云目录页面任务取消时，Store 内部刷新任务与 UI 生命周期耦合，结果存在被外层取消丢弃的窗口。
- 代码级根因 3：Cloud Access 更新 session 后触发 `XboxDataStore.sync`，造成主机和 TitleHub 重新加载。
- 代码级根因 4：`AsyncImage` 缺少应用级图片字节缓存与 URL 请求合并；详情背景/Hero、账户头像/背景存在同 URL 多消费者。
- 代码级根因 5：候选 URL 变化和 `.id(currentURL)` 会重建图片加载状态，首批目录、metadata hydration 和活动合并可能让已显示图片再次进入 loading。

## Validation

- 已检查 `XBXRCApp.swift`、`AppRootView.swift`、五个 Tab、`GameDetailView.swift`、`XboxDataStore.swift`、`CloudLibraryStore.swift`、`CloudCatalogSnapshotRepository.swift`、`CircularCardCarousel.swift` 和已有 `XBXRCTests.swift`。
- 已运行 `git diff --check -- docs/project-task.md docs/rfcs/2026-07-22-ios-data-cache-reload-audit.md docs/reports/2026-07-22-ios-data-cache-reload-audit.md`。
- 当前工作区未发现 iOS 真机 Runtime Trace JSONL；桌面 trace 与 Rust 测试 trace 不足以证明 iOS 现场请求次数。

## Risks

- `AsyncImage` 的内部缓存行为受 URLCache 配置和服务端缓存头影响，UI `.empty` 事件不能单独证明发生了网络下载。
- 前台刷新节流、图片磁盘缓存容量和 Catalog stale-while-revalidate 需要真实账号与真机数据确认。

## Follow-up

- 后续实现方向已在 [`2026-07-22-ios-lazy-data-refresh.md`](../rfcs/2026-07-22-ios-lazy-data-refresh.md) 固定为首次进入“快照优先 + 后台刷新一次”，后续由用户手动刷新。

- 实现 Store owner generation、Profile freshness window、TitleHub/成就 single-flight，以及 UI observer 与 Store 请求生命周期分离。
- 实现共享图片 loader，补齐内存/磁盘命中、URL 合并、失败退避、候选替换和背景/前景共享测试。
- 从设置页导出 iOS Runtime Trace，复现“启动 -> 五 Tab 往返 -> 前后台 -> 详情进出”，验证请求原因和图片网络事实。
