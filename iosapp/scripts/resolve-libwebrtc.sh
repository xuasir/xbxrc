#!/bin/sh

set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
IOSAPP_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
PROJECT="$IOSAPP_DIR/XBXRC.xcodeproj"
SCHEME="XBXRC"
PACKAGE_DIR="$IOSAPP_DIR/Packages/WebRTC"
PACKAGE_MANIFEST="$PACKAGE_DIR/Package.swift"
ARTIFACT_MANIFEST="$PACKAGE_DIR/artifact-manifest.json"
MODE=${1:-resolve}

if [ ! -f "$PACKAGE_MANIFEST" ] || [ ! -f "$ARTIFACT_MANIFEST" ]; then
  echo "缺少 WebRTC 本地 package 合同" >&2
  exit 1
fi

VERSION=$(/usr/bin/plutil -extract mirror.version raw -o - "$ARTIFACT_MANIFEST")
ARTIFACT_URL=$(/usr/bin/plutil -extract artifact.url raw -o - "$ARTIFACT_MANIFEST")
ARTIFACT_CHECKSUM=$(/usr/bin/plutil -extract artifact.swiftPMChecksum raw -o - "$ARTIFACT_MANIFEST")
ARTIFACT_SIZE_BYTES=$(/usr/bin/plutil -extract artifact.sizeBytes raw -o - "$ARTIFACT_MANIFEST" 2>/dev/null || true)
EXPECTED_DEVICE_SHA=$(/usr/bin/plutil -extract artifact.binarySHA256.iosDeviceArm64 raw -o - "$ARTIFACT_MANIFEST")
EXPECTED_SIMULATOR_SHA=$(/usr/bin/plutil -extract artifact.binarySHA256.iosSimulatorUniversal raw -o - "$ARTIFACT_MANIFEST")
SOURCE_PACKAGES_DIR=${SOURCE_PACKAGES_DIR:-}
DERIVED_DATA_PATH=${DERIVED_DATA_PATH:-}
XCODEBUILD_BIN=${XCODEBUILD_BIN:-xcodebuild}
CURL_BIN=${CURL_BIN:-/usr/bin/curl}
UNZIP_BIN=${UNZIP_BIN:-/usr/bin/unzip}
SHASUM_BIN=${SHASUM_BIN:-/usr/bin/shasum}
DOWNLOAD_TIMEOUT_SECONDS=${DOWNLOAD_TIMEOUT_SECONDS:-600}

if ! grep -Fq "$ARTIFACT_URL" "$PACKAGE_MANIFEST"; then
  echo "Package.swift 的 WebRTC URL 与 artifact manifest 不一致" >&2
  exit 1
fi

if ! grep -Fq "$ARTIFACT_CHECKSUM" "$PACKAGE_MANIFEST"; then
  echo "Package.swift 的 WebRTC checksum 与 artifact manifest 不一致" >&2
  exit 1
fi

format_bytes() {
  bytes=$1
  awk -v bytes="$bytes" 'BEGIN {
    if (bytes >= 1073741824) printf "%.1f GiB", bytes / 1073741824
    else if (bytes >= 1048576) printf "%.1f MiB", bytes / 1048576
    else if (bytes >= 1024) printf "%.1f KiB", bytes / 1024
    else printf "%d B", bytes
  }'
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "[WebRTC] 缺少必需命令: $1" >&2
    exit 1
  fi
}

