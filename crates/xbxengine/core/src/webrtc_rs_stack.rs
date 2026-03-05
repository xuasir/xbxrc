use std::sync::{Arc, Mutex};

use tokio::runtime::Runtime;
use xbxengine_protocol::{XbxEngineDisplayStateDto, XbxEngineIceCandidateDto};

use crate::{
    webrtc_rs_control::{
        WebRtcRsDataChannelMediaControl, WebRtcRsMediaControlContext, WebRtcRsMediaControlPort,
    },
    webrtc_rs_data_channel::WebRtcRsDataChannelState,
    webrtc_rs_render::WebRtcRsRenderState,
    webrtc_rs_transport::WebRtcRsTransportState,
    XbxEngineMediaNegotiationRequest, XbxEngineMediaRuntimeStats, XbxEngineRenderFrame,
    XbxEngineRuntimeConfig, XbxEngineRuntimeError,
};

/**
 * active `webrtc-rs` 媒体栈的组合根：
 * - backend 不再直接持有一堆分散状态
 * - transport / data-channel / video 细节统一从这里编排
 * - control 动作口单独注入，后续补 recovery/render owner 时不回写 stack 主体
 */
pub(crate) trait WebRtcRsMediaStackPort: Send {
    fn rebuild_peer_connection(
        &mut self,
        request: &XbxEngineMediaNegotiationRequest,
    ) -> Result<(), XbxEngineRuntimeError>;
    fn create_offer(&self) -> Result<String, XbxEngineRuntimeError>;
    fn apply_remote_description(
        &self,
        answer_sdp: &str,
        remote_candidates: &[XbxEngineIceCandidateDto],
    ) -> Result<(), XbxEngineRuntimeError>;
    fn apply_display_state(
        &mut self,
        state: XbxEngineDisplayStateDto,
    ) -> Result<(), XbxEngineRuntimeError>;
    fn local_candidates_snapshot(&self) -> Vec<XbxEngineIceCandidateDto>;
    fn snapshot_runtime_stats(&self) -> XbxEngineMediaRuntimeStats;
    fn take_latest_render_frame(&mut self) -> Option<XbxEngineRenderFrame>;
    fn request_video_keyframe(&mut self) -> Result<(), XbxEngineRuntimeError>;
    fn stop(&mut self);
}

pub(crate) struct WebRtcRsActiveMediaStack {
    runtime: Runtime,
    transport: WebRtcRsTransportState,
    control: Box<dyn WebRtcRsMediaControlPort>,
    runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    data_channel_state: Arc<Mutex<WebRtcRsDataChannelState>>,
    render_state: Arc<Mutex<WebRtcRsRenderState>>,
    runtime_config: XbxEngineRuntimeConfig,
}

impl WebRtcRsActiveMediaStack {
    pub(crate) fn new(runtime_config: XbxEngineRuntimeConfig) -> Self {
        Self::with_control(
            Box::<WebRtcRsDataChannelMediaControl>::default(),
            runtime_config,
        )
    }

    pub(crate) fn with_control(
        control: Box<dyn WebRtcRsMediaControlPort>,
        runtime_config: XbxEngineRuntimeConfig,
    ) -> Self {
        Self {
            runtime: tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("build webrtc-rs runtime"),
            transport: WebRtcRsTransportState::default(),
            control,
            runtime_stats: Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default())),
            data_channel_state: Arc::new(Mutex::new(WebRtcRsDataChannelState::default())),
            render_state: Arc::new(Mutex::new(WebRtcRsRenderState::default())),
            runtime_config,
        }
    }
}

impl WebRtcRsMediaStackPort for WebRtcRsActiveMediaStack {
    fn rebuild_peer_connection(
        &mut self,
        request: &XbxEngineMediaNegotiationRequest,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.data_channel_state = Arc::new(Mutex::new(WebRtcRsDataChannelState::default()));
        // 先切断 decode/render 热路径，当前阶段只保留网络稳定器相关链路。
        self.render_state
            .lock()
            .expect("lock render state")
            .reset()?;
        self.transport.rebuild_peer_connection(
            &self.runtime,
            request,
            self.data_channel_state.clone(),
            self.runtime_stats.clone(),
            &self.runtime_config.webrtc,
        )
    }

    fn create_offer(&self) -> Result<String, XbxEngineRuntimeError> {
        self.transport.create_offer(&self.runtime)
    }

    fn apply_remote_description(
        &self,
        answer_sdp: &str,
        remote_candidates: &[XbxEngineIceCandidateDto],
    ) -> Result<(), XbxEngineRuntimeError> {
        self.transport
            .apply_remote_description(&self.runtime, answer_sdp, remote_candidates)
    }

    fn local_candidates_snapshot(&self) -> Vec<XbxEngineIceCandidateDto> {
        self.transport.local_candidates_snapshot()
    }

    fn apply_display_state(
        &mut self,
        state: XbxEngineDisplayStateDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.render_state
            .lock()
            .expect("lock render state")
            .apply_display_state(state)
    }

    fn snapshot_runtime_stats(&self) -> XbxEngineMediaRuntimeStats {
        self.runtime_stats
            .lock()
            .expect("lock runtime stats")
            .clone()
    }

    fn take_latest_render_frame(&mut self) -> Option<XbxEngineRenderFrame> {
        self.render_state
            .lock()
            .ok()
            .and_then(|mut render_state| render_state.take_latest_frame())
    }

    fn request_video_keyframe(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.control
            .request_video_keyframe(WebRtcRsMediaControlContext {
                runtime: &self.runtime,
                transport: &self.transport,
            })
    }

    fn stop(&mut self) {
        self.transport.stop_peer_connection(&self.runtime);
        if let Ok(mut render_state) = self.render_state.lock() {
            render_state.stop();
        }
    }
}
