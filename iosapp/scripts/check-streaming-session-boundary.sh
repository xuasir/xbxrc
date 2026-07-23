#!/bin/sh
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
RUST_SESSION="$REPO_ROOT/crates/xbox-ios-bridge/src/streaming.rs"
SWIFT_RUNTIME="$REPO_ROOT/iosapp/XBXRC/Platform/Streaming/StreamingRuntime.swift"
SWIFT_ACTOR="$REPO_ROOT/iosapp/XBXRC/Platform/Streaming/StreamSessionActor.swift"
GENERATED_SWIFT="$REPO_ROOT/iosapp/XBXRC/Platform/RustBridge/Generated/xbox_ios_bridge.swift"

require_pattern() {
  pattern="$1"
  file="$2"
  message="$3"
  if ! rg -q "$pattern" "$file"; then
    echo "$message" >&2
    exit 1
  fi
}

reject_pattern() {
  pattern="$1"
  file="$2"
  message="$3"
  if rg -q "$pattern" "$file"; then
    echo "$message" >&2
    exit 1
  fi
}

require_pattern 'pub fn create_stream_session' "$RUST_SESSION" 'Swift 只能使用统一 Rust session factory'
require_pattern 'let access = load_stream_access' "$RUST_SESSION" 'Rust session factory 必须从 access handle 解析权威 target'
require_pattern 'type IosSessionFlow = SessionFlowService' "$RUST_SESSION" 'iOS session 必须复用 SessionFlowService'
require_pattern 'pub async fn next_remote_ice_batch' "$RUST_SESSION" '远端 ICE 拉取策略必须由 Rust session 持有'

require_pattern 'createScopedStreamSession' "$SWIFT_RUNTIME" 'Swift bridge 必须使用带 target/account/generation 校验的 Rust session factory'
require_pattern 'targetType: request\.target' "$SWIFT_RUNTIME" 'Swift bridge 必须显式投影 cloud/home target'
require_pattern 'ownerGeneration: request\.ownerGeneration' "$SWIFT_RUNTIME" 'Swift bridge 必须传递 access lease generation'
reject_pattern 'createStreamSession\(' "$SWIFT_RUNTIME" 'Swift bridge 不得回退未作用域的 session factory'
require_pattern 'session\.nextRemoteIceBatch' "$SWIFT_RUNTIME" 'Swift bridge 必须消费 Rust-owned remote ICE batch'
require_pattern 'peer\.addRemoteCandidates' "$SWIFT_ACTOR" 'Swift actor 必须只负责向 libwebrtc 应用远端 ICE'

reject_pattern 'create_home_stream_session' "$RUST_SESSION" 'Rust 不应维护平行 home session factory'
reject_pattern 'createHomeStreamSession' "$SWIFT_RUNTIME" 'Swift 不得维护目标专用 session factory'
reject_pattern 'session\.snapshot\(' "$SWIFT_RUNTIME" 'Swift bridge 不得轮询 Rust session snapshot'
reject_pattern 'Task\.sleep|emptyIcePolls|observedRemoteIce|icePollInterval' "$SWIFT_RUNTIME" 'Swift bridge 不得持有远端 ICE 轮询策略'
reject_pattern 'emptyIcePolls|observedRemoteIce|icePollInterval' "$SWIFT_ACTOR" 'Swift actor 不得持有远端 ICE 轮询策略'
reject_pattern 'func snapshot\(' "$GENERATED_SWIFT" 'UniFFI 不应向 Swift 导出 session snapshot 轮询接口'
reject_pattern 'func pollIce\(' "$GENERATED_SWIFT" 'UniFFI 不应向 Swift 导出单次 ICE poll 接口'
reject_pattern 'func keepAlive\(' "$GENERATED_SWIFT" 'UniFFI 不应向 Swift 导出 keepalive 控制接口'
require_pattern 'func releaseStreamAccess\(' "$GENERATED_SWIFT" 'UniFFI 必须使用统一 stream access release 接口'
reject_pattern 'func releaseCloudAccess\(|func releaseHomeAccess\(' "$GENERATED_SWIFT" 'UniFFI 不应维护目标专用 access release 接口'

echo 'streaming session boundary check passed'
