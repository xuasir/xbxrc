# iOS 串流启动闪退修复与原生 SDK 验证

用户反馈 xCloud 与 xHome 均在出画面前闪退。SDP 诊断修复、实机报告定位的 UniFFI 启动栈优化，以及后续 Home 操作轮询包装修复均已交付；真实 SDK 回归、最新 264 项 Rust 测试、Debug iPhoneOS 构建和 PC Tauri 编译通过。任务为 `Validation Pending`，当前构建的本机 Home、实机首帧、退出重连与六维 trace 沿清单验收。

## 可复现崩溃与修复

`StreamingRuntimeDiagnostics.sdpSummaryPayload` 将全部 media section 的 `rtpmap` 放入 `Dictionary(uniqueKeysWithValues:)`。仓库现有 [PC SDP 样本](../../../src/player/infra/webrtc/SdpManipulator.test.ts)在音频与视频中复用 payload 96，生产函数逐字提取的独立复现触发 `Fatal error: Duplicate values for key: '96'`，进程以 SIGTRAP 终止。原始[崩溃输出](diagnostics/sdp-repro-before.log)、[复现源码](diagnostics/sdp-repro-before.swift)与[函数哈希](diagnostics/sdp-repro-source.json)已保留。

[修复入口](../../../iosapp/XBXRC/Platform/Streaming/LibWebRtcRuntime.swift)按各自 media section 解析 codec；同一 section 的重复映射采用最后一个值。audio/video 摘要、payload 顺序和已有字段继续沿原合同生成。[修复后复现](diagnostics/sdp-repro-after.swift)覆盖跨 section 同号、重复行、空 SDP、多路视频与缺失映射，5 项全部通过。

## 实机报告分析接续

[实机分析](device-crash-analysis.md)确认异常为 `EXC_BAD_ACCESS / SIGBUS`，访问地址落入 `Stack Guard`。报告内机器码显示 `WrappedFuture::new` 单个栈帧预留 477,280 字节，约 466.1 KiB；工作线程栈为 544 KiB，叠加上层构造调用后耗尽栈空间。故障位于 `XboxStreamSession.start` 的 FFI future 创建阶段，Peer、SDP offer 与视频解码沿后续步骤执行；两种串流共用该入口。

同日缓存 Rust 开发库的类型大小探针得到 `start=119216`、`boxed_start=8` 字节。[结构化摘录](device-crash-analysis.json)、机器码和探针证据已归档。后续在 Rust 内部按阶段堆分配启动上下文、公共会话及 5 个重型信令/恢复分支；`start` future 降至 3,184 B，实际 Debug iPhoneOS 产物中对应 `WrappedFuture::new` 栈帧降至 13,136 B。

[栈优化交付](stack-fix/README.md)覆盖 15 个导出异步入口的 8 KiB 体积预算、3 个堆上启动阶段的 32 KiB 体积预算，以及 7 个实际 544 KiB 小栈案例。真实 FFI 的创建、poll、401 透传、cancel/free、pending 直接释放及对象释放通过；xCloud 夹具走实际 HTTP create → Provisioned → close。最终 bridge 73、shared streaming 176、protocol 10，共 259 项通过；[结构化验收](stack-fix/validation.json)关联设备机器码、源码哈希及生成绑定合同校验。

## 本机 Home 操作轮询接续

用户新增 `My Mac (Designed for iPad)` 的 Home 启动异常，截图停在 `SessionCancelToken::run`。公共取消入口在 async/select/timeout 构造前将操作固定到堆上，凭据包装 future 从 24,944 B 降至 816 B，Home 上下文从 28,960 B 降至 1,568 B；三个启动阶段统一满足 8 KiB 类型体积预算。新增五项回归，当前 11 个小栈测试覆盖 20 个 544 KiB 案例，实际 FFI 验证 Home/Cloud 的生产控制流程、认证续期错误、取消与资源释放。最新 Rust 264 项、Debug iPhoneOS 构建、PC Tauri 编译及格式/边界检查通过。

[操作轮询交付](home-auth-stack-fix/README.md)保留截图、基线失败、最终日志、设备机器码和源码哈希。调试现场已由用户退出；截图符号与当前产物的差异已记录，完整运行栈与异常精确归因由当前源码的新运行补充。

## SDP 修复的原生验收