find_existing_source_packages_dir() {
  home_dir=${HOME:-}
  derived_data_root="$home_dir/Library/Developer/Xcode/DerivedData"
  project_name=${PROJECT##*/}
  project_name=${project_name%.xcodeproj}
  latest_path=""
  latest_mtime=0

  if [ -z "$home_dir" ] || [ ! -d "$derived_data_root" ]; then
    return 1
  fi

  for candidate in "$derived_data_root/$project_name"-*; do
    [ -d "$candidate/SourcePackages" ] || continue
    [ -f "$candidate/info.plist" ] || continue
    workspace_path=$(/usr/bin/plutil -extract WorkspacePath raw -o - "$candidate/info.plist" 2>/dev/null || true)
    [ "$workspace_path" = "$PROJECT" ] || continue
    candidate_mtime=$(stat -f '%m' "$candidate" 2>/dev/null || printf '0')
    if [ "$candidate_mtime" -ge "$latest_mtime" ]; then
      latest_path="$candidate/SourcePackages"
      latest_mtime=$candidate_mtime
    fi
  done

  if [ -n "$latest_path" ]; then
    printf '%s\n' "$latest_path"
    return 0
  fi
  return 1
}

if [ -n "$SOURCE_PACKAGES_DIR" ]; then
  resolved_source_packages_dir=$SOURCE_PACKAGES_DIR
elif [ -n "$DERIVED_DATA_PATH" ]; then
  resolved_source_packages_dir="$DERIVED_DATA_PATH/SourcePackages"
else
  resolved_source_packages_dir=$(find_existing_source_packages_dir || true)
fi

artifact_root=""
download_tmp_root=""
cleanup() {
  if [ -n "$download_tmp_root" ] && [ -d "$download_tmp_root" ]; then
    rm -rf -- "$download_tmp_root"
  fi
}
trap cleanup EXIT INT TERM

prepare_artifact_cache() {
  artifact_root=$1
  if ! mkdir -p "$artifact_root"; then
    echo "[WebRTC] 无法创建 artifact 缓存目录: $artifact_root" >&2
    exit 1
  fi

  cached_count=$(find "$artifact_root" -type d -name WebRTC.xcframework -print | wc -l | tr -d '[:space:]')
  if [ "$cached_count" -eq 1 ]; then
    cached_xcframework=$(find "$artifact_root" -type d -name WebRTC.xcframework -print -quit)
    echo "[WebRTC] 命中 XCFramework 缓存: $cached_xcframework"
    return 0
  fi
  if [ "$cached_count" -gt 1 ]; then
    echo "[WebRTC] 缓存中发现多个 WebRTC.xcframework ($cached_count): $artifact_root" >&2
    echo "[WebRTC] 请清理旧的 SourcePackages/artifacts 后重试" >&2
    exit 1
  fi

  artifact_target="$artifact_root/webrtc/WebRTC"
  if [ -d "$artifact_target" ] && [ "$(find "$artifact_target" -mindepth 1 -print -quit)" ]; then
    echo "[WebRTC] 发现未完成的 artifact 缓存: $artifact_target" >&2
    echo "[WebRTC] 请删除该目录后重试，脚本不会覆盖未知的残留文件" >&2
    exit 1
  fi

  if ! mkdir -p "$artifact_target"; then
    echo "[WebRTC] 无法创建 artifact 目标目录: $artifact_target" >&2
    exit 1
  fi

  require_command "$CURL_BIN"
  require_command "$UNZIP_BIN"
  require_command "$SHASUM_BIN"
  if ! download_tmp_root=$(mktemp -d "$artifact_root/.webrtc-download.XXXXXX"); then
    echo "[WebRTC] 无法在 artifact 缓存中创建下载临时目录: $artifact_root" >&2
    exit 1
  fi
  archive="$download_tmp_root/WebRTC.xcframework.zip"
  expected_size_text=""
  if [ -n "$ARTIFACT_SIZE_BYTES" ]; then
    expected_size_text=" ($(format_bytes "$ARTIFACT_SIZE_BYTES"))"
  fi

  echo "[WebRTC] 未找到 XCFramework 缓存"
  echo "[WebRTC] 下载固定 artifact$expected_size_text"
  echo "[WebRTC] URL: $ARTIFACT_URL"
  if ! "$CURL_BIN" \
    --fail \
    --location \
    --show-error \
    --progress-bar \
    --retry 3 \
    --retry-all-errors \
    --retry-delay 1 \
    --connect-timeout 15 \
    --speed-limit 1024 \
    --speed-time 30 \
    --max-time "$DOWNLOAD_TIMEOUT_SECONDS" \
    --output "$archive" \
    "$ARTIFACT_URL"; then
    echo "[WebRTC] 下载失败: $ARTIFACT_URL" >&2
    echo "[WebRTC] 请检查网络、代理、DNS 和 GitHub Release 可达性；下载超时为 ${DOWNLOAD_TIMEOUT_SECONDS}s" >&2
    exit 1
  fi

  actual_size=$(wc -c < "$archive" | tr -d '[:space:]')
  if [ -n "$ARTIFACT_SIZE_BYTES" ] && [ "$actual_size" -ne "$ARTIFACT_SIZE_BYTES" ]; then
    echo "[WebRTC] 下载文件大小不匹配: 期望 $ARTIFACT_SIZE_BYTES，实际 $actual_size" >&2
    exit 1
  fi

  actual_checksum=$("$SHASUM_BIN" -a 256 "$archive" | awk '{print $1}')
  if [ "$actual_checksum" != "$ARTIFACT_CHECKSUM" ]; then
    echo "[WebRTC] ZIP checksum 不匹配: 期望 $ARTIFACT_CHECKSUM，实际 $actual_checksum" >&2
    exit 1
  fi
  echo "[WebRTC] 下载完成，checksum 校验通过"

  extracted_root="$download_tmp_root/extracted"
  mkdir -p "$extracted_root"
  if ! "$UNZIP_BIN" -q "$archive" -d "$extracted_root"; then
    echo "[WebRTC] ZIP 解压失败: $archive" >&2
    exit 1
  fi

  extracted_count=$(find "$extracted_root" -type d -name WebRTC.xcframework -print | wc -l | tr -d '[:space:]')
  if [ "$extracted_count" -ne 1 ]; then
    echo "[WebRTC] ZIP 中的 WebRTC.xcframework 数量为 $extracted_count，期望 1" >&2
    exit 1
  fi
  extracted_xcframework=$(find "$extracted_root" -type d -name WebRTC.xcframework -print -quit)
  if ! mv "$extracted_xcframework" "$artifact_target/WebRTC.xcframework"; then
    echo "[WebRTC] 无法写入 SwiftPM artifact 缓存: $artifact_target/WebRTC.xcframework" >&2
    exit 1
  fi
  echo "[WebRTC] artifact 已写入缓存: $artifact_target/WebRTC.xcframework"
}

run_package_resolution() {
  require_command "$XCODEBUILD_BIN"
  echo "[WebRTC] 开始 SwiftPM package resolve"
  if [ -n "$SOURCE_PACKAGES_DIR" ] && [ -n "$DERIVED_DATA_PATH" ]; then
    "$XCODEBUILD_BIN" \
      -verbose \
      -resolvePackageDependencies \
      -project "$PROJECT" \
      -scheme "$SCHEME" \
      -derivedDataPath "$DERIVED_DATA_PATH" \
      -clonedSourcePackagesDirPath "$SOURCE_PACKAGES_DIR"
  elif [ -n "$SOURCE_PACKAGES_DIR" ]; then
    "$XCODEBUILD_BIN" \
      -verbose \
      -resolvePackageDependencies \
      -project "$PROJECT" \
      -scheme "$SCHEME" \
      -clonedSourcePackagesDirPath "$SOURCE_PACKAGES_DIR"
  elif [ -n "$DERIVED_DATA_PATH" ]; then
    "$XCODEBUILD_BIN" \
      -verbose \
      -resolvePackageDependencies \
      -project "$PROJECT" \
      -scheme "$SCHEME" \
      -derivedDataPath "$DERIVED_DATA_PATH"
  elif [ -n "$resolved_source_packages_dir" ]; then
    "$XCODEBUILD_BIN" \
      -verbose \
      -resolvePackageDependencies \
      -project "$PROJECT" \
      -scheme "$SCHEME" \
      -clonedSourcePackagesDirPath "$resolved_source_packages_dir"
  else
    echo "[WebRTC] 尚未找到已有 DerivedData，Xcode 将使用默认缓存路径"
    "$XCODEBUILD_BIN" \
      -verbose \
      -resolvePackageDependencies \
      -project "$PROJECT" \
      -scheme "$SCHEME"
  fi
}

case "$MODE" in
  resolve)
    if [ -n "$SOURCE_PACKAGES_DIR" ]; then
      if ! mkdir -p "$SOURCE_PACKAGES_DIR"; then
        echo "[WebRTC] 无法创建 SourcePackages 目录: $SOURCE_PACKAGES_DIR" >&2
        exit 1
      fi
    fi
    if [ -n "$DERIVED_DATA_PATH" ]; then
      if ! mkdir -p "$DERIVED_DATA_PATH"; then
        echo "[WebRTC] 无法创建 DerivedData 目录: $DERIVED_DATA_PATH" >&2
        exit 1
      fi
    fi

    if [ -n "$resolved_source_packages_dir" ]; then
      prepare_artifact_cache "$resolved_source_packages_dir/artifacts"
    else
      echo "[WebRTC] 无法在 resolve 前定位 Xcode DerivedData，保留 Xcode 默认解析流程"
      echo "[WebRTC] 如需下载进度条，请设置 SOURCE_PACKAGES_DIR 或 DERIVED_DATA_PATH"
    fi

    if ! run_package_resolution; then
      echo "[WebRTC] SwiftPM package resolve 失败" >&2
      echo "[WebRTC] URL: $ARTIFACT_URL" >&2
      if [ -n "$resolved_source_packages_dir" ]; then
        echo "[WebRTC] SourcePackages: $resolved_source_packages_dir" >&2
      fi
      echo "[WebRTC] 请检查 SwiftPM/Clang 缓存写权限以及网络连接" >&2
      exit 1
    fi
    echo "[WebRTC] SwiftPM package resolve 完成"
    ;;
  --verify-only)
    ;;
  *)
    echo "用法: $0 [--verify-only]" >&2
    exit 2
    ;;
