# iOS WebRTC 预编译二进制供应链 Report

## Summary

- Related RFC: [`docs/rfcs/2026-07-17-ios-webrtc-binary-supply-chain.md`](/Users/guo.xu/Documents/code/games/xbxrc/docs/rfcs/2026-07-17-ios-webrtc-binary-supply-chain.md)
- iOS WebRTC 已收口为仓库内本地 Swift Package + 固定 GitHub Release binary target。开发、Release 与 CI 使用同一 XCFramework ZIP 和 SwiftPM checksum，开发机只需下载约 41 MB 资产。

## Delivered

- 创建 [`webrtc-137.7151.04`](https://github.com/xuasir/xbxrc/releases/tag/webrtc-137.7151.04) Release，发布 `WebRTC.xcframework.zip`、`artifact-manifest.json` 与 `LICENSE-WebRTC.txt`。
- 增加本地 `iosapp/Packages/WebRTC` package，固定 Release URL、checksum、上游 package revision、libwebrtc commit、架构、二进制 SHA、Privacy Manifest 与许可来源。
- Xcode 从 `XCRemoteSwiftPackageReference` 迁移到 `XCLocalSwiftPackageReference`，保留 `WebRTC` product、Swift import、Frameworks phase 和 `-ObjC` 链接合同。
- `resolve-libwebrtc.sh` 使用版本隔离缓存，执行 SwiftPM 解析、唯一 artifact、Device/Simulator arm64、Headers、module map、Privacy Manifest 与二进制指纹校验。

## Changes

- 固定上游来源为 `stasel/WebRTC 137.0.0` revision `b85669f32ffb3f48ce3a8f18ad828c6f559a8a0c`，对应 libwebrtc `branch-heads/7151` commit `cec4daea7ed5da94fc38d790bd12694c86865447`。
- 固定完整 ZIP checksum 为 `9b45c5c5ecae392403758bb7262f408aa3cff705d41e862dd766856b610c3edd`。
- 将 codec capability 枚举调整为 M137 提供的 `RTCPeerConnectionFactory.rtpSenderCapabilities(forKind:)` API。
- README 更新为 Release 镜像、缓存路径和升级流程说明，移除 remote package pin 与 `Package.resolved` 合同。

## Validation

- `./iosapp/scripts/resolve-libwebrtc.sh`：SwiftPM 镜像下载与全部 artifact 门禁通过。
- Debug `xcodebuild ... -destination 'generic/platform=iOS' ... build`：Device arm64 完整构建成功。
- Debug `xcodebuild ... -destination 'generic/platform=iOS Simulator' ... build`：Simulator arm64 完整构建成功。
- Release `xcodebuild ... -configuration Release -destination 'generic/platform=iOS' ... build`：生产配置优化编译、Rust release 静态库和 WebRTC framework 链接成功。
- `sh -n`、JSON 解析、PBX plist lint、local package resolve、定向 `git diff --check` 通过。

## Risks

- GitHub Release 可用性影响全新环境首次解析；SwiftPM 缓存支持已解析环境的增量构建。
- 上游 M137 资产未发布 dSYM，WebRTC 内部崩溃栈保留地址、版本与二进制指纹，应用层符号化保持完整。
- 社区预编译包的安全升级依赖上游发布节奏；每次升级需要重新固定 revision、commit、checksum 和双平台构建证据。

## Follow-up

- 真实 Xbox 账号串流验收继续覆盖 ICE、H.264/Opus、DataChannel、RTCStats 与生命周期清理。
- 下次 WebRTC 升级发布新 `webrtc-*` tag，保持已发布资产内容不可变。
