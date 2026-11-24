// swift-tools-version: 6.2
import PackageDescription
let package = Package(name: "XBXRCStartupDiagnosticsProbe", platforms: [.macOS("26.0")], targets: [.target(name: "XBXRC", path: "Sources"), .testTarget(name: "XBXRCTests", dependencies: ["XBXRC"], path: "Tests")])
