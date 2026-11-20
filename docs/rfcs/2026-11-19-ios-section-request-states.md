# iOS 信息页分区请求状态 RFC

## 状态与授权

2026-11-19，Done；2026-11-20完成接续回归与收口。主窗口单窗口执行。用户已明确授权“按这个方案完全实施”，追加要求错误提示具有清晰信息层级和可点按重试入口。交付与验收见[Report](../reports/2026-11-19-ios-section-request-states.md)。

设计依据：[ISU](../isu/2026-11-19-ios-section-request-states.md)。本次按该方向实施账户/个人/游戏/主机安装库/相册及云目录来源状态，保留现有 SwiftUI、Store、公共 Rust API/认证归属。

## 目标与范围

Rust bridge 提供类型化错误及复合查询的独立来源，Swift 原 Store 保存成功快照、失败、请求阶段及成功更新时间；页面按来源装配统一局部提示。依照用户后续确认的视觉方向，提示沿用透明、无背景表面，使用14pt半粗标题、12pt原因/旧内容说明和44pt重试操作；整行可点按，宽度跟随所属分区。主机状态与存储提示位于主机图文区域下方的全宽位置。

首次失败位于内容槽位，刷新失败紧贴标题并保留内容，分页失败位于页尾且保留原 continuation；登录恢复使用既有 AuthStore 协调。成功空结果、缺少字段和失败各有语义。商品资料共用一个恢复边界，单个媒体下载/播放独立恢复。

## 执行阶段

- [x] P1：Rust错误合同、账户及安装库来源查询，生成UniFFI绑定。
- [x] P2：原Store扩展，来源快照/时间、分页和取消收尾、认证恢复。
- [x] P3：统一错误组件、中英资源、所有信息页及目录来源迁移。
- [x] P4：Rust与Swift定向测试、手机/iPad真实页面交互和布局、设备构建、格式/边界验证。
- [x] P5：证据归档、临时产物清理、Report与任务账本收口。
- [x] P6：接续核验资料来源去重、时长局部失败的既有回归与当前中英文资源，归档r4证据并清理临时产物。

## 验收

定向验证复合来源部分失败和单来源重试次数，成功空结果后的刷新失败，原 continuation分页恢复，成功来源及时可见，账户/scope迟到响应隔离，401/403/429/timeout/unknown文案及取消busy结束。真实页面覆盖局部重试、明确错误层级、44pt按钮、320pt/手机/iPad、中英/深浅/大字号。真实服务覆盖及实际登录恢复由后续实机使用继续验证。

## 风险与控制

当前工作区有其他任务的已暂存改动，按文件原版本增量修改并保留暂存状态。公共WebApiError当前保留状态和类型，Retry-After由HTTP层消化；UI仅消费明确传递的冷却事实。聚合数据改为来源结果后，旧内容与成功时间按来源保存；既有测试替身通过协议默认实现继续适配。

临时目录：`/private/tmp/xbxrc-ios-section-states-20261119/r1`、`r2`、`r3`、`r4`，开工及接续基线已记录；四轮构建/结果包/日志与一次性文件已清理，共1.808GiB。r1～r3清理1.336GiB，r4清理0.472GiB；每轮磁盘增量见清理记录。SwiftPM依赖缓存保存在 `target/ios-section-states-packages`，WebRTC资源可用。

## 最终验收

2026-11-19，r3 手机29项、iPad3项、Rust bridge67项全部通过，设备目标构建、Swift格式、pnpm lint:fix、cargo fmt检查、串流边界和diff检查通过。最终截图与机器摘要已归档到[实施证据](../../dev-docs/validation/2026-11-19-ios-section-request-states/implementation/README.md)。

主机状态和存储提示采用图文下方全宽透明排布；无障碍字号下主机信息纵排，容量按宽度适配。320pt中英文标准字号使用 `.large`；无障碍样例主动设置 `.accessibility3`，与标准字号预览分别归档。旧内容说明的中英文及时间格式回归通过。

19张最终截图、机器摘要、源码哈希和已解决诊断归档；暂存区基线一致，临时目标全部清理。详见[清理记录](../../dev-docs/validation/2026-11-19-ios-section-request-states/implementation/cleanup.json)。

2026-11-20，r4资料持久化/来源去重、时长局部失败、错误本地化3项定向回归通过，26条提示的中英文与格式占位符核验通过。31份源码与r3验收基线一致；本地化资源的后续变化由当前编译及文案核验覆盖。本轮修改涵盖两项旧断言迁移及来源计数替身调整，其他测试改动与暂存区完整保留。收口期间同一文件新增串流用例，新增块保留，其余代码与r4验收源码哈希一致。[补充证据](../../dev-docs/validation/2026-11-19-ios-section-request-states/implementation/continuation/checks.json)、[并发记录](../../dev-docs/validation/2026-11-19-ios-section-request-states/implementation/continuation/concurrent-update.json)及[r4清理记录](../../dev-docs/validation/2026-11-19-ios-section-request-states/implementation/continuation/cleanup.json)已归档。
