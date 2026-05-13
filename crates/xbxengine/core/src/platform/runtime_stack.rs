use std::sync::Arc;

use crate::{
    XbxEngineInputBackend, XbxEngineMediaBackend, XbxEngineRuntimeConfig, XbxHostRenderFramePush,
    XbxNegotiationBackend,
};

/**
 * 当前主线只保留一条 Rust 媒体栈入口：
 * - 当前宿主统一接入 `rtc` sans-io 协商路径
 * - 不再在产品代码里并行保留旧 reference stack
 * - 后续 decode/render 的替换继续在 active stack 内部演进
 */
pub fn create_active_media_backend(
    input_backend: Box<dyn XbxEngineInputBackend>,
    runtime_config: XbxEngineRuntimeConfig,
    host_render_frame_push: Option<Arc<dyn XbxHostRenderFramePush>>,
) -> Box<dyn XbxEngineMediaBackend> {
    Box::new(XbxNegotiationBackend::new(
        input_backend,
        runtime_config,
        host_render_frame_push,
    ))
}
