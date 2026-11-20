# iOS 手柄页与标准触屏布局验收

日期：2026-11-20。任务：`2026-11-20-ios-controller-settings-scope`，simple。

手柄页保留连接与测试、触屏手柄、摇杆调节、按键与轴映射、震动、菜单操作六个入口。触屏页通过只读行展示“串流触屏布局：标准手柄”，正式串流直接装配标准触屏手柄。

通用 Xbox、地平线按键/踏板及官方格式示例继续通过“布局与输入测试”进入。旧版保存的测试布局加载后恢复 `classic`；旧键盘输入来源恢复手柄与触屏，键盘映射、透明度、显示模式和摇杆偏好随迁移保留。

| 验证 | 结果 | 证据 |
| --- | --- | --- |
| iPhone 18 Pro Simulator 编译与首轮回归 | 66 项单元、2 项界面测试通过 | [首轮摘要](test-summary-initial.json) |
| 补充旧键盘来源迁移与设置复验 | 38 项设置测试通过 | [最终摘要](test-summary-final.json) |
| Swift 格式/lint、前端 lint、串流边界 | 通过；前端 lint 引入的 src 变更为 0 | [静态检查](static-checks.txt) |
| 变更范围、截图哈希与暂存区 | 5 个 Swift 文件核验，既有暂存区哈希相同 | [源码核验](source-checks.json) · [本轮差异](implementation.diff) |

界面回归覆盖手柄映射操作，以及从触屏页进入测试、切换通用/地平线、横竖屏按压与拖动、踏板保持、查看记录/诊断和清空归零。旧偏好测试覆盖标准布局及全部内置测试布局的载入和再次保存，并核对原有设置与映射。

最终截图：[手柄六入口](controller-sections.png) · [标准触屏设置](controller-standard-touch-settings.png) · [地平线测试入口](forza-playground.png)。三张截图已经视觉核对。

本轮临时根目录：`/private/tmp/xbxrc-ios-controller-scope-20261120/r1`。必要截图和摘要归档后，按[进程核验](process-check.json)与[临时清单](temporary-artifacts.json)清理 6070 项、544.82 MiB 临时产物；空父目录同步清理，磁盘可用空间增加 390.63 MiB。路径及磁盘核验见 [清理记录](cleanup.json)，共享 WebRTC/SwiftPM 与 Rust 缓存保留。

构建及报告导出的环境诊断见 [诊断记录](diagnostics.md)。
