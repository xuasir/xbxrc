# 官方游戏触屏布局保存与实时切换验收

2026-11-24：功能实现与自动冒烟完成，应用内真实官方包和 Home 实机体验待验。关联 [RFC](../../../docs/rfcs/2026-11-23-ios-saved-official-touch-layouts.md)。

## 已交付

- 云游戏详情查询、选择、预览并保存官方布局；本机布局库按游戏分组管理。
- 删除单个布局回收编译快照，最后一个共享引用删除后回收展开文件、图片及包原件；重新读取布局库会重试清理。
- 串流菜单统一选择标准、自定义模板与已保存游戏布局；页面单按钮循环切换标准与最近布局。
- 布局替换沿现有触屏输入链释放多指、按键、摇杆、扳机、toggle/sticky 和 layer。最近布局跨启动保存。
- 删除按钮位于行尾，默认标记位于标题右侧。首次读取本机库成功前保留最近记录；读取失败保持记录，成功空列表确认缺失后清除。

## 验证证据

| 验证 | 结果 | 证据 |
| --- | --- | --- |
| 包解析与公共 API | 12 + 75 项通过，复用上轮结果 | [前轮摘要](previous-validation.json) |
| 本机原子保存、更新、删除与回收 | 12 项通过，复用上轮结果 | [前轮摘要](previous-validation.json) |
| 本轮 Swift 定向命令 | 60 项通过，含首次读取保护的 2 项新增回归 | [摘要](unit-final-summary.json) |
| 手机完整入口流程 | 3 项通过，复用 r2 结果 | [摘要](phone-final-summary.json) |
| iPad 完整入口流程 | 3 项通过 | [摘要](ipad-final-summary.json) |
| iPhoneOS 编译 | 通过，最低 iOS 26.0 | [编译摘要](device-build-summary.json) |
| Rust 格式与前端 lint | 格式检查通过；前轮 `cargo fmt`、`pnpm lint:fix` 已通过 | [总记录](validation.json) |

用户要求收尾采用最小冒烟。本轮复用已通过的界面和 Rust 结果，新增启动修复使用两个定向回归与一次设备编译确认；后续实际包验收采用一份应用内保存的官方布局。界面流程使用测试客户端和构造布局；实际包来源单独记录。

## 真实包与实机待验

用户已在 Xcode 的 iPad 模拟器内登录。临时命令行程序访问 Keychain 返回 `-34018`，表示缺少应用 entitlement；自动审批随后拒绝向该程序复制应用签名身份与 Keychain 访问组，拒绝理由为跨越会话凭据的访问边界。下一步由用户在已登录应用内正常保存一份官方布局，验证只读取 `Library/Application Support/TouchLayouts` 中的布局资源。

Home 采用包中的默认 Context 和手动布局选择。实际游戏动作、手感与连续串流沿仓库约定由用户验收；需要分析串流健康时，同时覆盖网络、恢复链、media/steady supply、submit/present 长尾和帧供给 delta。

## 归档与清理

- 14 张最终截图与 1 张修改前定位图：[图片清单](retained-images.json)。
- 失败定位及审批结果：[诊断](failure-diagnostics.json)；实际包尝试：[状态](live-smoke-summary.json)。
- 47 份源文件指纹：[源指纹](source-hashes.json)；既有 Swift 接线基线差异：[差异](swift-integration-changes.diff)。
- 本任务临时根：`/private/tmp/xbxrc-ios-saved-official-touch-layouts-20261124`。清理结果见 [清理记录](cleanup.json)。共享 SDK、Rust 缓存与用户本机布局库持续保留。

2026-11-24 404 反馈修复：商品身份请求已按官方 SDK 对齐，失败提示和重试已统一；5 项 Rust 和 1 条 UI 冒烟通过。Mac 应用编译成功，真实请求测试在 XCTest 建立连接前被系统终止，服务回包继续待正常应用内验收。[本轮证据](../2026-11-24-ios-official-touch-layouts-404/README.md)。
