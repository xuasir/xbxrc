# iOS 信息页分区请求状态：实施验收

日期：2026-11-19；2026-11-20接续收口。最终视觉轮次：r3；补充回归：r4。相关[RFC](../../../../docs/rfcs/2026-11-19-ios-section-request-states.md)。

## 最终行为

账户资料、玩家/个人游戏、安装库、相册和云目录按来源保存成功内容、失败及成功时间。首次失败位于所属内容槽位，刷新失败保留已有内容，下一页失败沿原 continuation 恢复；认证续期沿现有 AuthStore 协调。

提示沿最初的透明表面，标题/原因/旧内容形成层级，整行提供至少44pt点击区域。主机状态和存储提示位于图文下方全宽位置；无障碍字号下主机信息纵排，容量按宽度适配。界面错误文案由有限的中英文资源投影。

## 验证

| 验证范围 | 结果 | 证据 |
| --- | --- | --- |
| 手机状态与原生交互 | 29项通过，0失败 | [机器摘要](phone-summary.json)、[用例摘要](phone-test-summary.txt) |
| iPad布局与原生交互 | 3项通过，0失败 | [机器摘要](ipad-summary.json)、[用例摘要](ipad-test-summary.txt) |
| Rust bridge | 67项通过，0失败 | [测试摘要](rust-tests-summary.txt) |
| iOS设备目标 | 构建通过，CODE_SIGNING_ALLOWED=NO | [构建摘要](device-build-summary.txt) |
| Swift格式、前端lint、Rust格式、串流边界 | 全部通过 | [Swift](swift-format-summary.txt)、[lint](lint-summary.txt)、[Rust格式](cargo-fmt-summary.txt)、[边界](streaming-boundary-summary.txt) |
| r4接续回归与资源核验 | 3项回归、26条中英文提示通过 | [机器摘要](continuation/test-summary.json)、[用例摘要](continuation/test-summary.txt)、[完整核验](continuation/checks.json) |

定向用例覆盖部分来源失败与单来源请求次数、成功空结果的刷新失败、分页游标恢复、相册成功来源及时展示、并发栏目合并与成功时间持久化、账户/语言切换的迟到响应隔离、共享401续期、取消结束busy及错误本地化。

r4补齐资料持久化/四来源去重、时长局部失败两条既有回归，同时验证当前编译资源的英文错误文案。31份源码与r3基线一致；本地化资源文件有后续变化，当前26条提示均具备中英文资源及匹配的格式占位符。测试文件的其他改动和暂存区通过[源码基线](continuation/source-hashes.json)核验完整保留。

收口期间测试文件新增串流用例，新增块完整保留；其余代码与r4验收源码哈希一致。本轮分区回归和共用替身保持验收版本，详见[并发变动](continuation/concurrent-update.json)。

## 截图与字号

[预览](preview.jpg)展示320pt窄屏的中英文系统标准字号。SwiftUI `.large` 对应系统标准字号；`.accessibility3` 是测试主动启用的无障碍字号。实际应用通过现有 Dynamic Type 跟随系统设置。

19张截图的来源、尺寸和SHA-256见[清单](manifest.json)。`phone/`保存手机交互与布局矩阵，`ipad/`保存iPad原生交互及宽屏布局；无障碍大字号样例独立命名。

## 归档与清理

[总核验](checks.json)、[源码与暂存基线](source-hashes.json)及[已解决诊断](resolved-diagnostics.json)保留验证证据；三轮临时目录按本任务清单收尾，结果见[清理记录](cleanup.json)。SwiftPM依赖缓存保存在 `target/ios-section-states-packages`。

r4临时目录按[清单](continuation/inventory.json)清理0.472GiB；本任务累计清理1.808GiB，19张最终截图和依赖缓存保留。[接续诊断](continuation/resolved-diagnostics.json)与[补充清理记录](continuation/cleanup.json)已归档。

实机验收重点覆盖真实账户的断网重试、登录恢复与访问受限提示。
