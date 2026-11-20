# UniFFI 串流启动栈优化交付

实机报告中的 `WrappedFuture::new` 在会话 future 构造时耗尽约 544 KiB 工作线程栈。本轮已将 Rust 启动上下文与公共会话执行按阶段 `Box::pin`，并收敛 5 个大型信令/恢复入口；优化局限于现有异步状态机的内存布局，沿既有 scope、generation、认证和清理合同执行。

| 项目 | 优化前 | 优化后 / 结果 |
| --- | --- | --- |
| 导出 `start` future | 119,216 B | 3,184 B，缩小 97.3% |
| 实机崩溃函数 `WrappedFuture::new` 的设备栈帧 | 477,280 B | 13,136 B，缩小 97.2% |
| 4 层 Rust 构造函数栈帧合计 | 原报告落入 Stack Guard | 当前设备产物 23,344 B |
| 导出异步入口 | 6 个超过 8 KiB | 15 个全部通过 8 KiB 预算 |
| 堆上启动阶段 | — | 3 个全部满足 32 KiB future 体积预算 |
| 新增回归 | — | 6 项通过；7 个实际 544 KiB 线程案例 |
| Rust 回归 | — | bridge 73、shared streaming 176、protocol 10，共 259 项通过 |
| Debug iPhoneOS | — | BUILD SUCCEEDED，关闭签名 |

[结构化验收](validation.json)、[设备栈布局](device-stack-layout.json)及[源码哈希](source-manifest.json)对应最终实现。设备反汇编中的 3 个 UniFFI 函数哈希与原 `.ips` 完全对应；[函数反汇编](device-future-disassembly.txt)记录优化后的栈指令，[入口反汇编](device-start-disassembly.txt)显示实际调用关系。[局部生产差异](production-layout.patch)以本轮开始时的工作区为基线，保持原有编辑的归属。

新增测试直接调用真实生成的 FFI `start`，覆盖 xCloud/xHome 的创建、poll、401 错误透传、pending 取消和直接 free，以及对象所有权释放。xCloud loopback 夹具继续走实际 `SessionFlow` 的 HTTP create、Provisioned、record 解码和 remote close；测试通过独立子进程为本地 URI 排除 proxy。[最终桥接日志](bridge-tests.log)包含当前 73 项和小栈生命周期回归，[公共层日志](rust-tests.log)包含 176+10 项验证。子测试的一条通过记录属于父测试的执行内容，Rust 汇总按父测试集计算。

UniFFI [重新生成校验](bindings-validation.json)确认 C 头文件、modulemap 逐字一致，Swift 唯一差异为末尾换行；工作区生成文件保持原内容。[生成日志](bindings-validation.log)、[设备构建日志](device-build.log)、[修复前预算失败](baseline-layout.log)和[已解决的夹具诊断](setup-diagnostics.json)保留验收边界。

自动检查覆盖构造安全、控制流程及资源释放。实机 xCloud/xHome 的实际首帧、退出重连、网络健康、恢复链、media supply、steady supply、submit/present 长尾与帧供给 delta 继续沿[原清单](../../2026-11-19-ios-streaming-pc-behavior-parity/device-acceptance.md)验收。当前任务为 `Validation Pending`，建议用当前代码重新编译安装后完成两种串流复测。

本轮临时 DerivedData、生成绑定、一次性基线、日志及额外 xcresult 已清理，回收 416.26 MiB，清理后可用空间 62.94 GiB；[清理记录](cleanup.json)保存路径与产物删除前的哈希核验。固定 WebRTC 与 Rust 构建缓存继续保留。原始报告的 Downloads 路径在清理前已为空，分析摘录、机器码与原始哈希保留在上级证据目录。
