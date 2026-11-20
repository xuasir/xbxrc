# iOS 十一张图片全局布局修订交付

## 范围与行为

关联聊天最新一轮的 11 张图片是本次最终范围，编号和文件 SHA-256 见[原图清单](../../dev-docs/validation/2026-11-17-ios-spacing-review/source-manifest.json)。对应连接测试、菜单操作、震动、游戏输入、账户、游玩统计、玩家详情、游戏概览、相册、安装库与主机；共性调整作用于公共组件和相关页面。

页面边距统一 16pt；信息细节 4pt，同组条目 8pt，标题与内容/工具组 12pt，独立分区 32pt；媒体集合维持16pt列间距和24pt行间距。设置选项保留12pt行间隔及44pt触屏目标，页尾保留内容留白与壳层底栏区域。

只读设备事实合并为紧凑组，说明、诊断状态、版本、TURN事实采用透明信息行；设置交互继续有独立表面。手柄图保持比例、居中并限宽440pt，映射字段与说明分别排布。我的、游戏详情和媒体类型共用透明文字Tab；搜索/排序/筛选按宽度同行或左对齐换行。主机选择、信息、安装入口与圆形动作连续排布。

相册日期、录制片段时长、个人游戏/安装时间和诊断记录日期按应用locale格式化；控制器与主机只读事实通过可本地化的值呈现。英文相册采用 `30 s` 与英文月份，应用界面语言与真实玩家/游戏名称分别保留各自来源。

## 实现与合同

公共令牌、信息分区、局部工具和分类位于 `AdaptiveSectionNavigation.swift` 与 `AppInformationPage.swift`，设置、控制器、个人与主机页面沿原有状态和导航绑定装配；[移动端风格合同](../../iosapp/DESIGN.md)同步更新。

我的保留原生左右分页、吸顶和详情返回栈。吸顶头部明确交互优先级；UI回归通过可见触屏中心点击并核对选中状态、好友详情返回和左右分页，覆盖实际触控行为。

## 验证

| 验证 | 最终结果与证据 |
| --- | --- |
| 原图数目 | 11 个独立文件存在，11 个 SHA-256 与原图清单匹配。 |
| 手机交互 | 13 项适用 UI 通过，2 项宽屏专用检查跳过。 |
| iPad 交互 | 15 项 UI 通过，包含宽屏尺寸、侧栏和分类切换。 |
| 语言复验 | 两端各 2 项通过，覆盖英文日期、`30 s`、控制器只读状态与单位。 |
| 最终布局 | 两端各 176 张：11 页 × 8 种尺寸/主题/字号 × 中英；总计 352 张。最终 `AppMetricGrid` 最小列宽为 132pt，宽屏六项指标连续同行。 |
| iPhoneOS | 最终源码构建通过：`/tmp/xbxrc-ios-spacing-complete-device.log`。 |
| 静态检查 | Swift 格式、`pnpm lint:fix`、`cargo fmt --all --check`、streaming session boundary 与暂存/未暂存 diff 检查通过。 |

交互结果按各项最后一次回归汇总，逐项状态见[交互清单](../../dev-docs/validation/2026-11-17-ios-spacing-review/interaction-results.json)，日志来源与校验信息见[验证清单](../../dev-docs/validation/2026-11-17-ios-spacing-review/validation.json)。最终布局结果包为 `/tmp/xbxrc-ios-spacing-complete-phone.xcresult` 和 `/tmp/xbxrc-ios-spacing-complete-ipad.xcresult`；对应日志均为 `TEST EXECUTE SUCCEEDED`。

[视觉目录](../../dev-docs/validation/2026-11-17-ios-spacing-review/README.md)当轮归档55张代表PNG；2026-11-18精简后保留[十一页预览](../../dev-docs/validation/2026-11-17-ios-spacing-review/preview.jpg)、原图来源和交互/构建摘要。已查看总览、窄屏英文相册/控制器及宽屏账户/统计，核对间距层级、透明分类、英文日期与秒单位和六项指标排列。截图采用确定性测试数据，图片加载回退也在画布中呈现。

## 交付边界

源码保留于当前工作区。真实账户串流、实体手柄和触觉沿既有迁移报告由用户实机验收；本次验证集中于页面、交互、布局和设备编译。
