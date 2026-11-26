# iOS 后台串流连接保持验收

2026-11-26，单窗口完成 iOS 前后台生命周期修复。后台保留现有 Rust 会话、PeerConnection、接收音频和已就绪的播放状态，画面呈现与输入按可见性暂停；回前台沿原连接续接。`UIBackgroundModes = [audio]` 使用现有 WebRTC 音频 owner 的系统后台能力，实际连接故障继续沿公共监督与恢复合同处理。

已启用的麦克风保持原有后台暂停采集行为，前台恢复失败由音频诊断记录。普通接收播放的前后台切换持续复用音频配置和激活租约。

用户授权启动现有 iPhone 18 Pro / iOS 27.2 Simulator。单轮 9 项定向 XCTest 全部通过，覆盖原 peer 与 playing 保留、重复切换、后台启动后的首次呈现、忙碌 peer 快照下直接续接、退出后的迟到通知、健康回报与新会话去重、Store 输入/呈现切换、后台音频打包声明，以及实际 ICE 故障的同会话恢复。前后台正常切换的 `validateResume` 调用为 0，peer 关闭、远端会话关闭、access 释放为 0，offer 保持初始一次；主动退出的完整释放另有断言。

修改范围的 Swift 解析、四空格格式、串流会话边界、Info.plist 和 diff 检查通过。构建和测试约 49 秒，用例执行约 0.34 秒；测试执行一轮。结果导出脚本首次返回 64，直接 CLI 导出成功。

- [测试结果](xctest-summary.json)与[用例明细](smoke-test-cases.log)
- [验证与源码指纹](validation.json)及[本轮实现差异](implementation.patch)
- [执行清单](execution-manifest.json)、[构建进程记录](xcode-run.json)与[清理记录](cleanup.json)

本轮临时 DerivedData、复制的 SourcePackages、xcresult、日志、格式配置和基线全部清理，释放 736.35 MiB；清理时关联构建、测试及导出进程均已结束，目标目录删除后可用空间约 86.6 GiB。最终证据、用户授权启动的模拟器及 App、共享 WebRTC/Rust/Xcode 缓存和用户原有改动保留。

真机验收继续由用户执行：Home 和 Cloud 各播放一次有声音的游戏，切后台约 30 秒再返回，确认声音、原画面与输入续接，再主动退出。真实 iOS 后台调度、网络和媒体供给使用实机结果收口；本轮代码与自动冒烟完成，任务保留 Validation Pending。