esac

if [ -z "$resolved_source_packages_dir" ]; then
  require_command "$XCODEBUILD_BIN"
  settings_output=$(mktemp "${TMPDIR:-/tmp}/xbxrc-build-settings.XXXXXX")
  settings_error=$(mktemp "${TMPDIR:-/tmp}/xbxrc-build-settings-error.XXXXXX")
  if ! "$XCODEBUILD_BIN" \
    -project "$PROJECT" \
    -scheme "$SCHEME" \
    -destination 'generic/platform=iOS' \
    -showBuildSettings >"$settings_output" 2>"$settings_error"; then
    echo "无法定位 Xcode 默认 DerivedData" >&2
    sed -n '1,80p' "$settings_error" >&2
    rm -f "$settings_output" "$settings_error"
    exit 1
  fi
  build_dir=$(awk -F ' = ' '/^[[:space:]]*BUILD_DIR = / { print $2; exit }' "$settings_output")
  rm -f "$settings_output" "$settings_error"
  if [ -z "$build_dir" ]; then
    echo "无法定位 Xcode 默认 DerivedData: xcodebuild 未返回 BUILD_DIR" >&2
    exit 1
  fi
  resolved_source_packages_dir="${build_dir%/Build/Products}/SourcePackages"
fi

artifact_root="$resolved_source_packages_dir/artifacts"
if [ ! -d "$artifact_root" ]; then
  echo "缺少 SwiftPM artifact 缓存目录: $artifact_root" >&2
  echo "请先用默认 resolve 模式下载 WebRTC，或设置 SOURCE_PACKAGES_DIR/DERIVED_DATA_PATH" >&2
  exit 1
