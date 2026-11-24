# iOS 相册与串流局部浮层验收

2026-11-24 完成引用聊天的原始需求与追加批注，当前任务由主窗口直接执行。

## 最终行为

- 保存媒体弹出约三分之一屏的下载浮窗，显示真实字节、进度条和百分比；右上角提供取消，下载期间通过浮窗保护页面返回与下拖交互。下载沿用共享 `AlbumFileDownload` 原生代理实现，分享结束后清理导出文件。
- 视频按实际画面比例嵌入详情页，竖屏预览左右贴边，保留应用主题背景；播放控制、全屏进入与退出由 `AVPlayerViewController` 提供。详情底部沿浮动导航栏 inset 避让，失败提示与重试保留完整空间。
- 画面调节以半屏浮层贴住屏幕底部；快捷窄栏、触屏布局选择限制为半屏高度。
- 诊断浮窗保持右上位置，横竖屏高度上限为可用区域的三分之二；说明文字进入紧凑布局的滚动区，关闭操作保持可达。

## 最小验证

Simulator 为 iPhone 18 Pro，iOS 27.0。相册定向 UI 冒烟通过，耗时 62.413 秒；覆盖实际进度增长、右上取消、手势保护、视频贴边和 16:9 比例、原生全屏进入与返回。串流定向 UI 冒烟通过，耗时 20.290 秒；覆盖四类浮层尺寸、画面贴底、横竖屏诊断位置及关闭操作。

两个结果取自各自最终通过的测试记录。串流所用 `completed-smoke` 合并轮次曾出现相册全屏断言失败，随后相册在 `media-verified` 独立轮次通过；详情见 [测试摘要](test-summary.json) 与 [日志摘录](build-test-excerpts.txt)。

续接轮次读取既有通过结果，完成正式源码恢复、Swift 语法解析、定向格式与 diff 检查；见 [验收记录](validation.json)、[格式结果](swift-format.json) 和 [源码哈希](source-hashes.json)。真实 Xbox 媒体分享保存与实机串流由用户验收。

## 最终截图

- [下载进度与右上取消](media-download-half-progress.png)
- [应用内贴边视频](media-video-inline-theme.png) · [原生全屏](media-video-native-fullscreen.png)
- [快捷窄栏](stream-half-rails-landscape.png) · [贴底画面调节](stream-half-picture-landscape.png) · [触屏布局选择](stream-half-layout-landscape.png)
- [横屏诊断](stream-half-diagnostics-landscape.png) · [竖屏诊断](stream-half-diagnostics-portrait.png)

## 诊断与清理

保留一张进度停滞的历史诊断图和必要失败摘要；[修复记录](failure-resolution.json) 对应最终通过状态。[归档 UI 探针](ui-smoke-probe.swift) 与 [夹具服务器源码](fixture-server.py) 作为验收证据留存。

四处临时源码已恢复，包括相册夹具、两条临时 UI 测试及下载 URL 校验。正式下载入口保持 HTTPS，详情见 [恢复记录](temporary-source-restoration.json)。

核对相关进程结束后，已清理 `/private/tmp/xbxrc-ios-media-half-panels-20261124-a`，覆盖全部 `.xcresult`、临时 DerivedData、重复导出截图、夹具、日志和一次性脚本，释放约 1333.97 MiB。清理后目录移除，磁盘可用约 45.01 GiB；见 [进程检查](process-check.json)、[产物清单](artifact-manifest.json) 和 [清理记录](cleanup.json)。
