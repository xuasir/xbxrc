# Home 启动操作轮询栈修复

用户在 `My Mac (Designed for iPad)` 发起 Home 串流时，Xcode 显示 `SessionCancelToken::run` 轮询路径发生 `EXC_BAD_ACCESS code=2`。[现场截图与记录](incident.json)保留目标、可见符号和异常地址。用户随后退出了调试现场；完整调用栈、寄存器和内存区域需由下一次运行采集。截图中的旧 provider 符号与当前设备产物存在差异，详见[构建对应诊断](setup-diagnostics.json)。

## 修复与量化

[公共取消入口](../../../../crates/xbox-streaming/src/session/operation.rs)在构造 async/select/timeout 包装前，通过 `Box::pin` 将操作固定到堆上；`SessionStartupScope::run` 持有轻量取消包装。取消优先级、超时取消作用域、错误透传和资源释放继续遵循现有公共会话合同，PC 与 iOS 共用该实现。

| 类型体积 | 修复前 | 修复后 | 当前预算 |
| --- | ---: | ---: | ---: |
| 凭据访问经作用域包装后的 future | 24,944 B | 816 B | 8 KiB |
| Home 上下文 future | 28,960 B | 1,568 B | 8 KiB |
| 公共会话执行 future | 31,768 B | 6,880 B | 8 KiB |
| 启动 Plan future | 6,144 B | 6,144 B | 8 KiB |

凭据包装体积下降 96.73%；底层凭据访问/续期 future 分别为 6,104 B、5,200 B。[修复前预算失败](baseline-credential-budget.txt)与[修复后通过](credential-budget-after.txt)固定同一测量入口。15 个 UniFFI 导出异步入口均满足 8 KiB 类型体积预算，`start` 保持 3,184 B；64 KiB 操作的取消、作用域和嵌套包装分别为 400/816/816 B。

## 验证

| 范围 | 结果与证据 |
| --- | --- |
| Rust 完整回归 | bridge 76、streaming 178、protocol 10，共 264 项通过、0 失败；[日志](rust-tests.txt) |
| 小栈回归 | 11 个测试覆盖 20 个 544 KiB 案例；[日志](stack-regressions.txt) |
| Home/Cloud 控制路径 | 实际生成 FFI 驱动生产 SessionFlow，loopback 夹具验证主机查询、create → Provisioned → close 和对象释放 |
| 认证续期 | Home/Cloud 实际 AuthFlow 发起请求，夹具返回 503；typed 错误透传、凭据更新状态及释放通过 |
| 取消/超时/释放 | 64 KiB 操作覆盖完成、超时、运行中取消、已取消、首次 poll 前释放和 pending 释放 |
| Debug iPhoneOS 构建 | generic/platform=iOS，关闭签名，构建成功；[摘要](device-build.summary.txt) |
| PC Tauri 编译 | `cargo check -p xbxrc --target-dir target/browser` 通过；[日志](pc-tauri-check.txt) |
| 格式与边界 | cargo fmt、会话边界和变更空白检查通过；[检查摘要](continuation-checks.json) |

测试代理环境由独立子进程隔离，实际请求由本地夹具响应。凭据 provider 的有效/过期状态、Home 与 Cloud 两种目标均得到覆盖。[结构化验收](validation.json)记录案例分布、请求及预算；[源码清单](source-manifest.json)绑定生产改动和测试源文件，续接时全部哈希一致。

[当前设备机器码](device-operation-prologues.json)共分析 107 个函数，取消包装相关单个异步栈帧最大 13,696 B，构造栈帧最大 15,264 B。类型体积与单个机器码栈帧各自提供静态证据；完整运行调用链峰值和截图异常的精确归因由新运行补充。

## 设备验收与清理

状态为 `Validation Pending`。重新编译当前源码，在 `My Mac (Designed for iPad)` 复测 Home 启动，再在实机核对 xCloud/xHome 首帧、退出重连；连续串流按[原清单](../../2026-11-19-ios-streaming-pc-behavior-parity/device-acceptance.md)采集网络健康、恢复链、media supply、steady supply、submit/present 长尾与帧供给 delta。异常复现时保留 `.ips` 或完整 LLDB 调用栈、SP/PC 和本轮构建标识。

本轮临时根目录为 `/private/tmp/xbxrc-ios-home-auth-stack-fix-20261120`，另有一份系统临时 xcresult。必要截图、基线失败、最终测试摘要、机器码和源码哈希保留在本目录；[清理记录](cleanup.json)记录进程核对、路径清单、回收空间与共享缓存保留项。

进程与归档哈希核对后，两处登记临时路径已清理，回收 413.69 MiB；清理后可用空间 61.42 GiB。最终截图、失败诊断、264 项回归日志、设备机器码和 PC 编译摘要保留；共享 Rust/WebRTC 缓存继续复用。
