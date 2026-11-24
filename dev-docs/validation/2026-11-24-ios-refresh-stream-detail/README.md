# iOS 刷新、串流快捷入口与安装游戏详情优化

下拉刷新现在由原生控件统一反馈：分区、刷新按钮和分页共用当前页面的刷新范围，骨架在这一轮保持静态；首次加载、独立按钮操作与重试继续在对应位置反馈。

串流菜单与快捷布局入口在左上并排，两个图标各保留44pt点击区域和8pt间距，背景为12%黑色覆盖率；辅助功能降低透明度时使用82%覆盖率。菜单展开后的两条窄栏沿既有表面和操作方式显示。

安装游戏详情的名称、简介和商品图片复用目录市场/语言请求及缓存，成就与游玩事实沿个人记录取值。设置切换后按新范围重新加载。标题同行右侧采用44pt圆形主机串流图标，缺少titleID的安装条目也沿商品ID加载资料。

最终通过3项定向冒烟：原生控件触发刷新时，正文额外spinner由3个变为0个，结束后独立加载反馈恢复到3个；实际详情视图分别发起TW/zh-TW与US/en-US请求并保留个人成就数据；正式应用入口检查标题右侧单一44pt播放动作和Tab切换；左右横屏检查快捷图标并排、8pt间距与虚拟按钮避让。Swift按改动行定向格式化，任务范围diff检查通过。

| 证据 | 文件 |
| --- | --- |
| 下拉期间保留系统loading | [截图](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-refresh-stream-detail/refresh-system-only.png) |
| 目录中文配置的详情 | [截图](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-refresh-stream-detail/installed-detail-configured-chinese.png) |
| 正式入口标题与播放动作 | [截图](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-refresh-stream-detail/installed-detail-title-action.png) |
| 横屏左侧快捷入口 | [截图](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-refresh-stream-detail/streaming-quick-left.png) |
| 横屏右侧快捷入口 | [截图](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-refresh-stream-detail/streaming-quick-right.png) |
| 原生刷新数量与请求参数 | [摘要](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-refresh-stream-detail/native-refresh-locale-summary.txt) |
| 最终测试结果 | [3项通过](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-refresh-stream-detail/final-test-summary.json) |

联合验收中的两项UI检查一次通过。原生夹具起初等待了ScrollView已消费的refresh环境入口，清理宿主时出现idle→failed(deinit)；改为UIRefreshControl事件与可见状态读取后通过。两次夹具尝试的结果及最终日志保留在[诊断记录](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-refresh-stream-detail/failure-diagnostics.json)。本轮临时探针已[精确恢复](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-refresh-stream-detail/temporary-probe-restoration.json)到原有源码，既有工作区与暂存修改继续保留。

构建、测试和导出进程结束后，已清理本轮隔离目录、DerivedData、结果包、日志、重复截图和一次性格式配置，分配空间估算601.19MiB；清理后可用空间46.16GiB。保留5张必要截图、摘要、诊断与共享SourcePackages缓存，路径及统计见[清理记录](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-24-ios-refresh-stream-detail/cleanup.json)。

实机使用最新构建核对刷新反馈、串流快捷入口遮挡和安装游戏详情语言。
