# iOS TAK 通用协议引擎实施报告

## 完成结果

按[已批准 RFC](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-11-26-ios-tak-protocol-engine.md)完成现有 Rust/Swift 链的协议化迁移。官方和社区来源共用版本化合同与编译 IR；iOS 几何、输入和绘制消费该结果。GTA6 的图标遗漏与整库同类问题通过公共能力修正。

冻结社区修订 `4416b1ab07588a3244a5c02ce1fa5dfa64d8df74` 的75个模板、85份布局全部可运行，其中70份完整兼容。剩余15份按能力降级，原文与诊断持续保留。手柄动作/轴的实现缺口为零，隔离的轴均为相对鼠标输出。逐布局结果见[协议整库清单](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-ios-tak-protocol-engine/protocol-corpus.json)。

## 核心变化

### 协议合同与公共编译

[生成器](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/scripts/generate-tak-protocol.py)从冻结 schema、运行时规则与图集形成183个 schema 定义和234个图标的联合合同。当前协议修订为 `tak-v4.1-1252720f5e3eb604`，IR版本1，编译器修订1。

[公共编译模块](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-touch-layouts/src/protocol/mod.rs)处理版本路由、历史转换、引用/Context、控件、类型化动作和轴、样式与能力诊断。社区包装、新包装版本及 Unicode 文件名按结构与安全路径边界处理；来源版本和内容保持可追溯。未知输入按控件或输出隔离，可选视觉字段使用默认与动作标签，严重状态错误及资源/复杂度预算维持硬边界。

整库审查还修正了输入数值的本地范围：官方 `ActionThreshold` 为非负值，缺省为0；`DeadzoneThreshold` 允许带符号值；正灵敏度按协议保留。22处阈值2/2.5和1处负死区恢复原始语义，完整兼容数量由52增至70。输出继续过滤非有限贡献并按手柄范围限幅。

### 原生执行、绘制与入口

[Swift 归一化入口](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Features/TouchControls/TouchLayoutNormalizer.swift)收敛为公共编译适配，既有输入贡献合并、多指释放、layer、状态更新和会话归属继续执行。[IR 模型](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Features/TouchControls/TouchLayoutIR.swift)保留协议修订、能力编号与精确诊断路径。

[图集读取](/Users/guo.xu/Documents/code/games/xbxrc/iosapp/XBXRC/Features/TouchControls/TouchIconAtlas.swift)使用官方3x位图与帧目录，复用裁剪结果。GTA6使用的 LT/LB/RT 和其他布局的 RB 属于官方234项图集；原177项本地目录的遗漏得到统一补齐。现有原生符号与动作标签承接缺失视觉资源的回退。

预览、卡片保存、已保存加载和串流选择统一使用 `isRunnable`；`isFullyCompatible` 单独报告完整兼容。来源诊断持续展示，保存可运行布局时消费同一编译判定。

### 本机缓存与动态更新

[本机库](/Users/guo.xu/Documents/code/games/xbxrc/crates/xbox-ios-bridge/src/touch_layouts/storage_repository.rs)新增独立缓存 `compiled/<来源revision>.<协议revision>.ir.json`，携带来源修订、协议修订、IR版本和校验值。旧修订、损坏缓存或资源变化时利用保存原文重新编译，保存项身份与来源 revision 保持稳定。

来源快照完整性损坏继续失败；装饰图片缺失或损坏使用回退并留下诊断。同版本重存继续修复共享资源。Context/JSON Patch沿现有会话、scope、generation与输入释放合同重新编译，旧缓存回收和库预算保持统一。

## 最小验收

| 验证 | 结果 |
| --- | --- |
| Rust 核心 | 最终34/34通过，含85份整库编译、历史转换、分级回退、输入数值及预算 |
| Rust 本机库 | 16项有效结果通过；失败两项定向补验通过，当前协议缓存换代再做1项补验 |
| 官方原方法 | 冻结发布字节经哈希校验与 AST 提取，7项数值对照通过 |
| iOS 能力 | 19项单元有效结果通过；覆盖图集、样式、多指、输入释放、保存/加载、动态状态与合法数值 |
| 正式 UI | 1条社区布局预览、触摸板拖动/释放、横竖屏和保存退出通过 |
| 最终合同轮次 | iPhone 18 Pro / iOS27.2模拟器，8项单元+1项UI，9/9通过 |
| 工程约束 | 绑定生成及模拟器构建成功；Rust fmt、Swift format、lint:fix、串流边界与 diff 检查通过 |

验收按影响复用通过结果，失败定向修正。[用例矩阵](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-ios-tak-protocol-engine/ios-test-matrix.json)保留各轮状态，[失败诊断](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-ios-tak-protocol-engine/failure-diagnostics.md)说明保存门禁、断言误置与旋转点击时序的修正。UI使用现有正常游戏入口与冻结GTA6布局夹具，截图的游戏身份沿既有Halo测试宿主保留。

## 清理与保留

[执行清单](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-ios-tak-protocol-engine/execution-manifest.json)归档源码基线和最终哈希，[实现差异](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-ios-tak-protocol-engine/implementation.diff)以本轮基线生成。

已确认本轮进程结束，删除登记临时根中的 DerivedData、5份xcresult、重复截图、录像、展开运行时、日志和基线ZIP，清理约0.79GiB产物，测得可用空间增加约0.76GiB。保留最终3张截图、1张失败诊断图、测试/源码证据，以及共享 `target/ios`、`target/ios-bindgen`、`target/browser` 缓存。模拟器恢复初始Shutdown状态，设备和应用数据保留。[清理记录](/Users/guo.xu/Documents/code/games/xbxrc/dev-docs/validation/2026-11-26-ios-tak-protocol-engine/cleanup.json)记录路径和磁盘空间。

## 后续验收与更新

鼠标、键盘与传感器能力按RFC暂缓，当前已支持控件持续执行。真实GTA6多指手感、远端游戏响应及持续串流沿仓库约定交接用户实机验收。

后续协议更新固定新来源修订并重新生成合同。编译或执行语义变化时递增生成器中的 `compilerRevision`，由协议修订触发本机缓存重编译；验证继续以协议能力和整库批次组织。
