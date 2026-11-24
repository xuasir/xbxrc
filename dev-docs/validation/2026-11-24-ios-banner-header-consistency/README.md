# iOS 通顶背景与游戏详情头部验收

2026-11-24。页面、状态栏和导航头部共用 `AppPageBackdrop`，按完整窗口坐标绘制并裁切通顶媒体背景。统一 380pt 高度与渐隐规则，浅色顶部画布覆盖率调整为 52%，中段为 70%；头像与游戏图片复用现有缓存，导航背景归当前活动返回栈的顶部页面所有。

个人游戏详情按“游戏封面、名称与记录 → 内容 Tab → 正文”排列。四个分类共用身份区，切换只对正文执行过渡；短窗和辅助字号收起额外留白，并使用 56pt 封面。游戏背景优先使用宽幅 hero，刷新时保留进入页面时已知的图片来源。

| 验收 | 结果 | 证据 |
| --- | --- | --- |
| 通顶背景与深浅色透出 | 通过；头像、个人游戏、云游戏、主机使用同一绘制组件与窗口裁切规则 | [玩家](banner-profile-dark.png)、[游戏深色](banner-game-phone-dark.png)、[游戏浅色](banner-game-phone-light.png)、[云游戏](banner-cloud-dark.png)、[主机](banner-console-dark.png) |
| 导航头部归属 | 通过；进入游戏后使用游戏背景，后台玩家图片更新保持当前游戏头部，返回后使用更新后的玩家图片 | [返回玩家](banner-profile-returned-dark.png)、[最终原生用例](final-test-summary.json) |
| 游戏身份区与四个内容分类 | 通过；名称位于 Tab 上方，概览、成就、相册、统计切换后身份区与 Tab 坐标稳定；相册类型切换、空态居中与返回安装库通过 | [概览](priority-game-overview.png)、[相册](priority-game-album-empty.png)、[交互用例来源](initial-test-summary.json) |
| 短窗布局 | 通过；横屏使用紧凑身份区，Tab 与正文保持可达 | [横屏](banner-game-landscape-dark.png) |
| 编译与源码检查 | 通过；Xcode27、iOS27、arm64无签名Simulator构建，最低部署保持iOS26；范围格式化与差异检查完成 | [构建摘要](build-test-excerpts.txt)、[格式化](swift-format.json)、[最终格式化](final-swift-format.json) |

[验收汇总](validation.json)合并两项唯一通过用例：交互用例采用首轮结果，背景状态与原生绘制采用最终结果。原生背景 PNG 使用本轮固定颜色与定位条纹，直接注入图片缓存，覆盖生产图片组件成功绘制路径；真实交互截图使用既有 Xbox 固定验收数据。

首轮探针捕获原生进入转场期间的头部衔接。标题背景改用窗口挂载、活动状态与顶部页面归属判定，采样等待原生转场完成，最终状态检查通过。[失败截图](initial-navigation-transition.png)与[诊断记录](failure-diagnostics.json)保留修正来源。首轮测试结束后，Xcode自动模拟器诊断打包进入600秒等待；精确停止本轮诊断子进程后结果包正常收尾，最终定向验证使用 `-collect-test-diagnostics never`。

[任务差异](task-changes.diff)、[源哈希](source-hashes.json)与[原生绘制探针](native-banner-smoke.swift)可用于复核。[恢复记录](temporary-source-restoration.json)确认临时测试源码恢复到任务开始时的原件，交互测试保留身份区与 Tab 位置断言。

[清理记录](cleanup.json)确认本轮构建、测试、导出和诊断进程已结束，临时目录、DerivedData、结果包、日志与重复导出清理625.71MiB。保留9张最终截图、1张失败诊断截图及摘要、差异、哈希和复现探针；既有WebRTC、Rust缓存与模拟器继续复用。

实机重点复核玩家 → 游戏 → 返回时的背景衔接，以及游戏详情四个 Tab 的切换观感。
