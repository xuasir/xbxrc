// swift-tools-version: 6.0
import PackageDescription
let package = Package(name: "TraceHarness", platforms: [.macOS(.v15)], products: [.library(name: "TraceHarness", targets: ["TraceHarness"])], targets: [.target(name: "TraceHarness"), .testTarget(name: "TraceHarnessTests", dependencies: ["TraceHarness"])])
