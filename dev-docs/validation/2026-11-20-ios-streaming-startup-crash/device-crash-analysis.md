# 2026-11-19 实机串流崩溃分析

这份报告确认 Rust/UniFFI 会话启动时发生栈溢出。故障位于 `XboxStreamSession.start()` 的 FFI future 构造阶段，Peer 创建、SDP offer 和视频解码沿后续步骤执行。xCloud 与 xHome 共用这个导出入口及固定 future 类型，符合用户反馈的两种串流都在首帧前退出。

## 调用栈与机器码证据

- 崩溃时间：2026-11-19 18:45:58 +08:00；App 0.1.0（1），报告包含 `XBXRC.debug.dylib`，设备型号码 `iPhone18,2`，iPhone OS 27.2.1。
- 异常：`EXC_BAD_ACCESS / SIGBUS / KERN_PROTECTION_FAILURE`。访问地址和 SP 同为 `0x16b507450`，落入 `0x16b504000～0x16b508000` 的 `Stack Guard`。该 guard 上方工作线程栈范围为 `0x16b508000～0x16b590000`，大小 544 KiB。
- 触发线程：`com.apple.root.user-initiated-qos.cooperative`。核心调用链为 `StreamSessionActor.start → prepareSignaling → RustStreamingControlSession.prepareSignaling → XboxStreamSession.start → uniffi_xbox_ios_bridge_fn_method_xboxstreamsession_start → rust_future_new → RustFuture::new → WrappedFuture::new`。
- 报告内嵌的机器码直接解出 `sub x9, sp, #0x74000`，随后逐 4 KiB 下移 SP 并执行 `str xzr, [sp]`。故障 PC 位于这个栈探测写指令，偏移 `WrappedFuture::new +20`。该函数完整预留量为 `0x74000 + 0x840 + 32 = 477,280` 字节，约 466.1 KiB；叠加上层构造调用的栈帧后耗尽工作线程栈。

[结构化摘录](device-crash-analysis.json)包含原文件 SHA-256、触发线程与保留字段；[机器码解码](diagnostics/device-ips-code-disassembly.txt)直接来自报告的 `instructionByteStream`。原始 `.ips` 保留在用户提供的 Downloads 路径。

## future 体积与代码顺序

针对同日缓存的 macOS arm64 Rust 开发库，独立类型大小探针输出 `start=119216`（约 116.4 KiB）、`exchange_offer=14688`、`close=176`、`boxed_start=8`。探针仅使用 `size_of::<F>()`，输入闭包始终保持未执行状态；[日志](diagnostics/device-ips-future-size.log)和[探针源码](diagnostics/device-ips-future-size.rs)用于解释大型 future 经 UniFFI 构造/包装时的栈占用。缓存库哈希和构建时间记录在结构化摘录中，后续生产修复的构造、poll 和取消行为需独立验收。

Swift 在 [StreamSessionActor](../../../iosapp/XBXRC/Platform/Streaming/StreamSessionActor.swift) 的 262 行等待 `prepareSignaling`，随后创建 Peer，279 行生成 offer；[RustStreamingControlSession](../../../iosapp/XBXRC/Platform/Streaming/StreamingRuntime.swift) 的 747 行调用 `session.start()`。Rust [导出入口](../../../crates/xbox-ios-bridge/src/streaming.rs)的 `start` 直接持有上下文准备、跨区发现及公共会话启动的异步分支，UniFFI 0.32 以按值 future 进入构造包装。

此前交付的 SDP 诊断字典修复消除了可复现的 `SIGTRAP` 致命错误；这份报告确认 `SIGBUS` 栈溢出。此前 12 项 SDK 测试覆盖 SDP、Peer、H264 和 NV12，当前故障链需要补充真实 Rust `start` FFI 构造覆盖。

## 修复落点与验收

修复优先缩小交给 UniFFI 的 `start` future，在 Rust 内部对重型上下文和公共会话启动的 await 分支进行堆分配。保留导出签名和既有 Rust/Tokio/UniFFI bridge，沿当前 `SessionFlow` 继续维护取消、代际、认证续期与远端清理约束。该方向遵循 [iOS 端到端合同](../../../docs/rfcs/2026-07-21-ios-streaming-end-to-end-contract-alignment.md)与[行为对齐 RFC](../../../docs/rfcs/2026-11-19-ios-streaming-pc-behavior-parity.md)，起点为 `XboxStreamSession.start` 和其内部两条重型 await 分支。

本轮完成报告解析、机器码解码与类型大小分析，生产修复继续待实施。验收需覆盖约 544 KiB 小栈环境下真实 FFI future 的创建、poll、cancel/free，以及 Debug iPhoneOS 构建与实机 xCloud/xHome 首帧。网络健康、恢复链、media supply、steady supply、submit/present 长尾与帧供给 delta 继续沿[原实机清单](../2026-11-19-ios-streaming-pc-behavior-parity/device-acceptance.md)收集成功启动后的会话 trace。

本轮分析编译与解码进程已结束，临时 object、可执行探针和一次性源码目录已清理，回收约 1.16 MiB。最终机器码、探针源码与摘要归档在本证据目录，原始报告和已有 Rust 依赖缓存保留。[清理记录](analysis-cleanup.json)保存本轮产物清单及空间核验。

## 2026-11-20 优化交付接续

后续[栈优化](stack-fix/README.md)已完成，以上章节保留分析阶段的原始证据与当时结论。Rust `start` future 由 119,216 降至 3,184 B；实际 Debug iPhoneOS 产物中，与 `.ips` 同一 UniFFI 函数对应的栈帧由 477,280 降至 13,136 B，4 层构造函数栈帧合计 23,344 B。15 个导出入口和 3 个堆上启动阶段分别满足 8 KiB、32 KiB future 体积预算，真实 FFI 小栈创建、poll、取消、释放及 xCloud 实际 HTTP 控制流程通过。

最终 259 项 Rust 回归与 Debug iPhoneOS 构建通过，[结构化验收](stack-fix/validation.json)和[源码快照](stack-fix/source-manifest.json)记录当前实现。任务为 `Validation Pending`，重新编译安装后完成 xCloud/xHome 首帧、退出重连与原六维 trace 清单的实机验收。

优化交付收尾时，原始 `.ips` 的 Downloads 路径已为空；以上分析、原始哈希、机器码与类型大小探针保留于证据目录。[栈优化清理](stack-fix/cleanup.json)记录这一文件状态和本轮临时产物清理结果。
