# 验收环境诊断

首轮 Xcode 命令退出码 74：沙箱阻止既有 SourcePackages 的 workspace-state.json、SwiftPM manifest 和 Clang 模块缓存写入。使用已授权的固定工程与 iPhone 18 Pro 构建入口重试后，编译和 68 项回归通过。

报告读取与导出首次在同一结果包上并行执行，xcresulttool 的惰性索引触发 TestReport 写入限制及 database.sqlite3 冲突。获得报告工具缓存写入许可后按顺序重试，摘要和全部九张 PNG 导出成功；最终保留三张必要截图。

补充旧键盘输入来源迁移后，重新编译并复验 38 项设置测试，结果为 Passed、失败数 0。Swift 格式/lint 补充检查通过。
