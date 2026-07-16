#!/bin/sh
set -eu

REPO_ROOT="$(cd "$SRCROOT/.." && pwd)"
OUTPUT_DIR="$DERIVED_FILE_DIR/rust"

CARGO_BIN="${CARGO:-}"
if [ -z "$CARGO_BIN" ]; then
  CARGO_BIN="$(command -v cargo 2>/dev/null || true)"
fi
if [ -z "$CARGO_BIN" ] && [ -x "$HOME/.cargo/bin/cargo" ]; then
  CARGO_BIN="$HOME/.cargo/bin/cargo"
fi
if [ -z "$CARGO_BIN" ]; then
  echo "Rust cargo was not found. Install Rust with rustup or set CARGO to its absolute path." >&2
  exit 1
fi

case "$PLATFORM_NAME" in
  iphoneos)
    RUST_TARGET="aarch64-apple-ios"
    ;;
  iphonesimulator)
    RUST_TARGET="aarch64-apple-ios-sim"
    ;;
  *)
    echo "Unsupported Apple platform: $PLATFORM_NAME" >&2
    exit 1
    ;;
esac

if [ "$CONFIGURATION" = "Release" ]; then
  CARGO_PROFILE="release"
  CARGO_PROFILE_FLAG="--release"
else
  CARGO_PROFILE="debug"
  CARGO_PROFILE_FLAG=""
fi

export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-26.0}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target/ios}"

"$CARGO_BIN" build \
  --manifest-path "$REPO_ROOT/Cargo.toml" \
  -p xbox-ios-bridge \
  --target "$RUST_TARGET" \
  $CARGO_PROFILE_FLAG

mkdir -p "$OUTPUT_DIR"
cp "$CARGO_TARGET_DIR/$RUST_TARGET/$CARGO_PROFILE/libxbox_ios_bridge.a" \
  "$OUTPUT_DIR/libxbox_ios_bridge.a"
