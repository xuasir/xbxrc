# FFmpeg Third-Party License Notice

This project vendors FFmpeg SDK artifacts under `src-tauri/resources/ffmpeg`.

## Effective license policy

- `windows-x64`: **LGPL-2.1-or-later**.
- macOS SDKs: **LGPL-3.0-or-later**, matching the binaries' `--enable-version3` configuration.
- Build and source metadata are tracked in `src-tauri/resources/ffmpeg/versions.json`.

## Artifact sources

- `windows-x64`: BtbN prebuilt package `ffmpeg-n8.1-latest-win64-lgpl-shared-8.1.zip`
- `macos-arm64`: locally built from `ffmpeg-8.1.tar.xz` with:
  - `--disable-gpl --disable-nonfree --enable-shared --enable-version3`
- `macos-x64`: locally built from `ffmpeg-8.1.tar.xz` with:
  - `--disable-gpl --disable-nonfree --enable-shared --disable-x86asm --enable-version3`

## macOS runtime dependencies

The ARM64 SDK includes libX11 1.8.13, libxcb 1.17.0, libXau 1.0.12, and libXdmcp 1.1.5.
Their source URLs, checksums, and individual COPYING notices are recorded beside the
runtime libraries in `macos-arm64/lib/runtime-manifest.json` and `COPYING.*`.
Both macOS SDKs require macOS 26.0 and use package-relative `@rpath` install names.

## Distribution notes

- Ensure packaged app includes this notice and LGPL license text.
- Keep source URL and checksum in sync with shipped binaries.
- If build flags or binary sources change, update `versions.json` and this file together.
