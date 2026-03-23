use std::sync::{Arc, Mutex};

use xbxengine_protocol::XbxEngineIceCandidateDto;

use crate::api::backend::{XbxEngineMediaNegotiationRequest, XbxEngineMediaRuntimeStats};
use crate::transport::rtc::connection::RtcConnectionService;
use crate::transport::rtc::sdp::policy::{summarize_sdp, validate_local_offer_sdp};
use crate::transport::rtc::sdp::{
    adapt_local_offer, adapt_remote_answer, normalize_remote_candidate, RtcSdpContext,
};
use crate::transport::rtc::stream::RtcMediaService;
use crate::{XbxEngineRuntimeConfig, XbxEngineRuntimeError};

// 负责 stack 层的 SDP/ICE 协商桥接，避免 stack.rs 继续堆叠连接细节。
pub(crate) struct RtcStackNegotiationBridge<'a> {
    runtime_config: &'a Arc<Mutex<XbxEngineRuntimeConfig>>,
    last_request: &'a Arc<Mutex<Option<XbxEngineMediaNegotiationRequest>>>,
    runtime_stats: &'a Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    connection: &'a Arc<Mutex<RtcConnectionService>>,
    media: &'a Arc<Mutex<RtcMediaService>>,
}

impl<'a> RtcStackNegotiationBridge<'a> {
    pub(crate) fn new(
        runtime_config: &'a Arc<Mutex<XbxEngineRuntimeConfig>>,
        last_request: &'a Arc<Mutex<Option<XbxEngineMediaNegotiationRequest>>>,
        runtime_stats: &'a Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        connection: &'a Arc<Mutex<RtcConnectionService>>,
        media: &'a Arc<Mutex<RtcMediaService>>,
    ) -> Self {
        Self {
            runtime_config,
            last_request,
            runtime_stats,
            connection,
            media,
        }
    }

    pub(crate) fn create_offer(&self) -> Result<String, XbxEngineRuntimeError> {
        let runtime_config = self
            .runtime_config
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcRuntimeConfigLockFailed"))?
            .clone();
        let raw_offer = self
            .connection
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionLockFailed"))?
            .create_raw_offer(&runtime_config.webrtc.negotiation, self.runtime_stats)?;
        let adapted_offer = adapt_local_offer(&raw_offer, &self.build_sdp_context());
        validate_local_offer_sdp(&adapted_offer)?;
        crate::xbx_log_info!(
            "[xbxengine][rtc-phase1] local offer created {}",
            summarize_sdp(&adapted_offer)
        );
        Ok(adapted_offer)
    }

    pub(crate) fn apply_remote_description(
        &self,
        answer_sdp: &str,
        remote_candidates: &[XbxEngineIceCandidateDto],
    ) -> Result<(), XbxEngineRuntimeError> {
        let normalized_answer = adapt_remote_answer(answer_sdp);
        let normalized_candidates = remote_candidates
            .iter()
            .filter_map(normalize_remote_candidate)
            .collect::<Vec<_>>();
        if let Ok(mut media) = self.media.lock() {
            media.apply_remote_answer_sdp(&normalized_answer);
        }
        self.connection
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionLockFailed"))?
            .apply_remote_description(
                &normalized_answer,
                &normalized_candidates,
                self.runtime_stats,
            )
    }

    pub(crate) fn add_remote_ice_candidates(
        &self,
        remote_candidates: &[XbxEngineIceCandidateDto],
    ) -> Result<(), XbxEngineRuntimeError> {
        let normalized_candidates = remote_candidates
            .iter()
            .filter_map(normalize_remote_candidate)
            .collect::<Vec<_>>();
        self.connection
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionLockFailed"))?
            .add_remote_ice_candidates(&normalized_candidates, self.runtime_stats)
    }

    pub(crate) fn local_candidates_snapshot(&self) -> Vec<XbxEngineIceCandidateDto> {
        let candidates = self
            .connection
            .lock()
            .ok()
            .map(|connection| connection.local_candidates_snapshot())
            .unwrap_or_default();
        crate::xbx_log_info!(
            "[xbxengine][rtc-stack] local_candidates_snapshot count={}",
            candidates.len()
        );
        candidates
    }

    pub(crate) fn local_ice_gathering_complete(&self) -> bool {
        let complete = self
            .connection
            .lock()
            .ok()
            .is_some_and(|connection| connection.local_ice_gathering_complete());
        crate::xbx_log_info!(
            "[xbxengine][rtc-stack] local_ice_gathering_complete complete={complete}"
        );
        complete
    }

    fn build_sdp_context(&self) -> RtcSdpContext {
        let runtime_config = self
            .runtime_config
            .lock()
            .ok()
            .map(|config| config.clone())
            .unwrap_or_default();
        let target_type = self.last_request.lock().ok().and_then(|request| {
            request
                .as_ref()
                .map(|value| value.session.target_type.clone())
        });
        RtcSdpContext {
            negotiation: runtime_config.webrtc.negotiation,
            session_target_type: target_type,
        }
    }
}
