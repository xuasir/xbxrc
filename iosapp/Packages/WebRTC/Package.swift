// swift-tools-version: 5.9

import PackageDescription

let package = Package(
    name: "XBXRCWebRTC",
    platforms: [
        .iOS(.v15),
    ],
    products: [
        .library(
            name: "WebRTC",
            targets: ["WebRTC"]
        ),
    ],
    targets: [
        .binaryTarget(
            name: "WebRTC",
            url: "https://github.com/xuasir/xbxrc/releases/download/webrtc-137.7151.04/WebRTC.xcframework.zip",
            checksum: "9b45c5c5ecae392403758bb7262f408aa3cff705d41e862dd766856b610c3edd"
        ),
    ]
)
