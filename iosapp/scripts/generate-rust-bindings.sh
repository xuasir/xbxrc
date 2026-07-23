#!/bin/sh
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
OUTPUT_DIR="$REPO_ROOT/iosapp/XBXRC/Platform/RustBridge/Generated"
LIBRARY_PATH="$REPO_ROOT/target/debug/libxbox_ios_bridge.a"

cd "$REPO_ROOT"
cargo build -p xbox-ios-bridge
cargo run -p xbox-ios-bridge --example uniffi-bindgen -- \
  --metadata-no-deps --swift-sources "$LIBRARY_PATH" "$OUTPUT_DIR"
cargo run -p xbox-ios-bridge --example uniffi-bindgen -- \
  --metadata-no-deps --headers "$LIBRARY_PATH" "$OUTPUT_DIR"
cargo run -p xbox-ios-bridge --example uniffi-bindgen -- \
  --metadata-no-deps \
  --modulemap \
  --module-name xbox_ios_bridgeFFI \
  --modulemap-filename xbox_ios_bridge.modulemap \
  "$LIBRARY_PATH" "$OUTPUT_DIR"

sed -i '' -E 's/[[:space:]]+$//' \
  "$OUTPUT_DIR/xbox_ios_bridge.swift" \
  "$OUTPUT_DIR/xbox_ios_bridgeFFI.h"
