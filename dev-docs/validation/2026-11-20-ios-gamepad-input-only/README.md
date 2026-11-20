# iOS 游戏输入边界验收

日期：2026-11-20。任务：`2026-11-20-ios-gamepad-input-only`，simple。

## 最终行为

iOS 游戏输入统一为触屏手柄与外接手柄。此前 PC 能力迁移带入的键盘采样、映射、来源分支和默认映射导出已清理。原触屏/手柄 Xbox 输入、热插拔归零、菜单接管、映射和震动继续沿现有合同执行。

- `StreamingKeyboardInput.swift` 与工程注册已删除，`StreamInputSession` 使用手柄和触屏来源，空闲状态表达等待输入。
- 移动偏好省略桌面输入来源与键盘映射。旧 JSON 字段由解码器忽略；再次保存保留触屏透明度、布局、摇杆等移动设置，并省略已撤下字段。
- DataChannel 的设备连接事实由外接手柄和触屏显示状态确定，Rust iOS 专用 `stream_keyboard_defaults` 导出已删除，Swift/FFI 绑定从 Rust 元数据重新生成。
- 控制器保持连接与测试、触屏、摇杆、按键与轴映射、震动、菜单操作六个入口；输入状态使用“外接手柄”中英文名称。当前产品边界记录在 `iosapp/DESIGN.md`，此前 PC 对齐 RFC/报告已注明范围更新。

## 验证

| 项目 | 结果 |
| --- | --- |
| iPhone 17e / iOS 27 Simulator 单元测试 | 84 项通过：39 设置与持久化、17 串流状态、28 触控布局 |
| 实际界面回归 | 2 项通过：控制器四个子页及六入口、正式串流竖屏位置与诊断关闭 |
| Rust iOS bridge | 76 项通过，覆盖 FFI 会话启动及取消 |
| iPhoneOS 目标编译 | 通过，`CODE_SIGNING_ALLOWED=NO`，最低 iOS 26.0 |
| 静态验证 | Rust fmt、Swift lint、前端 lint:fix、会话边界与 diff 检查通过 |
| 源码审计 | 应用源码/生成绑定/工程/iOS Rust bridge 的键盘游戏输入符号匹配数均为 0 |

测试核对旧字段退场、移动偏好保存、触屏按压释放、外接手柄接管、菜单抑制与归零恢复。控制器界面通过 `--settings-ui-test` 使用现有模拟手柄夹具，正式覆盖层通过 `--streaming-panels-ui-test` 装配，截图中的设备状态与背景属于自动化夹具；真实外接手柄热插拔、持续串流与手感由用户复测。

6 张最终截图见 `manifest.json`，测试摘要见 `validation.json`、`phone-summary.json`、`phone-test-summary.txt` 与 `rust-test-summary.txt`，源码审计及校验见 `source-audit.json`、`source-sha256.json`。沙箱首轮诊断见 `failure-diagnostics.json`。

## 临时产物

本轮临时根：`/private/tmp/xbxrc-ios-gamepad-input-only-20261120`。必要证据归档后清理本轮 DerivedData、xcresult、导出截图、日志和查看缩略图。共享 `target/ios`、`target/ios-bindgen`、`target/browser`、WebRTC SourcePackages 及其他任务数据保留。清理核验见 `cleanup.json`。
