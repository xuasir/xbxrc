# iOS 串流诊断与通用 Xbox 触屏布局验收

日期：2026-11-20。任务：`2026-11-20-ios-streaming-panels-touch-parity`。

## 实现

- 本地串流与云游戏使用 `TouchGamepadOverlay`，统一加载应用内 `generic.json` 通用 Xbox 布局，通过现有 `TouchLayoutSurface` 和 `TouchInputAccumulator` 绘制、处理多指并投影至原 `StreamInputSession`。此前正式串流覆盖层使用独立 classic 控件排布，造成了与标准布局的样式差异。
- 竖屏手柄视口使用下半区，横屏使用全区；Menu、View、Guide、肩键、扳机、摇杆和 ABXY 均随标准布局落在手柄视口内。空白触点继续交给视频手势，打开菜单、方向变化和关闭触屏会释放输入。
- 原功能菜单继续使用现有呈现方式；诊断入口加入快捷操作一级。诊断菜单及四组详情共享右上方局部浮层，宽度上限 420pt、竖屏高度约 46%、横屏高度约 80%，保持部分游戏画面可见。菜单和详情均提供关闭按钮，详情内部支持滚动。
- Guide 使用现有标准 `XboxLogo` 资源并复用符号缓存。顶部运行指标以及网络、显示、恢复、会话四组字段对齐 PC；接收、解码、提交、呈现帧率使用同一采样窗，近期丢包使用五秒窗口，首次、重连和累计计数回退重建基线。
- 显示配置来自实际源尺寸、输出尺寸、SR 和锐化设置；指标过期及平台缺乏实测的字段显示 `--`。复制诊断摘要按白名单输出四组 JSON。

## 验证范围

最终验证：iPhone 17e 的 83 项单元测试和 3 项 UI 测试通过；iPad Pro 11-inch (M5) 的 2 项定向 UI 测试通过。iPhoneOS 真机目标编译通过（`CODE_SIGNING_ALLOWED=NO`，最低 iOS 26.0）。Swift 格式检查、`pnpm lint:fix`、串流会话边界检查与 `git diff --check` 均通过。

测试概要见 `validation.json`、`phone-summary.json`、`ipad-summary.json`；截图及输入回放索引见 `manifest.json`，源码校验见 `source-sha256.json`，前轮修复诊断见 `failure-diagnostics.json`。

界面测试通过 DEBUG 参数 `--streaming-panels-ui-test` 装配正式覆盖层和诊断控件，使用黑色背景且没有真实远端会话；诊断读数截图来自可复现的合成指标夹具。横竖屏坐标、浮层范围、四组切换、滚动、复制、指标开关、关闭和输入释放由自动化验证。手机及 iPad 尺寸的宿主截图验证布局适配。

真实本地/云串流的首帧、动态读数、触控手感与持续播放由用户实机复测。

## 临时产物

本轮临时根为 `/private/tmp/xbxrc-ios-streaming-panels-20261120`，使用 `r1`、`r2`、`r3` 分轮隔离。测试结束及必要证据归档后清理本轮 DerivedData、xcresult、重复截图、日志等。共享 `target/ios`、`target/ios-webrtc/137.7151.04/SourcePackages` 及其他任务数据继续保留；清理记录见 `cleanup.json`。
