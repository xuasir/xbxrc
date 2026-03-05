use crate::{
    backend::{
        PlaceholderXbxEngineMediaBackend, XbxEngineMediaBackend, XbxEngineMediaNegotiation,
        XbxEngineMediaNegotiationRequest, XbxEngineMediaRuntimeStats, XbxEngineRenderFrame,
    },
    input::{XbxEngineInputBackend, XbxEngineInputStatus},
    webrtc_rs_negotiation_profile::current_webrtc_rs_negotiation_profile,
    webrtc_rs_stack::{WebRtcRsActiveMediaStack, WebRtcRsMediaStackPort},
    XbxEngineRuntimeConfig, XbxEngineRuntimeError,
};
use xbxengine_protocol::{
    XbxEngineDisplayStateDto, XbxEngineIceCandidateDto, XbxEngineInputEventDto,
};

/**
 * 当前 active stack 只保留 backend 外壳职责：
 * - 持有 placeholder 输入/宿主门面
 * - 组装 webrtc-rs transport / data channel / video / control 子模块
 * - 对外实现统一 media backend trait
 */
pub struct WebRtcRsNegotiationBackend {
    inner: PlaceholderXbxEngineMediaBackend,
    stack: Box<dyn WebRtcRsMediaStackPort>,
}

impl WebRtcRsNegotiationBackend {
    pub fn new(
        input_backend: Box<dyn XbxEngineInputBackend>,
        runtime_config: XbxEngineRuntimeConfig,
    ) -> Self {
        Self {
            inner: PlaceholderXbxEngineMediaBackend::with_input_backend(input_backend),
            stack: Box::new(WebRtcRsActiveMediaStack::new(runtime_config)),
        }
    }

    fn rebuild_peer_connection(
        &mut self,
        request: &XbxEngineMediaNegotiationRequest,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.stack.rebuild_peer_connection(request)
    }

    fn create_offer(&self) -> Result<String, XbxEngineRuntimeError> {
        self.stack.create_offer()
    }

    fn apply_remote_description_to_peer_connection(
        &self,
        answer_sdp: &str,
        remote_candidates: &[XbxEngineIceCandidateDto],
    ) -> Result<(), XbxEngineRuntimeError> {
        self.stack
            .apply_remote_description(answer_sdp, remote_candidates)
    }

    fn stop_peer_connection(&mut self) {
        self.stack.stop();
    }
}

impl XbxEngineMediaBackend for WebRtcRsNegotiationBackend {
    fn negotiate(
        &mut self,
        request: XbxEngineMediaNegotiationRequest,
    ) -> Result<XbxEngineMediaNegotiation, XbxEngineRuntimeError> {
        let profile = current_webrtc_rs_negotiation_profile();
        let mut negotiation = self.inner.negotiate(request.clone())?;
        self.rebuild_peer_connection(&request)?;
        negotiation.local_offer_sdp = self.create_offer()?;
        negotiation.local_candidates = self.stack.local_candidates_snapshot();
        negotiation.surface_id = format!("wgpu:{}", request.viewport.viewport_id);
        negotiation.video_width = profile.width;
        negotiation.video_height = profile.height;
        // 首帧统计必须等真实 RTP/解码完成后再更新，避免协商阶段伪造 readiness。
        negotiation.first_frame_packet_arrival_time_ms = None;
        negotiation.frame_decoded_time_ms = None;
        negotiation.frame_rendered_time_ms = None;
        Ok(negotiation)
    }

    fn apply_remote_description(
        &mut self,
        answer_sdp: String,
        remote_candidates: Vec<XbxEngineIceCandidateDto>,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.inner
            .apply_remote_description(answer_sdp.clone(), remote_candidates.clone())?;
        self.apply_remote_description_to_peer_connection(&answer_sdp, &remote_candidates)
    }

    fn apply_display_state(
        &mut self,
        state: XbxEngineDisplayStateDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.inner.apply_display_state(state.clone())?;
        self.stack.apply_display_state(state)
    }

    fn set_audio_volume(&mut self, value: f32) -> Result<(), XbxEngineRuntimeError> {
        self.inner.set_audio_volume(value)
    }

    fn set_microphone_capturing(&mut self, capturing: bool) -> Result<(), XbxEngineRuntimeError> {
        self.inner.set_microphone_capturing(capturing)
    }

    fn press_controller_button(
        &mut self,
        button: String,
        duration_ms: u64,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.inner.press_controller_button(button, duration_ms)
    }

    fn set_keyboard_pointer_enabled(&mut self, enabled: bool) -> Result<(), XbxEngineRuntimeError> {
        self.inner.set_keyboard_pointer_enabled(enabled)
    }

    fn push_keyboard_pointer_input(
        &mut self,
        event: XbxEngineInputEventDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.inner.push_keyboard_pointer_input(event)
    }

    fn current_input_status(&self) -> Result<XbxEngineInputStatus, XbxEngineRuntimeError> {
        self.inner.current_input_status()
    }

    fn snapshot_runtime_stats(&self) -> Result<XbxEngineMediaRuntimeStats, XbxEngineRuntimeError> {
        Ok(self.stack.snapshot_runtime_stats())
    }

    fn take_latest_render_frame(
        &mut self,
    ) -> Result<Option<XbxEngineRenderFrame>, XbxEngineRuntimeError> {
        Ok(self.stack.take_latest_render_frame())
    }

    fn request_video_keyframe(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.stack.request_video_keyframe()
    }

    fn stop(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.stop_peer_connection();
        self.inner.stop()
    }
}
