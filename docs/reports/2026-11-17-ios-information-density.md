# iOS 宽屏信息密度与透明吸顶交付

## Result

按用户最新要求，Tab常态和吸顶均透明，与页面大背景融合；纵向滚动保持吸顶，左右切换保留各页状态。正文按共同滚动坐标裁剪在Tab下面，顶部导航透明并使用同一大背景。

## Changes

- 参考PC `library.scss`和`AccountPanel.vue`的透明资料/人员行、局部工具区，结合iOS公共字号和44pt触屏合同。
- 资料刷新、记录入口、重试等信息动作按自然宽度展示；好友采用40pt头像、紧凑名称/状态/活动、横向Gamerscore，信息行高度通过小于88pt的宽屏验证。
- 搜索入口为透明图标与文字；展开输入限宽400pt，排序/存储/相册筛选采用局部Menu/Picker。
- 云详情商品、语言和无障碍信息采用透明语义分区；游玩按钮至少112×44pt，等待/权限说明相邻，局部底部薄面板限宽480pt。
- 个人/云游戏进度标签与云详情日期按应用语言格式化。媒体封面、相册比例与原数据/认证/导航/串流合同沿现有装配。

## Validation

| 验证 | 结果 / 证据 |
| --- | --- |
| 页面完整回归 | 手机8项普通流程通过；iPad10项通过，包含宽屏按钮/人员行尺寸验证。/tmp/xbxrc-ios-density-phone.log、density-ipad.log |
| 最终透明吸顶 | 手机3项、iPad浅色4项、暗色2项增量UI通过；覆盖分页、好友详情返回、相册返回、英文显示和宽屏尺寸。/tmp/xbxrc-ios-density-transparent-phone.log、transparent-ipad-light.log、transparent-ipad-dark.log |
| 双语布局 | 每端84张：7页面×6尺寸/主题/字号×中英文；最终transparent-phone与transparent-ipad-light结果包 |
| iPhoneOS | 用户最新透明吸顶与语言资源构建通过；/tmp/xbxrc-ios-density-transparent-device.log |
| 静态检查 | Swift格式、pnpm lint:fix、streaming session boundary、暂存/未暂存diff检查通过 |

[视觉证据](../../dev-docs/validation/2026-11-17-ios-information-density/README.md)当轮归档63张代表PNG和三个对照预览，已校正原生宽屏截图方向；2026-11-18精简后保留三个预览与本报告的测试摘要。测试iPad外观恢复为浅色。实机账户、串流、实体输入与触觉沿原[迁移报告](2026-11-17-ios-pc-capability-migration.md)验收。
