# 本轮已修复的验证诊断

- 真机 SDK 初次类型检查发现 `CVBufferCopyAttachment` 已由 Swift 管理返回对象，以及可选 String 的动态模式匹配诊断；已采用直接读取对象和显式比较，最终严格检查通过。初始摘要与输出见 `device-typecheck-initial.json` / `device-typecheck-initial.log`。
- `xcresulttool` 首次摘要导出受系统 `TestReport` 缓存写权限限制；使用已授权本地导出权限后成功，14 项 iOS 测试结果保留。
- 最终 iOS 与 Rust 测试失败数均为 0。
