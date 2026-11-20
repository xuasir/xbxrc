#!/bin/sh
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
RUST_SESSION="$REPO_ROOT/crates/xbox-ios-bridge/src/streaming.rs"
SWIFT_RUNTIME="$REPO_ROOT/iosapp/XBXRC/Platform/Streaming/StreamingRuntime.swift"
SWIFT_ACTOR="$REPO_ROOT/iosapp/XBXRC/Platform/Streaming/StreamSessionActor.swift"
SWIFT_STREAMING_DIR="$REPO_ROOT/iosapp/XBXRC/Platform/Streaming"
SWIFT_PEER="$SWIFT_STREAMING_DIR/LibWebRtcRuntime.swift"
SWIFT_PLAYER="$SWIFT_STREAMING_DIR/StreamingPlayerView.swift"
SWIFT_CONTRACTS="$SWIFT_STREAMING_DIR/StreamingContracts.swift"
SWIFT_RENDERER="$SWIFT_STREAMING_DIR/StreamingMetalRenderer.swift"
GENERATED_SWIFT="$REPO_ROOT/iosapp/XBXRC/Platform/RustBridge/Generated/xbox_ios_bridge.swift"

require_pattern() {
  pattern="$1"
  file="$2"
  message="$3"
  if ! rg -U -q "$pattern" "$file"; then
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
require_pattern 'start_session_execution_with_scope' "$RUST_SESSION" 'iOS 启动取消必须进入共享 SessionStartupScope'
require_pattern 'session\.startupProgress' "$SWIFT_RUNTIME" 'Swift 必须消费 Rust 的安全启动进度投影'
require_pattern 'resolveCloudGame' "$SWIFT_CONTRACTS" '云游戏开始前必须复核当前商品启动身份'
require_pattern 'pub async fn next_remote_ice_batch' "$RUST_SESSION" '远端 ICE 拉取策略必须由 Rust session 持有'
require_pattern 'async fn recover_streaming_token' "$RUST_SESSION" 'iOS session 必须接入公共401恢复入口'
require_pattern 'configuration_pending' "$RUST_SESSION" '远端ICE完成必须携带当前configuration事实'
reject_pattern 'console_address_ice_candidates|REMOTE_ICE_EMPTY_POLLS_AFTER_FIRST_CANDIDATE' "$RUST_SESSION" '主机候选绑定和完成判断必须消费公共信令事实'

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

require_pattern '^@preconcurrency import WebRTC' "$SWIFT_PEER" 'iOS 串流必须直接依赖 libwebrtc'
require_pattern '^@preconcurrency import WebRTC' "$SWIFT_RENDERER" '自定义 Metal renderer 必须消费 libwebrtc 解码帧'
require_pattern 'peerFactory: LibWebRTCPeerRuntimeFactory' "$SWIFT_RUNTIME" '生产串流入口必须装配唯一 libwebrtc runtime'
require_pattern 'StreamingFrameFeedbackSession\(\s*preventResolutionDrops:' "$SWIFT_PEER" '常规和清晰度模式必须共用持续帧反馈'
require_pattern 'session\.superviseRuntime' "$SWIFT_RUNTIME" 'Swift 必须消费公共Rust监督合同'
require_pattern 'control\.supervise' "$SWIFT_ACTOR" '原生恢复动作必须消费公共监督结果'
require_pattern 'let rawValue: RTCVideoTrack' "$SWIFT_CONTRACTS" '视频轨道必须保留强类型 RTCVideoTrack'
require_pattern 'StreamingMetalVideoSurfaceView' "$SWIFT_PLAYER" '播放器必须装配自定义 Metal surface'
require_pattern 'nextTrack\.add\(view\.renderer\)' "$SWIFT_PLAYER" '视频轨道必须接入自定义 renderer'
reject_pattern 'canImport\((WebRTC|MetalKit)\)|UnavailableStreamingPeerRuntime|webRTCUnavailable' "$SWIFT_STREAMING_DIR" 'iOS 串流必须在构建时满足 WebRTC/Metal 依赖'
reject_pattern 'rawValue: AnyObject|rawValue as\? RTCVideoTrack|trackTypeMismatch' "$SWIFT_STREAMING_DIR" '视频轨道必须沿强类型原生身份传递'
reject_pattern 'RTCMTLVideoView|RTCEAGLVideoView|WKWebView|AVSampleBufferDisplayLayer|VTDecompressionSession|import (AVKit|WebKit)|ffmpeg|xbxengine' "$SWIFT_STREAMING_DIR" 'iOS 串流必须沿单一 libwebrtc 解码与自定义 Metal 上屏路径'

echo 'streaming session boundary check passed'
