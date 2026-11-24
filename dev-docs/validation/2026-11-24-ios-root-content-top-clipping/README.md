# iOS 主页面顶部裁切修复

切换主页面后，自定义导航背景从真实的116pt范围拉高到168pt，多出的52pt覆盖了从140pt开始的首行文字。诊断截图中，控制器名称和“应用偏好”的元素坐标完整，实际文字被背景遮住。

背景现在由独立原生容器持有 SwiftUI，四边约束到完整导航顶边、导航栏底边及左右边缘；宿主关闭安全区扩展，背景按真实窗口坐标取样。原有通顶透出规则及游戏身份区下方 Tab 沿用上一轮合同。

最终验收通过两项 Simulator 定向冒烟：原生探针依次检查五页的首次布局、背景边界和首行绘制；正式应用入口切换五页并返回我的、主机，检查首行实际像素、Xbox 标志、标题及底栏几何。导航栏保持54pt，底边116pt；控制器和设置首行从140pt开始完整显示。Swift 格式和任务范围 diff 检查通过。

| 页面 | 最终截图 |
| --- | --- |
| 主机 | [查看](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-root-content-top-clipping/final-hosts.png) |
| 云游戏 | [查看](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-root-content-top-clipping/final-library.png) |
| 我的 | [查看](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-root-content-top-clipping/final-my.png) |
| 控制器 | [查看](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-root-content-top-clipping/final-controller.png) |
| 设置 | [查看](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-root-content-top-clipping/final-settings.png) |

诊断保留 [控制器首行缺失](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-root-content-top-clipping/diagnostic-controller-missing-title.png) 和 [设置首行缺失](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-root-content-top-clipping/diagnostic-settings-missing-title.png)。[原生几何](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-root-content-top-clipping/final-native-geometry.json)、[正式入口几何](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-root-content-top-clipping/final-ui-geometry.txt)、[最终测试摘要](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-root-content-top-clipping/final-test-summary.json)及[失败诊断](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-root-content-top-clipping/diagnostic-attempts.json)保留可复查证据。

临时探针已精确恢复到任务开始的源码，原有工作区改动按基线哈希保留。本轮隔离目录及结果包、DerivedData、日志、重复截图和一次性配置已清理，临时产物占用估算为922.39MiB；清理后可用磁盘46.54GiB。共享 SourcePackages 保留，路径和统计见 [清理记录](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-root-content-top-clipping/cleanup.json)。

实机验收使用最新构建，检查五页切换与游戏详情返回。
