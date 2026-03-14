use crate::{
    api::backend::{
        PlaceholderXbxEngineMediaBackend, XbxEngineMediaBackend, XbxEngineMediaNegotiation,
        XbxEngineMediaNegotiationRequest, XbxEngineMediaRuntimeStats, XbxEngineRenderFrame,
    },
    api::input::{XbxEngineInputBackend, XbxEngineInputStatus},
    api::runtime::XbxEngineNegotiationRuntimeConfig,
    transport::webrtc::stack::{XbxActiveMediaStack, XbxMediaStackPort},
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
pub struct XbxNegotiationBackend {
    inner: PlaceholderXbxEngineMediaBackend,
    stack: Box<dyn XbxMediaStackPort>,
    negotiation_config: XbxEngineNegotiationRuntimeConfig,
}

impl XbxNegotiationBackend {
    pub fn new(
        input_backend: Box<dyn XbxEngineInputBackend>,
        runtime_config: XbxEngineRuntimeConfig,
    ) -> Self {
        let negotiation_config = runtime_config.webrtc.negotiation.clone();
        Self {
            inner: PlaceholderXbxEngineMediaBackend::with_input_backend(input_backend),
            stack: Box::new(XbxActiveMediaStack::new(runtime_config)),
            negotiation_config,
        }
    }

    fn rebuild_peer_connection(
        &mut self,
        request: &XbxEngineMediaNegotiationRequest,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.stack.rebuild_peer_connection(request)
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

impl XbxEngineMediaBackend for XbxNegotiationBackend {
    fn sync_runtime_config(
        &mut self,
        runtime_config: &XbxEngineRuntimeConfig,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.negotiation_config = runtime_config.webrtc.negotiation.clone();
        self.stack.sync_runtime_config(runtime_config.clone());
        Ok(())
    }

    fn negotiate(
        &mut self,
        request: XbxEngineMediaNegotiationRequest,
    ) -> Result<XbxEngineMediaNegotiation, XbxEngineRuntimeError> {
        let mut negotiation = self.inner.negotiate(request.clone())?;
        self.rebuild_peer_connection(&request)?;
        negotiation.local_offer_sdp = self.stack.create_offer()?;
        negotiation.local_candidates = self.stack.local_candidates_snapshot();
        negotiation.surface_id = format!("wgpu:{}", request.viewport.viewport_id);
        negotiation.video_width = self.negotiation_config.target_resolution_width;
        negotiation.video_height = self.negotiation_config.target_resolution_height;
        // 首帧统计必须等真实 RTP/解码完成后再更新，避免协商阶段伪造 readiness。
        negotiation.first_frame_packet_arrival_time_ms = None;
        negotiation.frame_decoded_time_ms = None;
        negotiation.frame_rendered_time_ms = None;
        Ok(negotiation)
    }

    fn create_offer(&mut self) -> Result<String, XbxEngineRuntimeError> {
        let offer_sdp = self.stack.create_offer()?;
        self.inner.last_offer_sdp = Some(offer_sdp.clone());
        Ok(offer_sdp)
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

    fn add_remote_ice_candidates(
        &mut self,
        remote_candidates: Vec<XbxEngineIceCandidateDto>,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.inner
            .add_remote_ice_candidates(remote_candidates.clone())?;
        self.stack.add_remote_ice_candidates(&remote_candidates)
    }

    fn local_candidates_snapshot(
        &self,
    ) -> Result<Vec<XbxEngineIceCandidateDto>, XbxEngineRuntimeError> {
        Ok(self.stack.local_candidates_snapshot())
    }

    fn local_ice_gathering_complete(&self) -> Result<bool, XbxEngineRuntimeError> {
        Ok(self.stack.local_ice_gathering_complete())
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
        self.stack.set_microphone_capturing(capturing)?;
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
        self.stack.set_keyboard_pointer_enabled(enabled)?;
        self.inner.set_keyboard_pointer_enabled(enabled)
    }

    fn push_keyboard_pointer_input(
        &mut self,
        event: XbxEngineInputEventDto,
    ) -> Result<(), XbxEngineRuntimeError> {
        self.stack.push_keyboard_pointer_input(event.clone())?;
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

    fn request_decoder_reset(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.stack.request_decoder_reset()
    }

    fn stop(&mut self) -> Result<(), XbxEngineRuntimeError> {
        self.stop_peer_connection();
        self.inner.stop()
    }
}
