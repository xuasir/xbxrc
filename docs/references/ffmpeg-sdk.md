# FFmpeg SDK 构建说明

固定 SDK 位于 `src-tauri/resources/ffmpeg/{windows-x64,macos-arm64,macos-x64}`；目标布局、版本、来源和校验值见 [SDK 目录说明](../../src-tauri/resources/ffmpeg/README.md)与 [versions.json](../../src-tauri/resources/ffmpeg/versions.json)。每个目标包含 include/、lib/，Windows 额外包含 bin/。

## 本地构建

```sh
pnpm tauri dev
pnpm tauri:build:dmg
pnpm tauri:build:nsis
```

package.json 的 tauri 入口统一经过 [with-ffmpeg-sdk.mjs](../../scripts/with-ffmpeg-sdk.mjs)，注入 FFMPEG_DIR/FFMPEG_INCLUDE_DIR/FFMPEG_LIB_DIR、固定 SDK 的 pkg-config、运行库搜索路径与 CMake 兼容参数。macOS 同时设置 dyld 搜索路径；本地 target/debug 与 target/release 使用 SDK 运行库。XBX_FFMPEG_SDK_TARGET 可覆盖所选目标目录。

## 更新与发布验证

替换目标 SDK 后更新 src-tauri/resources/ffmpeg/versions.json，同步 [LICENSE-NOTICE.md](../../src-tauri/resources/ffmpeg/LICENSE-NOTICE.md)和 [COPYING.LGPLv2.1](../../src-tauri/resources/ffmpeg/COPYING.LGPLv2.1)。默认许可策略为 LGPL-2.1-or-later。

按目标执行 cargo check -p xbxrc 和对应 Tauri 构建，再运行 pnpm release:check-bundle；发布资产集合使用 pnpm release:check-collected，工作流合同使用 pnpm release:verify-pipeline。CI 使用仓库中的同一资源目录，构建与发布检查以这些脚本和工作流为准。
