# 本轮已解决的构建诊断

- Swift 宏子进程受嵌套 sandbox 限制；本轮编译使用 `-disable-sandbox`，最终 iPhoneOS 严格并发编译通过。
- Xcode 缺少 Metal Toolchain；已安装官方 27B47 组件，预编译 Streaming.metal 构建通过。
- XCTest 的 Metal 填充 API 采用 Swift overlay 的 Range 参数，测试包编译通过。
