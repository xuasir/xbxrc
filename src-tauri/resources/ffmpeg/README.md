# FFmpeg SDK Layout

This directory stores pinned FFmpeg SDK artifacts used by local builds, CI, and Tauri packaging.

## Targets

- `windows-x64/`
- `macos-arm64/`
- `macos-x64/`

Each target folder must contain:

- `include/` for FFmpeg headers
- `lib/` for import/static libraries and runtime libraries on macOS
- `bin/` for runtime DLLs on Windows

## Build integration

- Use `node scripts/with-ffmpeg-sdk.mjs -- <command>` to inject:
  - `FFMPEG_DIR`
  - `FFMPEG_INCLUDE_DIR`
  - `FFMPEG_LIB_DIR`
  - runtime search path (`PATH`)

## Version pinning

Record version/source/checksum/license details in `src-tauri/resources/ffmpeg/versions.json` when replacing SDK assets.
