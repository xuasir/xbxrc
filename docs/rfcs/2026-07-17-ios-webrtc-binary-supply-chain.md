# iOS WebRTC 预编译二进制供应链 RFC

## Status

- Completion: 已完成
- Current State: complete
- Owner: agent
- Last Updated: 2026-07-17

## Background

- 实施前的 iOS 串流代码通过远程 Swift Package 引用 `stasel/WebRTC`，工程填写版本 `137.7151.04`。
- 用户要求开发、Release 与 CI 使用同一 WebRTC 二进制，同时避免维护 Chromium/libwebrtc 源码和几十 GB 本地构建环境。
- 原有 `resolve-libwebrtc.sh` 校验 remote package pin、架构、module map 和 SHA；Xcode 直接依赖上游 package，产物 URL 与供应链元数据由上游控制。
- 用户已明确确认按“固定预编译 XCFramework + 镜像 + checksum”路线实施。
- 用户已授权联网下载固定 WebRTC 资产，并授权在 `xuasir/xbxrc` 创建对应 GitHub Release。

## Goal

- 将 Xcode WebRTC 依赖迁移为仓库内本地 Swift Package，package 通过 remote binary target 消费固定 GitHub Release 资产。
- 用版本化 manifest 固化 artifact 版本、上游 revision、archive SHA-256、二进制 SHA-256、架构、module 和许可元数据。
- 提供开发/CI 共用的 resolve 与 verify 流程，SwiftPM 负责下载、缓存和完整 ZIP checksum 校验。
- 保持 Swift `import WebRTC`、现有串流 runtime 和 App target 链接合同不变。

## Scope

- In scope:
  - `iosapp/Packages/WebRTC` 本地 binary package。
  - WebRTC artifact manifest、SwiftPM 解析与结构校验脚本。
  - Xcode project 从 remote package reference 迁移到 local package reference。
  - README、CI 调用约定、升级流程和供应链验证测试。
- Out of scope:
  - Chromium/libwebrtc 源码构建。
  - 修改 libwebrtc 内部 jitter buffer、NACK 或线程调度。
  - 建设独立对象存储服务。
  - 模拟器真实账号串流验收。

## Architecture

```text
GitHub Release: webrtc-137.7151.04
        | WebRTC.xcframework.zip
        v
local Package.swift remote binaryTarget(url, checksum)
        | SwiftPM download/cache/verify
        v
local package product: WebRTC
        |
        v
XBXRC App target -> import WebRTC
```

约束：

- XCFramework ZIP 保存在版本化 GitHub Release，Git 仓库保持轻量。
- `Package.swift`、manifest、LICENSE 与 Xcode 工程提交 Git，版本升级必须同步更新。
- SwiftPM checksum 覆盖完整 ZIP；脚本检查唯一 XCFramework、Device/Simulator arm64、Headers、module map、Privacy Manifest 和二进制 SHA。
- Release URL 内容保持不可变；修订产物时发布新 tag 与新 checksum。
- Xcode 只引用仓库内 local package，开发、Release 与 CI 共享同一依赖合同。

## Plan

1. 审计当前 remote package、artifact 缓存和 pbxproj 引用。
2. 获取 M137 固定 artifact，核验上游 package revision、libwebrtc commit、checksum、架构和许可。
3. 在 `xuasir/xbxrc` 创建固定 Release 并上传 XCFramework、manifest 与 LICENSE。
4. 创建本地 Swift Package 与 artifact manifest，迁移 Xcode package reference，保持 product 名称 `WebRTC`。
5. 更新 resolve/verify 脚本、README 与升级约定。
6. 执行 shell lint、manifest 校验、PBX lint、Swift parse/typecheck 与可用环境下的 Xcode build。

## Validation

- [x] manifest schema 与必填字段校验通过
- [x] 上游 archive SHA-256 与 SwiftPM checksum 一致
- [x] XCFramework 同时包含 iOS Device arm64 与 Simulator arm64
- [x] `WebRTC` module map、Headers、Privacy Manifest 与二进制 SHA 校验通过
- [x] Xcode 工程只引用 local package
- [x] Swift 源码 parse/typecheck 通过
- [x] Debug Device、Debug Simulator 与 Release Device build 通过
- [x] `git diff --check`

## Risks

- GitHub Release 可用性影响首次依赖解析；SwiftPM 本地缓存支持后续增量构建。
- 社区预编译包的安全响应依赖上游；manifest 固定来源，升级流程负责 CVE 与 revision 复核。
- 上游 M137 Release 未提供 dSYM；崩溃符号能力覆盖应用层，WebRTC 内部栈保留地址与版本指纹。
- Release tag 指向远程 `release/test` 当前提交，二进制资产的内容权威由 checksum 与 manifest 保证。

## Progress

- [x] 用户确认固定预编译 XCFramework + 镜像 + checksum 路线。
- [x] 当前 artifact 与 remote package 状态审计完成；Xcode 可用最小迁移为 `XCLocalSwiftPackageReference + remote binaryTarget(url:checksum:)`。
- [x] M137 固定资产下载与审计完成：上游 package `137.0.0`、revision `b85669f...`、libwebrtc `branch-heads/7151` commit `cec4daea...`。
- [x] 本地 package、manifest、LICENSE、脚本和 Xcode 最小迁移已完成。
- [x] GitHub Release `webrtc-137.7151.04` 已创建，XCFramework、manifest 与 LICENSE 已上传。
- [x] M137 codec capabilities 调用已对齐实际 Objective-C API。
- [x] 验证、Report 与任务收口已完成。

## Execution Notes

- Date: 2026-07-17 | Status: complete
- Decision: 复用现有 WebRTC product、PBXBuildFile、Frameworks phase、target dependency 与 `-ObjC`；只替换 package reference，避免影响 Hosts、Library、Streaming 并发改动。
- Decision: 本地 Package.swift 固定镜像 URL 与 SwiftPM checksum，脚本只负责 artifact 结构、架构、module、许可和二进制指纹门禁。
- Evidence: `WebRTC-M137.xcframework.zip` 大小 41,307,432 bytes，checksum `9b45c5c5...3edd`；Device binary SHA `23d0005c...80d9`，Simulator binary SHA `db570f28...a223`；两个 iOS slice 均包含 Headers、`WebRTC` module map 与 Privacy Manifest。
- Decision: 上游资产缺少 dSYM，manifest 显式记录 `unavailable-upstream`，Release 不生成来源不明的符号包。
- Evidence: `resolve-libwebrtc.sh` 从 `xuasir/xbxrc` Release 完成 SwiftPM 解析和 artifact 门禁；Debug Device arm64、Debug Simulator arm64 与 Release Device arm64 完整构建均成功。
