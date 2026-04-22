# FFmpeg Third-Party License Notice

This project vendors FFmpeg SDK artifacts under `src-tauri/resources/ffmpeg`.

## Effective license policy

- Current vendored FFmpeg artifacts are intended to be **LGPL-2.1-or-later**.
- Build and source metadata are tracked in `src-tauri/resources/ffmpeg/versions.json`.

## Artifact sources

- `windows-x64`: BtbN prebuilt package `ffmpeg-n8.1-latest-win64-lgpl-shared-8.1.zip`
- `macos-arm64`: locally built from `ffmpeg-8.1.tar.xz` with:
  - `--disable-gpl --disable-nonfree --enable-shared`
- `macos-x64`: locally built from `ffmpeg-8.1.tar.xz` with:
  - `--disable-gpl --disable-nonfree --enable-shared --disable-x86asm`

## Distribution notes

- Ensure packaged app includes this notice and LGPL license text.
- Keep source URL and checksum in sync with shipped binaries.
- If build flags or binary sources change, update `versions.json` and this file together.
