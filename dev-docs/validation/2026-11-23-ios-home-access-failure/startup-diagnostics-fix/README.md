# iOS 主机连接启动诊断补齐

已完成诊断代码与编译验证，实际连接阻断待相同环境的新Trace确认。用户确认Wi-Fi裸连、Xbox待机、“连接时唤醒主机”开启、PC端可连接；现有日志记录授权成功后会话准备54.837秒失败，以及下一次授权请求超时。

## 源码发现

- 唤醒设置经Swift请求快照透传到Rust，公共会话编译器消费该配置。
- PC复用有效的Home凭据，续期按目标范围执行；iOS `prepare_home_access`调用完整`refresh_and_finalize`，包含Home及Cloud请求。公共认证流程捕获Cloud失败并保留诊断，Home可继续成功返回；额外请求能增加等待，连接故障的因果关系仍待验证。
- iOS此前通过通用`session.failed`收口，Trace保留终态与清理动作，Rust具体启动错误在这一边界缺少记录。

## 本轮改动

- `sessionCreateStarted`增加实际请求的`powerOnRequested`和target；增加`controlSessionCreated`、`signalingPreparationStarted`与`signalingPreparationSucceeded`。
- 复用既有300ms进度轮询，已观测进度变化时记录`startupProgressChanged`；沿用Rust安全投影的阶段、状态、队列秒数及重试次数。
- 失败收口新增essential级`sessionFailure`，保留HTTP状态、固定网络类别、固定启动/运行时错误码、资源存在标志与最后已观测进度。
- 分类函数使用固定输出与白名单，错误正文仅参与匹配；身份、凭据、完整地址保留在原错误对象内。现有认证、唤醒、串流与清理行为沿用。

## 验证

- Foundation/XCTest宿主探针执行真实生产诊断函数和正式测试源码：10项通过，0失败；类型声明从真实生产源码提取，探针源码与仓库源码逐字节一致。
- Xcode 27.0 / iPhoneOS 27.0 / arm64 / Debug正式App构建通过，签名关闭；结果摘要为0错误、2项既有`HostListView.swift`弃用提示。
- Swift解析、工程plist、定向格式检查、会话边界检查与diff空白检查通过。
- 首次完整构建在SwiftPM/Clang系统缓存写入时被沙箱拦截，退出74；按正常权限流程重试后完整构建退出0。环境失败诊断与成功编译锚点分别归档。

宿主测试验证错误分类与隐私边界，iPhoneOS构建验证正式App类型接线。主机唤醒、会话就绪、首帧与实际日志落盘由实机重试形成证据；网络、恢复链、media supply、steady supply、submit/present长尾与帧供给delta按会话到达阶段接续评估。

## 证据与下一轮

`validation.json`、`tests.json`、`device-build-summary.json`、`source-hashes.json`、`task-changes.diff`和`cleanup.json`记录本轮结果；`sources/`保留本次验收的代码快照，`logs/`保留必要测试与构建诊断。

使用更新后的iOS构建，在相同Wi-Fi裸连、主机待机和唤醒开启条件下重试，失败后立即导出完整诊断日志。下一轮先读取`sessionFailure`的错误类别和`startupProgressChanged`的最后阶段，再选择对应的网络、发现或主机就绪检查。

本轮临时路径为`/private/tmp/xbxrc-ios-home-startup-diagnostics-20261123-33399546-r1`。构建、测试与结果导出进程结束后已清理该目录，移除1534个临时文件、540.24MiB分配空间；清理后磁盘可用43.11GiB。共享Rust/WebRTC依赖缓存、用户原始日志与工作区既有改动保留，进程核验与清理结果记录在`process-check.json`和`cleanup.json`。
