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
EXPECTED_DEVICE_SHA=$(/usr/bin/plutil -extract artifact.binarySHA256.iosDeviceArm64 raw -o - "$ARTIFACT_MANIFEST")
EXPECTED_SIMULATOR_SHA=$(/usr/bin/plutil -extract artifact.binarySHA256.iosSimulatorUniversal raw -o - "$ARTIFACT_MANIFEST")
SOURCE_PACKAGES_DIR=${SOURCE_PACKAGES_DIR:-}
DERIVED_DATA_PATH=${DERIVED_DATA_PATH:-}

if ! grep -Fq "$ARTIFACT_URL" "$PACKAGE_MANIFEST"; then
  echo "Package.swift 的 WebRTC URL 与 artifact manifest 不一致" >&2
  exit 1
fi

if ! grep -Fq "$ARTIFACT_CHECKSUM" "$PACKAGE_MANIFEST"; then
  echo "Package.swift 的 WebRTC checksum 与 artifact manifest 不一致" >&2
  exit 1
fi

case "$MODE" in
  resolve)
    if [ -n "$SOURCE_PACKAGES_DIR" ]; then
      mkdir -p "$SOURCE_PACKAGES_DIR"
    fi
    if [ -n "$DERIVED_DATA_PATH" ]; then
      mkdir -p "$DERIVED_DATA_PATH"
    fi

    if [ -n "$SOURCE_PACKAGES_DIR" ] && [ -n "$DERIVED_DATA_PATH" ]; then
      xcodebuild \
        -resolvePackageDependencies \
        -project "$PROJECT" \
        -scheme "$SCHEME" \
        -derivedDataPath "$DERIVED_DATA_PATH" \
        -clonedSourcePackagesDirPath "$SOURCE_PACKAGES_DIR"
    elif [ -n "$SOURCE_PACKAGES_DIR" ]; then
      xcodebuild \
        -resolvePackageDependencies \
        -project "$PROJECT" \
        -scheme "$SCHEME" \
        -clonedSourcePackagesDirPath "$SOURCE_PACKAGES_DIR"
    elif [ -n "$DERIVED_DATA_PATH" ]; then
      xcodebuild \
        -resolvePackageDependencies \
        -project "$PROJECT" \
        -scheme "$SCHEME" \
        -derivedDataPath "$DERIVED_DATA_PATH"
    else
      # 开发默认复用 Xcode GUI 的 DerivedData，解析后可直接在 Xcode 中构建。
      xcodebuild \
        -resolvePackageDependencies \
        -project "$PROJECT" \
        -scheme "$SCHEME"
    fi
    ;;
  --verify-only)
    ;;
  *)
    echo "用法: $0 [--verify-only]" >&2
    exit 2
    ;;
esac

if [ -n "$SOURCE_PACKAGES_DIR" ]; then
  resolved_source_packages_dir=$SOURCE_PACKAGES_DIR
elif [ -n "$DERIVED_DATA_PATH" ]; then
  resolved_source_packages_dir="$DERIVED_DATA_PATH/SourcePackages"
else
  build_dir=$(xcodebuild \
    -project "$PROJECT" \
    -scheme "$SCHEME" \
    -destination 'generic/platform=iOS' \
    -showBuildSettings 2>/dev/null \
    | awk -F ' = ' '/^[[:space:]]*BUILD_DIR = / { print $2; exit }')
  if [ -z "$build_dir" ]; then
    echo "无法定位 Xcode 默认 DerivedData" >&2
    exit 1
  fi
  resolved_source_packages_dir="${build_dir%/Build/Products}/SourcePackages"
fi

artifact_root="$resolved_source_packages_dir/artifacts"
if [ ! -d "$artifact_root" ]; then
  echo "缺少 SwiftPM artifact 缓存: $artifact_root" >&2
  exit 1
fi

artifact_count=$(find "$artifact_root" -type d -name WebRTC.xcframework -print | wc -l | tr -d '[:space:]')
if [ "$artifact_count" -ne 1 ]; then
  echo "WebRTC.xcframework 数量必须为 1，当前为 $artifact_count: $artifact_root" >&2
  exit 1
fi

xcframework=$(find "$artifact_root" -type d -name WebRTC.xcframework -print -quit)
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

  actual_sha=$(/usr/bin/shasum -a 256 "$binary" | awk '{print $1}')
  if [ "$actual_sha" != "$expected_sha" ]; then
    echo "WebRTC 二进制 SHA-256 不匹配: $identifier" >&2
    exit 1
  fi
}

verify_slice "$device_identifier" "$device_library_path" "$EXPECTED_DEVICE_SHA"
verify_slice "$simulator_identifier" "$simulator_library_path" "$EXPECTED_SIMULATOR_SHA"

echo "WebRTC artifact 校验通过"
echo "  version: $VERSION"
echo "  url: $ARTIFACT_URL"
echo "  checksum: $ARTIFACT_CHECKSUM"
echo "  artifact: $xcframework"
echo "  device: $device_identifier"
echo "  simulator: $simulator_identifier"
