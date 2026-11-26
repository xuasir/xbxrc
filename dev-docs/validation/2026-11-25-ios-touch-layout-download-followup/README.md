# iOS 布局下载与 Cloud / Home 切换

日期：2026-11-25；任务：`2026-11-25-ios-touch-layout-download-followup`。R1 完成共用请求、资源解析和重试缓存修复；R2 接续两个详情下载入口与 Cloud / Home 串流切换。

## 修复范围

- 官方 TAKX 的 Manifest、Context、布局与引用文档统一按 UTF-8 解码，执行 ECMAScript 文本边缘空白处理后解析，兼容 BOM。错误保留行列；单份损坏布局保留诊断，其他有效布局继续可选择、预览和保存。缺失可选 Context 沿引用实际需要诊断，文件及解包预算保持原合同。
- 社区来源与 better-xcloud 一致：按已经校验的 Xbox Title ID 选取游戏索引，`product_id` 保留为来源元数据，保存使用当前游戏的商品身份。来源修订、全部基础布局合并、作者、默认布局和本机资源使用现有 Rust/UniFFI 合同。
- 官方与社区查询独立返回来源状态，公共社区请求使用独立资源路径。社区成功非空目录按标题缓存，容量上限 64；失败和空目录继续允许重试。用户重新查询清理目标缓存并取得新修订，旧请求通过代次门禁保持最新缓存。
- 地平线 6 沿当前上游的社区选源规则作为回归样本；普通游戏覆盖官方和社区并行查询，以及两种来源的下载失败恢复与预览准备。

## 日志依据

用户原始诊断为 218 行、单个 dev 会话，时间范围为北京时间 09:40:19–09:41:11。[分析](trace-analysis.json)和[关键事件](trace-anchors.json)已保留。

| 事件序号 | 事实 |
| --- | --- |
| 77 | Cloud 授权成功 |
| 78 | 社区来源被商品身份匹配规则阻断 |
| 79、175 | 官方目录成功，各返回一个布局包 |
| 82、211 | 官方资源解析报“布局 JSON 格式无效” |
| 202 | 社区来源超时 |

Schema、sequence、privacy、budget 和关键流程覆盖通过。全流程 gate 的两项配对缺口位于图片请求 seq 42、185；布局问题依据上述独立事件定位。用户失败游戏包的原始字节仍待新构建取得，具体编码原因保持待证实状态；微软原始样例直接证明 BOM 兼容缺口。

## 原件与参考

- [better-xcloud 源码摘录](better-xcloud-reference.json)：主线 `ea4d19f2800cee9cfa387a0c9c37b3711cf208c7`，gh-pages `4416b1ab07588a3244a5c02ce1fa5dfa64d8df74`；记录标题选源、成功缓存、资源修订和地平线 6 定义。
- [微软原件清单](official-sample-reference.json)：`xbox-game-streaming-tools` commit `99d5449bb054908876ceb867f948652f3b9cb41b`。两个完整 TAKX 包合计七份布局，七份均含 UTF-8 BOM；自定义资源包包含五档图片密度，测试验证九份 2x 图片的原始字节与哈希。原件及许可证位于 [fixtures](fixtures)。
- 社区地平线 6 索引引用 `forza`，当前上游布局与应用现有 Forza 资源语义一致。回归覆盖当前商品身份保存、默认布局、作者与重启加载。

## R1 定向验证

| 验证 | 结果 |
| --- | --- |
| `xbox-touch-layouts` | 23 项通过，包括两个微软原件、BOM、Context、布局隔离与版本规则 |
| iOS bridge `touch_layouts::` | 15 项通过，包括两种来源预览、保存、重启、资源原件与回收 |
| iPhone 18 Pro / iOS 27 单元回归 | 9 项通过，包括来源失败/取消、缓存刷新与延迟旧结果、两种下载重试、图片绘制及保存门禁 |
| UI 冒烟 | 1 项通过，43.215 秒；官方来源失败时社区可用，卡片 → 预览 → 横竖屏 → 保存 → 本机列表 |
| 格式与差异 | Rust `cargo fmt`、Swift 本轮修改范围格式化、`git diff --check` 通过 |

[Rust 摘要](rust-test-summary.json)、[XCTest 摘要](xctest-summary.json)、[测试明细](xctest-tests.json)、[构建诊断](build-diagnostics.log)、[格式记录](format-validation.json)与[本轮代码差异](task-changes.patch)已归档。四张 UI 截图逐张查看，见[截图清单](screenshots.json)。