| 范围 | 结果 | 证据 |
| --- | --- | --- |
| SDP 生产函数独立回归 | 5 passed | [日志](diagnostics/sdp-repro-after.log) |
| iPhone 17e 原生 SDK 回归 | 12 passed、0 failed、0 skipped | [Xcode 摘要](phone-summary.json)、[最终日志](diagnostics/native-smoke-final.log) |
| iPhoneOS 设备目标 | BUILD SUCCEEDED，关闭签名 | [日志](diagnostics/device-build.log) |
| 会话边界与源文件对应 | 边界通过，3 份源码哈希一致且早于最终测试 | [源码快照](source-manifest.json) |
| 固定 WebRTC 依赖 | 137.7151.04 checksum、Device/Simulator 二进制校验通过 | [缓存校验](diagnostics/webrtc-cache-verify.log) |

新增测试直接调用真实 libwebrtc，验证两种反馈模式的 Peer/offer 创建、原生 H.264 的首帧解码及后台回调，以及 NV12 帧从后台 ingress 到现有 Metal presenter 的完成事件。初次呈现测试使用了设备参数为空的继承初始化入口；测试改为显式装配系统 Metal 设备后通过，[初次失败摘要](diagnostics/render-fixture-failure.txt)保留该原因。生产 Metal、解码和传输链继续沿已有实现。

## Trace 与实机待验

原始 trace 为 1 个 dev 文件、1 个 session、19 行，覆盖 2026-11-20 09:43:26.160～09:43:29.807。schema、sequence、配对、脱敏及预算门均 PASS；文件 7,688 字节，预算 32 MiB，留存 1/6，压力通知为 0。[门报告](phone-trace-gate.json)与[事件摘录](streaming-events.json)关联 `displayPipelineReady → videoFrameAcceptedToMailbox → displaySubmit → displayPresent → displayFirstPresent`；提交、完成各 1 次。

| 维度 | 本轮证据与接续范围 |
| --- | --- |
| 网络健康 | 本地 offer 创建已验证；真实路径、selected pair、RTT、jitter、丢包与反馈待实机 trace |
| 恢复链健康 | 真实触发、动作预算和后续新帧待实机 trace |
| media supply | 独立夹具验证 H.264 首帧回调与 NV12 GPU 完成；真实接收/解码供帧待验 |
| steady supply | 连续 15 秒真实供帧窗口待验 |
| submit/present 长尾 | 单帧完成已验证；P95/P99 与实际上屏待连续实机样本 |
| 帧供给 delta | NV12 夹具 accepted/submit/complete 为 1/1/1；真实会话累计增量待验 |

已收到 `XBXRC-2026-11-19-184559.ips`，该报告确认会话异步桥接栈溢出。SDP 修复对应独立的 `SIGTRAP` 诊断致命错误。当前代码已完成导出 future 栈优化与真实 FFI 小栈回归；重新编译安装后复测 xCloud/xHome 首帧、退出重连，串流表现沿[原实机清单](../2026-11-19-ios-streaming-pc-behavior-parity/device-acceptance.md)验收。

## 清理

构建、测试和导出进程均已结束。本轮临时目录、4 份 xcresult、重复构建及一次性下载脚本已回收，共 0.92 GiB，清理后可用空间 65.23 GiB。最终诊断、测试摘要、19 行原始 trace 和源码哈希保留在本目录；86.81 MiB 的固定 WebRTC 缓存迁入仓库忽略目录 `target/ios-webrtc/137.7151.04/SourcePackages`，其 artifact 路径重定位与二进制校验均完成。[清理记录](cleanup.json)保存路径与空间核验。[完整摘要](test-summary.json)记录自动验收和实机待验边界。

实机报告分析的独立临时目录另回收约 1.16 MiB；最终机器码、类型大小探针及摘录已归档，原始报告和 Rust 缓存保留。[分析清理记录](analysis-cleanup.json)保存本轮清单。

栈优化轮次的临时 DerivedData、生成绑定、基线源码、日志及额外 xcresult 已按独立清单清理，回收 416.26 MiB，清理后可用空间 62.94 GiB；固定 WebRTC 与 Rust 缓存保留。[栈优化清理](stack-fix/cleanup.json)记录路径、设备产物删除前的哈希核验与保留项。最终核验时原始 `.ips` 的 Downloads 路径已为空，报告分析摘录、机器码与原始 SHA-256 保留在本目录。

Home 操作轮询接续的独立临时根目录及一份系统临时 xcresult 已清理，回收 413.69 MiB，清理后可用空间 61.42 GiB；必要证据与共享 Rust/WebRTC 缓存保留。[轮询修复清理](home-auth-stack-fix/cleanup.json)记录归档和路径核验。