fi

artifact_count=$(find "$artifact_root" -type d -name WebRTC.xcframework -print | wc -l | tr -d '[:space:]')
if [ "$artifact_count" -eq 0 ]; then
  echo "未找到 WebRTC.xcframework 缓存: $artifact_root" >&2
  echo "请先用默认 resolve 模式完成下载" >&2
  exit 1
fi
if [ "$artifact_count" -gt 1 ]; then
  echo "WebRTC.xcframework 缓存数量为 $artifact_count，期望 1: $artifact_root" >&2
  exit 1
fi

xcframework=$(find "$artifact_root" -type d -name WebRTC.xcframework -print -quit)
echo "[WebRTC] 开始校验 XCFramework: $xcframework"
info_plist="$xcframework/Info.plist"
if [ ! -f "$info_plist" ]; then
  echo "WebRTC.xcframework 缺少 Info.plist" >&2
  exit 1
fi

device_identifier=""
device_library_path=""
simulator_identifier=""
simulator_library_path=""
index=0
while platform=$(/usr/bin/plutil -extract "AvailableLibraries.$index.SupportedPlatform" raw -o - "$info_plist" 2>/dev/null); do
  identifier=$(/usr/bin/plutil -extract "AvailableLibraries.$index.LibraryIdentifier" raw -o - "$info_plist")
  library_path=$(/usr/bin/plutil -extract "AvailableLibraries.$index.LibraryPath" raw -o - "$info_plist")
  variant=$(/usr/bin/plutil -extract "AvailableLibraries.$index.SupportedPlatformVariant" raw -o - "$info_plist" 2>/dev/null || true)
  architectures=$(/usr/bin/plutil -extract "AvailableLibraries.$index.SupportedArchitectures" json -o - "$info_plist")

  if [ "$platform" = "ios" ] && printf '%s' "$architectures" | grep -q '"arm64"'; then
    if [ "$variant" = "simulator" ]; then
      simulator_identifier=$identifier
      simulator_library_path=$library_path
    elif [ -z "$variant" ]; then
      device_identifier=$identifier
      device_library_path=$library_path
    fi
  fi

  index=$((index + 1))
