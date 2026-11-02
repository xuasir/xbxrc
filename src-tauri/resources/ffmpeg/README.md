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

## macOS runtime contract

The app and both macOS SDKs require macOS 26.0. Runtime libraries use `@rpath`
identities and ship with their complete dependency closure. `runtime-manifest.json`
records the source and normalized binary checksums; `COPYING.*` carries dependency notices.

The wrapper collects the independent runtime libraries for the selected architecture
into the app's `Contents/Frameworks`. Tauri signs the main executable and these
libraries with the same identity. Local ad-hoc builds use the corresponding
non-Hardened Runtime policy; certificate builds retain Hardened Runtime by default.
The app links with `@executable_path/../Frameworks`, so installed and relocated
bundles resolve their own runtime libraries. Preparation restores executable file
permissions and signs normalized assets; the manifest pins the resulting bytes.

To prepare existing SDK assets in a new directory, run from the repository root:

```sh
node scripts/prepare-macos-ffmpeg-runtime.mjs macos-arm64 /tmp/xbxrc-sdk-arm64 /opt/homebrew
node scripts/prepare-macos-ffmpeg-runtime.mjs macos-x64 /tmp/xbxrc-sdk-x64
```

The preparation command reads local assets and writes only to the new output directory.
Copy a verified output into the corresponding SDK folder before building. CI consumes
the committed assets directly and validates both SDKs plus the built app:

```sh
pnpm test:macos-runtime
pnpm release:check-macos-runtime src-tauri/resources/ffmpeg/macos-arm64
pnpm release:check-macos-runtime src-tauri/resources/ffmpeg/macos-x64
pnpm release:check-macos-runtime target/release/bundle/macos/xbxrc.app
```
