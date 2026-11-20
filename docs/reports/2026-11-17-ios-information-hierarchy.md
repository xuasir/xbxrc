# iOS 信息页面层级与图片批注交付报告

## Summary

按用户要求完成PC能力迁移后，完成信息页面层级调整及第二轮10条图片批注。后续宽屏表面/密度调整记录于[密度RFC](../rfcs/2026-11-17-ios-information-density.md)。

## Delivered

- 安装库使用同一个随内容滚动的上半区，主机图下对齐并留顶部空白；身份与56pt圆形串流浮钮横向组合，存储摘要处于上半区底部。
- 下半区从搜索、排序和存储筛选开始；数量、更新时间、加载状态合成一行小提示，完整错误信息可展开。搜索清除和筛选返回保留原数据状态。
- 我的顶部采用纯文字标题，身份区保留72pt头像，成就点数/好友/关注/粉丝合为一排；长标签和大字号保留完整值，必要时横向滚动。
- 账户、好友、成就、游玩统计、相册在下半区以原生分页切换，横向Tab随纵向滚动吸附在工具栏下；每页保留筛选与内容状态，详情沿同一NavigationStack返回。
- 信息页统一使用24/18/14/12pt及large至xxxLarge系统缩放范围，窄屏通过布局适配。玩家、游戏详情、统计和云详情同步采用公共字号。
- 无障碍父容器使用contain分组，保留子按钮名称、值与标识；装饰图片退出无障碍树和点击处理。

## Validation

| 验证 | 结果 / 证据 |
| --- | --- |
| 手机页面交互 | 8项通过，宽屏专用项跳过；英文、云截图收藏、好友秘密成就、安装库分区/筛选/搜索清除、吸顶分页/详情返回/导航恢复、相册返回。以information-review-phone-2、phone-final及album-phone-2日志合并最终通过证据 |
| iPad页面交互 | 9项通过；information-review-ipad、ipad-2及album-ipad日志，包含宽屏My分页与其他页面侧栏 |
| 双语布局 | 每端72张，6页面×6尺寸/主题/字号×中英文；information-review-phone-final和information-review-ipad-2结果包 |
| iPhoneOS | 最终设备构建通过；/tmp/xbxrc-ios-information-review-device-final.log |
| 静态检查 | Swift格式、pnpm lint:fix、streaming session boundary、暂存和未暂存diff检查通过 |

各日志与xcresult位于/tmp/xbxrc-ios-information-review-*。前轮截图沿[信息页面证据](../../dev-docs/validation/2026-11-17-ios-information-hierarchy/README.md)保留；最新批注截图已导出为/tmp/xbxrc-ios-information-review-phone-final-attachments，最终宽屏风格将另存密度报告与对照预览。

## Boundaries

本轮保持原数据、认证、导航与串流运行时边界。真实账户图片、xCloud/xHome串流、实体手柄和触觉沿[迁移报告](2026-11-17-ios-pc-capability-migration.md)实机验收。
