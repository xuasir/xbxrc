#!/bin/sh
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
OUTPUT_DIR="$REPO_ROOT/iosapp/XBXRC/Platform/RustBridge/Generated"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target/ios-bindgen}"
LIBRARY_PATH="$CARGO_TARGET_DIR/debug/libxbox_ios_bridge.a"

cd "$REPO_ROOT"
cargo build -p xbox-ios-bridge
cargo run -p xbox-ios-bridge --example uniffi-bindgen -- \
  --metadata-no-deps --swift-sources "$LIBRARY_PATH" "$OUTPUT_DIR"
python3 "$SCRIPT_DIR/patch-uniffi-swift-concurrency.py" \
  "$OUTPUT_DIR/xbox_ios_bridge.swift"
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
