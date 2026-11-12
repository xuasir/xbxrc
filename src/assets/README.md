# 界面素材

| 素材 | 清晰度来源 |
| --- | --- |
| Xbox 手柄 | 微软官方透明正面图，1488×1032 RGBA；[来源与坐标](ctrl/README.md) |
| Series X / S 主机 | 微软官方2000px原图，1536×1536透明画布；[来源与处理](console/README.md) |
| 导航与串流图标 | SVG路径，统一显式尺寸、viewBox比例和无障碍装饰标记，适配任意DPR |
| 应用图标 | 用户原版，源文件及50张平台PNG、ICNS、ICO与提交 `e33f554a` 内容一致；[回退核验](../../dev-docs/validation/2026-11-12-hd-ui-assets/checks/app-icon-restore.json) |
| 背景 | 现有3840×1402原图 |
| 游戏封面等远程图 | `RemoteImage` 按CSS尺寸和最高DPR 3请求图片，按候选源回退 |

兼容原图与`iosapp`存量主机资源同步高清版本。存量iOS各色应用图标已有1024px RGB原图，启动和连接图标为SVG。

全量资源尺寸、哈希及界面证据见[高清素材验证记录](../../dev-docs/validation/2026-11-12-hd-ui-assets/README.md)。
