# iOS 触控布局与输入验证

本轮交付通用/Forza JSON 归一化、共享控件渲染与多指输入、本地能力报告，以及透明底、细描边的矢量控件。入口为“控制器 → 触屏手柄 → 布局与输入测试”。

- 临时根目录：`/private/tmp/xbxrc-ios-touch-layout-20261118/r1`。
- 复用现有 WebRTC SourcePackages；Xcode 更新其工作区元数据，已有依赖缓存保留。模拟器及设备 DerivedData 使用本轮目录。
- [测试摘要](test-summary.json)：18 项纯 Swift 输入/布局回放，手机与 iPad 实际入口 UI 测试，iOS 设备目标编译，Swift format、工程 plist 和会话边界检查均通过。
- Forza/Halo 随包 JSON 与 2026-11-12 冻结原件逐字节一致；[资源说明](../../../iosapp/XBXRC/Resources/TouchLayouts/SOURCES.md)记录来源。
- [诊断](diagnostics.md)保留编译、方向等待、cluster、符号及截图修正记录。
- [透明控件层](generic-transparent-surface.png)按透明背景导出，画面中心 alpha=0。
- 手机最终截图：通用[竖屏](phone-generic-portrait.png)/[横屏](phone-generic-landscape.png)、Forza [按键](phone-forza-buttons-landscape.png)/[保持油门](phone-forza-sticky-landscape.png)/[踏板竖屏](phone-forza-pedal-portrait.png)。
- iPad 最终截图：[通用竖屏](ipad-generic-portrait.png)、[踏板竖屏](ipad-forza-pedal-portrait.png)。横屏按压和拖动也通过入口 UI 测试。

多指来源、同键释放、短按、rollover、绝对/相对摇杆、layer、pullAction、toggle、sticky、取消归零由回放验证。XCUITest 采集实际 UIKit 按压/拖动；设备编译关闭签名。实机多指手感与人体工学接续用户验收。完整 TAKX、动态 Context、图片资源与触摸板速度映射按能力报告继续扩展。

[清理记录](cleanup.json)：本轮临时目录已移除，释放约 969.3 MiB；最终截图、摘要、诊断及共用依赖缓存保留。
