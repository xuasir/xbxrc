# main / rust 发行隔离验证

## Browser 稳定线

- 当前构建提交：`0b7dae51`（main），包含 P0 基线、公共协议/构建身份、发布更新身份门禁、完整原生裁剪及默认目录固定。
- 前端：455 项；公共 Rust：147 项；Tauri：44 项；lint/typecheck/生产构建、生命周期测试通过。
- 发布：8 项版型/分支/tag/版本/identity/hash负例门禁，两 edition 的 macOS/Windows模拟 `bundle -> manifest -> latest.json` 通过。CI YAML解析通过；Browser Cargo normal 依赖树裁剪门禁通过。
- 实际构建：`pnpm tauri build --bundles app,dmg --config '{"bundle":{"createUpdaterArtifacts":false}}'`，使用本地默认 `target/browser`，与 CI 一致。从仓库根目录和 src-tauri 读取 Cargo metadata 均得到同一目录。
- 安装身份 `com.xuasir.xbxrc`，arm64，最低 macOS 26.0；仅1个非系统Mach-O（应用自身），签名/依赖闭包/原生运行库及跨平台资源裁剪通过。
- app 文件合计 18,928,324 bytes；DMG 8,987,875 bytes。产物位于 `target/browser/release/bundle/{macos,dmg}`。
- 当前 app 复制到 `/private/tmp/xbxrc-p1-browser-final-618uyn8b/xbxrc.app` 后再次通过签名与依赖闭包检查。
- 旧 Cargo 缓存迁移曾残留原路径中的 Tauri permissions 引用；清理受影响的 Tauri/plugin release 缓存后，默认目录重建通过。后续沿固定目录构建。
- 本地使用 ad-hoc 签名；此轮生成本地安装包。更新包签名与真实发布由 CI secrets 执行。

## Native WIP

- 原生源码提交：`ebc368d`（main/rust 首次边界合并），构建提交：`9f4bb3c`；本地按 CI 转换为 `0.1.0-native-wip.1` 后构建，仓库保持基础版本。
- none / native-media / native-gamepad / native-wip 编译和依赖归属门禁通过。媒体输入提交由 xbxEngine RPC 承担，媒体单项可以消费 Browser 输入。
- 前端50文件/473项；Rust引擎1277项（24 ignored）、Tauri212项、手柄core36项；关闭原生的Tauri48项；Mac运行库/FFmpeg隔离版本探针8项；发行8项和两edition模拟通过。
- 全量引擎测试发现5个旧测试数据/断言缺口：补齐clean-anchor来源，把host-present合成时间线绑定当前时钟，并校正display-stable投影；原有生产恢复逻辑继续保留。依据：现行completion/display/transport-recovery合同与2026-06-02恢复边界报告。
- 实际 app 为55,599,963 bytes，DMG为25,471,961 bytes；安装身份 `com.xuasir.xbxrc.native-wip`，版本 `0.1.0-native-wip.1`，arm64、macOS26.0。包内14个当前架构Framework，含应用的15个Mach-O闭包与完整签名通过。
- app/DMG 位于 `target/native-wip/release/bundle/{macos,dmg}`，app名称为 `xbxrc Native WIP.app`。移动到 `/private/tmp/xbxrc-p1-native-relocated.app` 后检查再次通过；资源只保留当前平台/架构SDK运行库与许可证。

## 证据边界

- 本轮固定本地 main/rust 开发线和对应 release/test、release/native-wip 快照。远端开发线同步、平台 CI 与真实发布继续作为后续步骤。
- 自动实机运行保持暂停。安装、登录、手柄、xHome/xCloud、退出回收、更新及长测由用户手动验收。
- 当前本地 macOS arm64 构建取证；Windows与macOS x86_64实际CI/安装证据分别记录。
- 辅助 `pnpm skills:verify` 仍报告旧锁文件中的 use-spatial-navigation 目录缺失；当前几何导航代码及本轮前端、Rust、发行检查均已通过，该辅助清单另行整理。