R1 的官方错误卡片来自主动注入的授权失效，用于验证单来源隔离；该轮四张截图记录社区成功流程。

## R2 Cloud / Home 按需切换

- Cloud 使用串流资格校验返回的当前游戏身份。打开选择器只查询官方与社区目录，展示来源和版本；点选条目后下载、准备并切换可用默认布局。包内其他布局在下载成功后展示，后续切换复用当前会话的成功包。
- Cloud 下载失败保留当前布局并提供来源重试；目录刷新、游戏变化与会话结束通过代次失效旧结果。待适配默认项允许选择同包中的可用布局。
- Home 展示本机模板和已保存的游戏布局，标准／最近快捷继续使用本机持久记录。Cloud 最近选择保存在当前会话；Home 最近记录独立保留。
- 两类切换共用原输入 controller，提交新布局前释放旧触点、sticky/toggle、layer、轴和按键，输出 neutral 帧后恢复输入。
- 云游戏详情与主机已安装游戏详情沿同一下载页提供官方和社区预览、保存，成功目录新增刷新入口。
- 横屏浮层的目标内容高度提升到 320pt，并按实际视口保留边界；列表可滚动点选模板、已保存项和当前游戏布局。竖屏继续使用居中的半屏浮层。

7 项单元和 3 条最终 UI 成功路径通过：查询后零下载、点选下载和包内缓存、来源失败恢复、旧 Cloud 请求失效、Home 快捷与输入释放，以及两个详情下载入口的四条成功操作。两个入口 UI 耗时 55.943 秒，最终 Cloud 切换 55.109 秒、Home 切换 58.370 秒。

Home 初轮断言受横屏懒加载条目定位影响；改为滚动定位后，截图确认半屏高度压缩列表。提升浮层高度后，仅补验受影响的 Cloud / Home 切换，两项全部通过。7 项单元与两个下载入口结果继续复用。初轮 Xcode 自动收集 600 秒系统诊断，在确认测试结束后停止该子进程；后续定向验收关闭自动系统诊断，保留必要的测试错误和截图。

[合并摘要](r2/combined-summary.json)、[初轮结果](r2/xctest-summary.json)、[最终切换结果](r2/final-switches-summary.json)、[Cloud 明细](r2/cloud-switch-details.json)、[Home 明细](r2/home-switch-details.json)、[高度修复前诊断](r2/home-attempt-test-details.json)和[初轮定位错误](r2-home-failure.txt)可复核。

| 场景 | 最终截图 |
| --- | --- |
| 云游戏详情下载 | [官方](screenshots/healthy-cloud-official-download.png) · [社区](screenshots/healthy-cloud-community-download.png) |
| 主机已安装游戏详情下载 | [官方](screenshots/healthy-installed-official-download.png) · [社区](screenshots/healthy-installed-community-download.png) |
| Cloud 查询后零下载 | [当前游戏目录](screenshots/cloud-current-game-query-only.png) |
| Cloud 点选下载并应用 | [官方](screenshots/cloud-official-layout-applied.png) · [社区](screenshots/cloud-community-layout-applied.png) |
| Cloud 横屏切换复用包 | [当前游戏选择器](screenshots/cloud-current-game-picker-landscape.png) |
| Home 已保存布局与快捷 | [最近保存项](screenshots/home-touch-quick-saved-portrait.png) · [标准布局](screenshots/home-touch-standard-landscape-right.png) |
| Home 横屏列表与最近模板 | [增高后的选择器](screenshots/home-touch-picker-landscape.png) · [最近模板](screenshots/home-touch-quick-custom-landscape.png) |

成功入口与串流 UI 使用可控请求客户端，资源解析与保存使用 R1 的微软完整原件回归。正常账号的实际服务返回、Cloud / Home 实机串流和手感按原 RFC 接续用户验收；推荐使用本次源码构建的 App 检查这些实际操作。

## 清理

本轮创建的参考仓库、复制的 SourcePackages、DerivedData、原始 xcresult、重复截图、测试日志和一次性脚本按[执行清单](execution-manifest.json)清理。[清理记录](cleanup.json)保留进程结束、删除范围及空间核验。用户原始日志、共享 Rust/WebRTC 缓存与原有模拟器数据保留。
