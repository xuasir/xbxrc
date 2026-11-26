# iOS 串流执行域 RFC 工程验收

M0–M3 代码已落地：完整触控边沿、独立 InputQueue、MainActor surface + RenderQueue、预编译 shader 和资源缓存、实际呈现观察与公共 Rust 显示恢复。

- iPhoneOS Swift 6 严格并发编译通过（38.41s）。
- iOS Simulator 应用、预编译 Metal shader 与定向 XCTest 包编译通过（最终构建 16.19s）。
- 公共 Rust 监督 27 项定向测试通过，包含显示停滞、解码/网络供给异常、后台/作用域变化、单飞与预算。
- 会话边界检查、Xcode 工程检查及本轮差异空白检查通过。
- 定向 XCTest 已准备，运行等待模拟器启动授权。测试范围含输入顺序/重试/neutral、GPU 与旧回调、pipeline 复用、NV12 实际渲染及 Home/Cloud 入口。

实际真机性能、外接手柄/触控、前后台接续和同档短测的 P95/P99 由真机验收固定。网络与 SDK 恢复链保持现有 libwebrtc 合同；应用监督分别消费接收/解码供给与实际显示事实。

本轮临时目录 `/private/tmp/xbxrc-ios-threading-20261126-ra23942o` 中构建中间产物已清理，可运行测试包保留在 `DerivedData/Build/Products`，用于待授权的定向测试。Metal Toolchain 27B47 与仓库依赖缓存保留供后续开发使用。
