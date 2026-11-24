# 串流快捷按钮自动隐藏与空白唤回验收

2026-11-24。接续关联聊天的最后一项需求，沿现有串流控制层完成两个快捷入口共同显隐与触点分流。

- 首次进入、单击串流空白与手动收起菜单后显示两个入口；5秒空闲后共同隐藏。
- 空白单击恢复快捷入口；菜单由菜单按钮或双指向内手势打开。横屏、竖屏共用显隐逻辑。
- 虚拟手柄触点沿真实控件命中处理，快捷显隐保持游戏输入；布局加载期间暂停隐藏计时。

[验收汇总](validation.json)：iPhone 18 Pro / iOS 27.0 Simulator、arm64无签名构建及3项定向冒烟通过，范围格式化与差异检查通过。最低部署沿iOS26。

| 验收 | 证据 |
| --- | --- |
| 既有状态用例补充快捷显隐、详情保持与手动关闭 | [最终测试摘要](final-test-summary.json)、[用例结果](final-test-results.json) |
| 隐藏后的原生窗口命中TouchLayoutSurfaceView，输入接管为false | [原生命中](hidden-controls-hit-routing.json)、[复现探针](native-hit-smoke.swift) |
| 5秒共同隐藏、真实A键输出16、横竖屏空白唤回、8pt间距与双指菜单 | [构建与测试摘要](build-test-excerpts.txt) |
| 横屏隐藏、横屏唤回、竖屏唤回 | [隐藏](streaming-quick-buttons-hidden.png)、[横屏](streaming-quick-buttons-revealed.png)、[竖屏](streaming-quick-buttons-revealed-portrait.png) |

原生独立窗口使用100ms计时检查隐藏后的命中；UI流程采用生产同值5秒。首轮XCTest的虚拟元素isHittable断言失败，本轮通过实际按压输出与原生命中确认交互。接续构建期间社区布局接口与Swift绑定正在同步，生成完成后的构建和定向测试通过；[失败诊断](failure-diagnostics.json)保留来源与处理结果。

[任务差异](task-changes.diff)、[源哈希](source-hashes.json)、[范围格式化](final-swift-format.json)及[探针恢复记录](temporary-source-restoration.json)记录最终源码。临时探针已从正式测试源移除，既有状态用例的快捷显隐断言保留。

[清理记录](cleanup.json)：构建、测试及导出进程结束后，本轮临时目录、DerivedData、结果包、日志与重复截图清理572.39MiB。保留3张最终截图、验收摘要、失败诊断和复现资料；共享Rust/WebRTC缓存沿用。

实机重点验收空白唤回与连续触屏操作的手感。
