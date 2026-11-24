# 官方布局云目录 404 与重试样式修复

2026-11-24。云目录修正、绑定和最小冒烟完成，真实服务回包等待正常 Xcode 应用确认。关联 [原 RFC](../../../docs/rfcs/2026-11-23-ios-saved-official-touch-layouts.md)。

云布局目录和下载前的包归属校验统一使用 `POST /v2/titles`，请求字段为 `alternateIds` / `alternateIdType=productId`，读取 `details.supportedTabs`。标题详情同时匹配商品 ID 和 Xbox TitleId；外层 `titleId` 属于串流服务身份。`supportedTabs` 缺省、null 或空数组显示空目录，版本和包身份校验继续生效。

微软 `game-stream` 的 Cloud 连接把标题详情传入 TAK 配置，Home 的标题切换才调用 `/v2/titles/inputconfigs`。r1 核对了方法定义；用户重跑仍失败后，r2 补齐调用范围并修正接入。公开源码摘录见 [Cloud/Home 调用合同](cloud-catalog-contract.txt) 和 [来源指纹](cloud-catalog-source.json)，r1 的 [方法定义摘录](official-sdk-contract.txt) 保留供追溯。

查询、下载、预览复用现有 `SectionRequestFailure` 与 `AppSectionRequestState`；保存失败使用中文文案。原始枚举描述保留在诊断，页面显示统一提示和右侧重试。目录诊断记录 `titleIdentity` / `cloudTitleDetails` 阶段及类型化状态码。

- [用户 404 截图](user-404.png)
- [统一重试截图](ipad-shared-retry.png)
- [重试恢复预览](ipad-recovered.png)
- [5 项 Rust 请求合同](rust-contract-summary.txt)
- [4 项新增云目录回归](catalog-regression-summary.json)：嵌套身份、商品匹配、null 目录和版本/包身份校验。
- [1 条 iPad UI 冒烟](ui-smoke-summary.json)：20.867 秒，覆盖类型化 404、中文提示、共享重试和恢复预览。

r2 的 Swift/Rust 绑定生成和 iPhoneOS 编译通过。共享重试界面复用 r1 的模拟器/UI 证据。r1 Mac XCTest 在建立连接前被系统终止，实际服务请求数为 0；详情见 [真实请求诊断](live-request-diagnostics.json)。本次用户失败的 Mac 应用日志位于受系统保护的容器，当前仅有历史模拟器日志。正常 Xcode 应用内的预览/保存和 Home 实机体验继续待验。

[r2 差异](task-changes-r2.diff)、[r2 源指纹](source-hashes-r2.json)与[r2 编译摘要](device-build-r2.json)记录当前实现。[r1 差异](task-changes.diff)、[源指纹](source-hashes.json)和[临时源码恢复](temporary-source-restoration.json)保留。两轮临时根、DerivedData、结果包及一次性源码均按清单清理；共享 SDK、Rust 缓存与用户数据保留。清理量见 [r1 记录](cleanup.json) 和 [r2 记录](cleanup-r2.json)。

11-24 接续已完成官方短版本兼容、better-xcloud 社区来源、紧凑卡片与直接全屏预览；预览共用触屏调整页样式，返回和保存均完成本机保存后退出。[最新证据](community-and-cards/README.md)记录最终 1 条 UI 冒烟与此前 31 项 Rust / 6 项 Swift 身份检查。真实官方服务与 Home 实机体验继续待验。
