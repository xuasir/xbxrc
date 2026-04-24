# FFmpeg SDK 封装化使用说明

## 目录约定

- `third_party/ffmpeg/windows-x64/{include,lib,bin}`
- `third_party/ffmpeg/macos-arm64/{include,lib}`
- `third_party/ffmpeg/macos-x64/{include,lib}`

构建和打包统一从上述目录读取头文件、链接库和运行库。

## 许可证策略

- 当前默认策略：仅接入 `LGPL-2.1-or-later` 的 FFmpeg 产物。
- 许可证元数据记录在 `third_party/ffmpeg/versions.json`。
- 发布产物需随包携带：
  - `third_party/ffmpeg/LICENSE-NOTICE.md`
  - `third_party/ffmpeg/COPYING.LGPLv2.1`

## 本地构建

- 日常开发与调试：
  - `pnpm tauri dev`
- macOS DMG:
  - `pnpm tauri:build:dmg`
- Windows NSIS:
  - `pnpm tauri:build:nsis`

以上命令都会通过 `scripts/with-ffmpeg-sdk.mjs` 注入（`tauri` 命令已统一包装）：

- `FFMPEG_DIR`
- `FFMPEG_INCLUDE_DIR`
- `FFMPEG_LIB_DIR`
- 运行时库搜索路径（`PATH` 前置 SDK 路径）

## SDK 资产更新

1. 替换目标目录下的 SDK 文件（保持目录结构不变）。
2. 更新 `third_party/ffmpeg/versions.json` 的版本、来源、哈希、许可证信息。
3. 运行最小验证：
   - `cargo test -p xbox-webapi`
   - `cargo check -p xbxrc`
   - `pnpm tauri:build:dmg`（macOS）
   - `pnpm tauri:build:nsis`（Windows）

## CI 说明

- GitHub Actions 不再下载外部 FFmpeg 预编译包。
- 工作流会先检查 `third_party/ffmpeg` 目录是否完整，再执行打包。