done

if [ -z "$device_identifier" ] || [ -z "$simulator_identifier" ]; then
  echo "WebRTC.xcframework 必须同时包含 iOS Device arm64 与 Simulator arm64" >&2
  exit 1
fi

verify_slice() {
  identifier=$1
  library_path=$2
  expected_sha=$3
  slice="$xcframework/$identifier/$library_path"
  binary="$slice/WebRTC"
  module_map="$slice/Modules/module.modulemap"

  if [ ! -f "$binary" ] || [ ! -f "$slice/Headers/WebRTC.h" ]; then
    echo "WebRTC slice 缺少二进制或 Headers: $identifier" >&2
    exit 1
  fi

  if [ ! -f "$module_map" ] || ! grep -Eq '(framework[[:space:]]+)?module[[:space:]]+WebRTC' "$module_map"; then
    echo "WebRTC slice 未暴露 WebRTC module: $identifier" >&2
    exit 1
  fi

  if [ ! -f "$slice/PrivacyInfo.xcprivacy" ]; then
    echo "WebRTC slice 缺少 PrivacyInfo.xcprivacy: $identifier" >&2
    exit 1
  fi

  actual_sha=$("$SHASUM_BIN" -a 256 "$binary" | awk '{print $1}')
  if [ "$actual_sha" != "$expected_sha" ]; then
    echo "WebRTC 二进制 SHA-256 不匹配: $identifier" >&2
    exit 1
  fi
}

require_command "$SHASUM_BIN"
verify_slice "$device_identifier" "$device_library_path" "$EXPECTED_DEVICE_SHA"
verify_slice "$simulator_identifier" "$simulator_library_path" "$EXPECTED_SIMULATOR_SHA"

echo "WebRTC artifact 校验通过"
echo "  version: $VERSION"
echo "  url: $ARTIFACT_URL"
echo "  checksum: $ARTIFACT_CHECKSUM"
echo "  artifact: $xcframework"
echo "  device: $device_identifier"
echo "  simulator: $simulator_identifier"
