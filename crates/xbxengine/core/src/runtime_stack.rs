use crate::{
    WebRtcRsNegotiationBackend, XbxEngineInputBackend, XbxEngineMediaBackend,
    XbxEngineRuntimeConfig,
};

/**
 * 当前主线只保留一条 Rust 媒体栈入口：
 * - Electron/main 当前真正托管的 `webrtc-rs` 协商路径
 * - 不再在产品代码里并行保留 `webrtcbin` reference stack
 * - 后续 decode/render 的替换继续在 active stack 内部演进
 */
pub fn create_active_media_backend(
    input_backend: Box<dyn XbxEngineInputBackend>,
    runtime_config: XbxEngineRuntimeConfig,
) -> Box<dyn XbxEngineMediaBackend> {
    Box::new(WebRtcRsNegotiationBackend::new(
        input_backend,
        runtime_config,
    ))
}
