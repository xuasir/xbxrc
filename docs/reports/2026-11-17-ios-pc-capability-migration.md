# iOS / iPad PC 页面与数据迁移交付报告

## Summary

- Related RFC: [页面与数据能力迁移](../rfcs/2026-11-12-ios-ipad-pc-capability-migration.md)。
- 五菜单、手机原生子页、iPad 二级侧栏、PC 设置、个人与主机数据、云目录和串流启动前置流程完成。2026-11-17 在本地 main 工作区统一验证，真实媒体运行与硬件验收沿用户指定的后续阶段。

## Delivered

- 主机设备事实、安装库排序/存储筛选及完整游戏四分区详情；我的账户、好友与目标玩家、历史/成就、时长和相册分页。
- 云目录最近/新增/热门/即将离库/全部/收藏，搜索筛选、持久收藏与置顶、商品资料、语言支持、无障碍事实及截图预览。
- 设置、控制器图示录入/震动与触屏操作逐项对齐 PC，范围、默认值、生效时机和平台差异见[设置报告](2026-11-16-ios-settings-parity.md)。
- 串流启动资格与身份复核、唤醒/创建/排队进度、取消与晚到响应隔离；沿原有共享 Rust session、libwebrtc 与 Metal。

## Changes

- 页面刷新保留成功数据，查询以账户、语言和请求代次隔离；双方成就独立分页并按服务配置及成就 ID 合并。
- 五个一级页面由原生 TabView 管理可见窗口和无障碍归属，保留各自 NavigationStack、筛选与返回位置。
- iPad 侧栏与 NavigationStack 使用 HStack 实际分配宽度，覆盖主机、云目录和公共二级分类；工具栏与内容点击落在右侧真实区域。
- 截图预览独立关闭标识，英文动态组合与无障碍同步，设置/控制器交互覆盖无键盘场景。

## Validation

| 验证 | 结果与证据 |
| --- | --- |
| Rust bridge | 54 项通过；/tmp/xbxrc-ios-migration-delivery-rust.log |
| iPhone | 169 单元、9 UI 通过，iPad 专用 1 项跳过；/tmp/xbxrc-ios-migration-resume-phone.log 与同名 xcresult |
| iPad | 169 单元、5 页面 UI 通过；/tmp/xbxrc-ios-migration-resume-ipad-3.log；5 设置 UI 通过，/tmp/xbxrc-ios-migration-resume-ipad-2.log |
| 设备编译 | iPhoneOS 完整与最终增量构建通过，Rust/UniFFI/WebRTC/设备 MetalFX 装配完成；/tmp/xbxrc-ios-migration-resume-device-final.log |
| 视觉 | 两端各 200 张尺寸/主题/字号布局截图；代表图及 manifest 见[视觉证据](../../dev-docs/validation/2026-11-17-ios-pc-capability-migration/README.md) |
| 静态检查 | Rust fmt、66 个迁移 Swift 文件格式检查、pnpm lint:fix、streaming session boundary 和 git diff --check 通过 |

UI 覆盖安装库存储筛选返回、完整游戏详情、好友秘密成就与条件、云收藏和截图关闭、应用语言即时生效、时长格式、宽屏分类切换恢复、设置整行边缘点击、底栏与末行空间、图示震动和页内录入，全程键盘数量为 0。Swift 格式检查保留两条循环书写建议，实际返回码为 0。

## Risks

- Xbox 真实账户的资格、隐私、主机电源和媒体有效期影响返回内容；页面保留真实错误和缺值。
- 启动前置 HTTP 和状态机已验证；真实 xCloud/xHome 音视频、实体输入/触觉、MetalFX 观感与功耗沿仓库约定由用户实机验收。
- 本地源码验证完成，修改保留于当前工作区；main 开发构建入口沿既有暂停规则。

## Follow-up

- 按用户 2026-11-17 要求继续调整信息密集页面：我的/账户与安装库采用身份或主机背景通顶，指标分组，明确主动作与辅助事实；同步排查玩家详情、个人游戏详情与统计摘要。
- 实机 xCloud/xHome 验收使用 fresh trace，同时检查网络、恢复链、media supply、steady supply、submit/present 长尾和帧供给 delta。
