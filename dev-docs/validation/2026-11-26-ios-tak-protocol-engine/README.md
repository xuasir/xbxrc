# iOS TAK 通用协议引擎验收

日期：2026-11-26。协议修订：`tak-v4.1-1252720f5e3eb604`，IR 版本：1，编译器修订：1。社区来源固定为 `4416b1ab07588a3244a5c02ce1fa5dfa64d8df74`。

## 结果

| 验证对象 | 结果与范围 |
| --- | --- |
| 社区整库 | 75 个模板、85 份布局使用同一编译入口；85 份可运行、70 份完整兼容，整布局通用回退为零 |
| 手柄 profile | 输入动作与轴的实现缺口为零；6 处隔离轴均为相对鼠标输出，1 处隔离控件为纯鼠标触摸板 |
| 图标合同 | 官方五档 DPI 均为 234 个名称；应用使用官方 3x 图集，GTA6 三类手柄图标进入公共绘制路径 |
| Rust | 核心 34 项通过；本机库 16 项按影响复用与补验后通过，当前协议缓存升级另做 1 项定向补验 |
| 官方数值方法 | 从冻结发布字节提取实际方法，7 项对照通过；覆盖阈值缺省 0、阈值大于 1、负死区与大于 100 的灵敏度 |
| iOS | 19 项单元与 1 项 UI 的最终有效结果通过；最终合同轮次为 8 项单元与 1 项 UI |
| 形式与边界 | Rust fmt、Swift format、前端 lint:fix、串流职责边界和 diff 检查通过 |

完整兼容与可运行分别表达。剩余诊断覆盖传感器、相对鼠标、可选样式、槽位容量及社区布尔形式；编译器保留来源并按能力处理。结构覆盖由整库编译证明，输入和绘制行为由能力分组单元与模拟器 Surface 冒烟证明。

## 证据索引

- [协议整库清单](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-ios-tak-protocol-engine/protocol-corpus.json)：每份布局的能力、精确诊断路径、兼容状态及隔离来源。
- [官方数值方法对照](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-ios-tak-protocol-engine/numeric-reference-replay.json)：发布原件哈希、AST 提取方法与 7 项观测值。
- [验收摘要](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-ios-tak-protocol-engine/validation-summary.json)与[iOS 用例矩阵](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-ios-tak-protocol-engine/ios-test-matrix.json)：保留每轮结果，按用例记录最后有效结果。
- [最终 Xcode 摘要](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-ios-tak-protocol-engine/ios-final-summary.json)：iPhone 18 Pro / iOS 27.2 模拟器，9/9 通过。
- [执行与来源清单](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-ios-tak-protocol-engine/execution-manifest.json)及[本轮实现差异](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-ios-tak-protocol-engine/implementation.diff)：源码基线、最终哈希、授权、临时路径及协议修订。
- [失败诊断](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-ios-tak-protocol-engine/failure-diagnostics.md)：首轮保存门禁、测试断言及旋转交互问题的定向修正。
- [冻结方案来源](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-ios-tak-protocol-engine-plan/README.md)：schema、官方运行时与图集的发布原件和哈希。

## 最终截图

截图沿正常游戏入口进入预览，UI 夹具在该入口提供冻结 GTA6 社区布局，游戏展示身份沿既有 Halo 测试宿主保留。正式 Surface、协议编译器与本机保存状态均按实际产品路径执行。

- [竖屏预览](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-ios-tak-protocol-engine/protocol-community-portrait.png)
- [横屏预览](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-ios-tak-protocol-engine/protocol-community-landscape.png)
- [保存完成状态](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-ios-tak-protocol-engine/protocol-community-saved.png)

## 收尾与设备边界

清理结果见[清理记录](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-ios-tak-protocol-engine/cleanup.json)。保留本目录的结果、源码差异和最终截图，以及共享 Rust 构建缓存。模拟器恢复初始 Shutdown 状态并保留设备与应用数据。

本轮完成模拟器和离线能力验收。真实 GTA6 多指手感、远端游戏响应与持续串流沿仓库约定由用户实机验收。
