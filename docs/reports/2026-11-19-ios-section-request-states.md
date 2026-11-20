# iOS 信息页分区请求状态 Report

日期：2026-11-19；2026-11-20补充接续收口。状态：Done。相关[RFC](../rfcs/2026-11-19-ios-section-request-states.md)及[验收证据](../../dev-docs/validation/2026-11-19-ios-section-request-states/implementation/README.md)。

## 交付结果

iOS信息页按请求来源呈现局部失败，成功内容与最近成功时间持续保留。提示沿最初的透明表面，标题、原因、旧内容说明形成层级，整行提供44pt以上重试区域。首次失败、刷新失败和分页失败分别使用对应位置与恢复动作。

| 页面/来源 | 最终行为 |
| --- | --- |
| 账户与玩家资料 | 身份、社交、好友数量、在线状态独立请求和提交；登录恢复汇入现有AuthStore。 |
| 个人游戏与成就 | 记录、时长、成就及对比来源独立恢复；成功空结果保留业务空态，下一页沿原游标重试。 |
| 主机安装库 | 游戏、存储、状态、封面补全各自保存来源状态；主机状态和存储提示位于图文下方全宽区域，点按说明文字即可重试。 |
| 云游戏与目录 | 商品资料、个人记录、等待时间分别恢复；最近/新增/热门/离库栏目定向恢复，栏目排序和成功时间独立保存。 |
| 相册与媒体 | 成功来源及时显示；截图/片段分别重试，分页保留内容与游标，媒体操作拥有独立提示。 |

## 主要改动

Rust bridge新增稳定错误分类及账户、安装库、目录来源查询，生成UniFFI绑定。Swift沿现有Store扩展成功快照、请求阶段、结构化失败和成功时间，复用scope/owner归属、去重与认证续期。

统一`AppSectionRequestState`及有限中英文错误文案，接入账户/个人/游戏/安装库/相册/目录。刷新失败保存旧内容，取消结束busy；并发栏目重试通过串行快照提交保留各自排序与时间。

主机页无障碍字号使用纵向信息排布，容量指标按宽度适配。补齐“显示上次内容”及时间说明的英文资源与回归断言。320pt中英文使用系统标准字号`.large`；无障碍测试主动设置`.accessibility3`，实际应用沿现有Dynamic Type跟随系统设置。

## 验证

| 验证 | 结果 |
| --- | --- |
| 手机定向状态与UI | 29项通过 |
| iPad布局与局部恢复UI | 3项通过 |
| Rust bridge | 67项通过 |
| iOS设备目标 | CODE_SIGNING_ALLOWED=NO构建通过 |
| Swift格式、pnpm lint:fix、cargo fmt检查、串流边界、diff检查 | 通过 |
| r4接续定向回归 | 3项通过；资料来源去重、时长局部失败、错误本地化 |

用例覆盖单来源请求次数、成功空结果后刷新失败、原continuation恢复、相册部分失败、账户与语言的迟到响应隔离、多个401共用一次续期、并发栏目合并与成功时间持久化、取消busy及技术错误的本地文案投影。

19张最终截图与标准字号中英文[预览](../../dev-docs/validation/2026-11-19-ios-section-request-states/implementation/preview.jpg)、机器测试摘要、源码哈希和已解决诊断归档到证据目录。

接续复核将资料持久化的既有断言调整为四个来源各请求一次，重复激活持续去重；时长失败的既有断言核验游戏列表成功状态与更新时间保留，失败归入时长分区。两项回归及错误文案回归共3项通过，26条提示的中英文与格式占位符通过静态核验。[补充证据](../../dev-docs/validation/2026-11-19-ios-section-request-states/implementation/continuation/checks.json)保留当前源码哈希、测试摘要及环境诊断；其他测试改动与暂存区通过哈希核验完整保留。

收口期间同一测试文件新增了串流相关用例，新增块完整保留；其余代码与r4验收源码哈希一致，本轮分区回归及共享替身保持验收版本。[并发变动记录](../../dev-docs/validation/2026-11-19-ios-section-request-states/implementation/continuation/concurrent-update.json)。

## 收尾

确认构建、测试、导出进程结束后清理`/private/tmp/xbxrc-ios-section-states-20261119/r1`、`r2`、`r3`，临时目标核验全部结束；清理1.336GiB，磁盘可用空间增加1.144GiB。`target/ios-section-states-packages`缓存及WebRTC资源保留，暂存区哈希与开工基线一致。[清理记录](../../dev-docs/validation/2026-11-19-ios-section-request-states/implementation/cleanup.json)。

r4测试和导出进程结束后清理0.472GiB，磁盘可用空间增加0.381GiB；本任务累计清理1.808GiB，临时目录目标全部结束。19张截图及依赖缓存继续保留。[补充清理记录](../../dev-docs/validation/2026-11-19-ios-section-request-states/implementation/continuation/cleanup.json)。

真实账户的服务覆盖、访问受限和登录恢复由实机使用继续验收。优先核验断网后各区块的提示、恢复网络后的局部重试，以及登录恢复后的原来源续载。
