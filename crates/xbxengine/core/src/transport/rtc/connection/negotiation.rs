use std::collections::HashSet;
#[cfg(test)]
use std::sync::Once;
use std::sync::{Arc, Mutex};

use xbxengine_protocol::XbxEngineIceCandidateDto;

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::connection::builder::{
    build_peer_connection, configure_offer_primitives,
};
use crate::transport::rtc::connection::data_channel::bootstrap_default_channels;
#[cfg(test)]
use crate::transport::rtc::connection::runtime_state::RtcIceCandidateKind;
use crate::transport::rtc::connection::{
    add_remote_candidate_to_peer, build_remote_answer_observation, candidate_identity_key,
    candidate_ip_family, classify_candidate_kind, collect_candidate_ip_families,
    extract_local_candidates_from_offer_sdp, is_end_of_candidates_candidate,
    is_end_of_candidates_marker, should_skip_remote_candidate_for_family_mismatch,
};
use crate::transport::rtc::events::{RtcConnectionLifecycleState, RtcTransportEvent};
use crate::transport::rtc::stats::now_ms_f64;
use crate::{XbxEngineMediaRuntimeStats, XbxEngineRuntimeError};
use xbxengine_protocol::XbxEngineSessionDto;

use super::service::RtcReadIngressCounters;
use super::RtcConnectionService;

#[cfg(test)]
fn ensure_test_rustls_crypto_provider() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

impl RtcConnectionService {
    pub(crate) fn rebuild(
        &mut self,
        session: &XbxEngineSessionDto,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        #[cfg(test)]
        ensure_test_rustls_crypto_provider();
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.message_handshake_acked_at_ms = None;
            stats.control_ready_at_ms = None;
        }
        let preserved_remote_candidates = self.state.lock().ok().map(|state| {
            (
                state.remote_candidates.clone(),
                state.pending_remote_candidates.clone(),
                state.remote_candidate_keys.clone(),
                state.pending_remote_candidate_keys.clone(),
            )
        });
        if let Ok(mut state) = self.state.lock() {
            state.reset_for_session(session.target_type.clone());
            if let Some((
                remote_candidates,
                pending_remote_candidates,
                remote_candidate_keys,
                pending_remote_candidate_keys,
            )) = preserved_remote_candidates
            {
                state.remote_candidates = remote_candidates;
                state.pending_remote_candidates = pending_remote_candidates;
                state.remote_candidate_keys = remote_candidate_keys;
                state.pending_remote_candidate_keys = pending_remote_candidate_keys;
            }
        }
        self.peer_connection = None;
        self.io_runtime.rebuild()?;
        self.control_service.reset();
        self.video_recovery_transport_state = Default::default();
        self.lifecycle_state = RtcConnectionLifecycleState::Connecting;
        self.lifecycle_state_since_ms = now_ms_f64();
        self.last_transport_metrics_sample_at_ms = 0.0;
        self.last_transport_metrics_sample_inbound_video_bytes_total = 0;
        self.lifecycle_observation_id = self.lifecycle_observation_id.saturating_add(1);
        self.remote_rtcp_twcc_observation_id = 0;
        self.controlled_twcc_feedback.reset();
        self.read_counters = RtcReadIngressCounters::default();
        self.pending_media_ingress_packets.clear();
        self.pending_gamepad_rumble_requests.clear();
        self.pending_transport_facts.clear();
        self.delayed_gamepad_added_due_at_ms = None;
        self.delayed_pli_prime_due_at_ms = None;
        self.local_rtcp_sender_ssrc = super::service::generate_local_rtcp_sender_ssrc();
        self.last_selected_pair_diagnostic = None;
        self.selected_pair_snapshot_emitted = false;
        let mut peer_connection =
            build_peer_connection(session, runtime_stats, &self.webrtc_runtime_config)?;
        configure_offer_primitives(&mut peer_connection)?;
        if let Ok(mut state) = self.state.lock() {
            bootstrap_default_channels(&mut peer_connection, &mut state)?;
        }
        for candidate in self
            .io_runtime
            .gather_local_candidates(session, runtime_stats.as_ref())?
        {
            let candidate_dto = XbxEngineIceCandidateDto {
                candidate: candidate.candidate.clone(),
                sdp_m_line_index: candidate.sdp_mline_index,
                sdp_mid: candidate.sdp_mid.clone(),
            };
            let kind = classify_candidate_kind(&candidate_dto.candidate);
            peer_connection
                .add_local_candidate(candidate)
                .map_err(|err| {
                    XbxEngineRuntimeError::new(format!(
                        "xbxEngineRtcAddLocalCandidateFailed: {err}"
                    ))
                })?;
            if let Ok(mut state) = self.state.lock() {
                state.record_local_candidate(candidate_dto, kind);
            } else {
                return Err(XbxEngineRuntimeError::new(
                    "xbxEngineRtcConnectionStateLockFailed",
                ));
            }
        }
        self.peer_connection = Some(peer_connection);
        self.drain_peer_events(runtime_stats)?;
        self.publish_event(
            runtime_stats,
            RtcTransportEvent::ConnectionLifecycleChanged(RtcConnectionLifecycleState::Connecting),
        );
        Ok(())
    }
    pub(crate) fn create_raw_offer(
        &mut self,
        _negotiation: &crate::api::runtime::XbxEngineNegotiationRuntimeConfig,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<String, XbxEngineRuntimeError> {
        let peer_connection = self
            .peer_connection
            .as_mut()
            .ok_or_else(|| XbxEngineRuntimeError::new("xbxEngineRtcPeerConnectionUnavailable"))?;
        let offer = peer_connection.create_offer(None).map_err(|err| {
            XbxEngineRuntimeError::new(format!("xbxEngineRtcCreateOfferFailed: {err}"))
        })?;
        let offer_sdp = offer.sdp.clone();
        peer_connection
            .set_local_description(offer)
            .map_err(|err| {
                XbxEngineRuntimeError::new(format!("xbxEngineRtcSetLocalDescriptionFailed: {err}"))
            })?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionStateLockFailed"))?;
        state.local_offer_sdp = Some(offer_sdp.clone());
        drop(state);
        self.pump(runtime_stats)?;
        self.publish_event(runtime_stats, RtcTransportEvent::LocalOfferCreated);
        Ok(offer_sdp)
    }

    pub(crate) fn apply_remote_description(
        &mut self,
        answer_sdp: &str,
        remote_candidates: &[XbxEngineIceCandidateDto],
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let peer_connection = self
            .peer_connection
            .as_mut()
            .ok_or_else(|| XbxEngineRuntimeError::new("xbxEngineRtcPeerConnectionUnavailable"))?;
        let answer =
            rtc::peer_connection::sdp::RTCSessionDescription::answer(answer_sdp.to_string())
                .map_err(|err| {
                    XbxEngineRuntimeError::new(format!("xbxEngineRtcAnswerParseFailed: {err}"))
                })?;
        peer_connection
            .set_remote_description(answer)
            .map_err(|err| {
                XbxEngineRuntimeError::new(format!("xbxEngineRtcSetRemoteDescriptionFailed: {err}"))
            })?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionStateLockFailed"))?;
        state.remote_answer_sdp = Some(answer_sdp.to_string());
        if is_end_of_candidates_marker(answer_sdp) {
            state.record_remote_end_of_candidates();
        }
        let pending_remote_candidates = state.pending_remote_candidates.clone();
        state.pending_remote_candidates.clear();
        state.pending_remote_candidate_keys.clear();

        let local_ip_families = collect_candidate_ip_families(&state.local_candidates);
        let mut merged_remote_candidates = Vec::new();
        let mut skipped_incompatible_count = 0u64;
        let mut seen_keys = HashSet::new();
        for candidate in pending_remote_candidates
            .iter()
            .chain(remote_candidates.iter())
        {
            if is_end_of_candidates_candidate(candidate) {
                state.record_remote_end_of_candidates();
                continue;
            }
            let kind = classify_candidate_kind(&candidate.candidate);
            let key = candidate_identity_key(candidate);
            if !seen_keys.insert(key.clone()) {
                continue;
            }
            if let Some(is_ipv6) = candidate_ip_family(candidate) {
                let family_mismatch =
                    !local_ip_families.is_empty() && !local_ip_families.contains(&is_ipv6);
                if family_mismatch {
                    if should_skip_remote_candidate_for_family_mismatch(
                        &local_ip_families,
                        kind,
                        is_ipv6,
                    ) {
                        skipped_incompatible_count = skipped_incompatible_count.saturating_add(1);
                        crate::xbx_log_warn!(
                            "[xbxengine][rtc-connection] skip remote candidate due to local family mismatch family={} candidate={}",
                            if is_ipv6 { "ipv6" } else { "ipv4" },
                            candidate.candidate
                        );
                        continue;
                    } else {
                        crate::xbx_log_info!(
                            "[xbxengine][rtc-connection] observed remote candidate family mismatch but not skipping family={} kind={:?} candidate={}",
                            if is_ipv6 { "ipv6" } else { "ipv4" },
                            kind,
                            candidate.candidate
                        );
                    }
                }
            }
            state.record_remote_candidate(candidate.clone(), kind, false, false);
            if !state.applied_remote_candidate_keys.contains(&key) {
                merged_remote_candidates.push((key, candidate.clone(), kind));
            }
        }
        drop(state);
        let remote_answer_observation = build_remote_answer_observation(answer_sdp);
        RuntimeStatsSink::new(runtime_stats.clone())
            .record_remote_answer_observation(remote_answer_observation);
        self.controlled_twcc_feedback
            .apply_remote_answer_bootstrap(answer_sdp, runtime_stats);
        if skipped_incompatible_count > 0 {
            crate::xbx_log_warn!(
                "[xbxengine][rtc-connection] skipped incompatible remote candidates count={}",
                skipped_incompatible_count
            );
        }
        for (key, candidate, _kind) in merged_remote_candidates {
            add_remote_candidate_to_peer(peer_connection, &candidate)?;
            if let Ok(mut state) = self.state.lock() {
                state.applied_remote_candidate_keys.insert(key);
            }
        }
        self.pump(runtime_stats)?;
        self.publish_ice_snapshot(runtime_stats, "rtcRemoteDescriptionApplied");
        self.publish_event(runtime_stats, RtcTransportEvent::RemoteDescriptionApplied);
        Ok(())
    }

    pub(crate) fn add_remote_ice_candidates(
        &mut self,
        remote_candidates: &[XbxEngineIceCandidateDto],
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), XbxEngineRuntimeError> {
        let mut should_cache_only = false;
        if let Some(peer_connection) = self.peer_connection.as_ref() {
            should_cache_only = peer_connection.remote_description().is_none();
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| XbxEngineRuntimeError::new("xbxEngineRtcConnectionStateLockFailed"))?;
        let local_ip_families = collect_candidate_ip_families(&state.local_candidates);
        let mut candidates_to_apply = Vec::new();
        let mut skipped_incompatible_count = 0u64;
        for candidate in remote_candidates {
            if is_end_of_candidates_candidate(candidate) {
                state.record_remote_end_of_candidates();
                continue;
            }
            let kind = classify_candidate_kind(&candidate.candidate);
            let key = candidate_identity_key(candidate);
            if let Some(is_ipv6) = candidate_ip_family(candidate) {
                let family_mismatch =
                    !local_ip_families.is_empty() && !local_ip_families.contains(&is_ipv6);
                if family_mismatch {
                    if should_skip_remote_candidate_for_family_mismatch(
                        &local_ip_families,
                        kind,
                        is_ipv6,
                    ) {
                        skipped_incompatible_count = skipped_incompatible_count.saturating_add(1);
                        crate::xbx_log_warn!(
                            "[xbxengine][rtc-connection] skip remote candidate due to local family mismatch family={} candidate={}",
                            if is_ipv6 { "ipv6" } else { "ipv4" },
                            candidate.candidate
                        );
                        continue;
                    } else {
                        crate::xbx_log_info!(
                            "[xbxengine][rtc-connection] observed remote candidate family mismatch but not skipping family={} kind={:?} candidate={}",
                            if is_ipv6 { "ipv6" } else { "ipv4" },
                            kind,
                            candidate.candidate
                        );
                    }
                }
            }
            state.record_remote_candidate(candidate.clone(), kind, should_cache_only, false);
            if should_cache_only {
                continue;
            }
            if !state.applied_remote_candidate_keys.contains(&key) {
                candidates_to_apply.push((key, candidate.clone(), kind));
            }
        }
        if should_cache_only {
            drop(state);
        } else {
            drop(state);
            let peer_connection = self.peer_connection.as_mut().ok_or_else(|| {
                XbxEngineRuntimeError::new("xbxEngineRtcPeerConnectionUnavailable")
            })?;
            for (key, candidate, _kind) in candidates_to_apply {
                add_remote_candidate_to_peer(peer_connection, &candidate)?;
                if let Ok(mut state) = self.state.lock() {
                    state.applied_remote_candidate_keys.insert(key);
                }
            }
            self.pump(runtime_stats)?;
        }
        if skipped_incompatible_count > 0 {
            crate::xbx_log_warn!(
                "[xbxengine][rtc-connection] skipped incompatible remote candidates count={}",
                skipped_incompatible_count
            );
        }
        self.publish_ice_snapshot(runtime_stats, "rtcRemoteCandidateAdded");
        self.publish_event(runtime_stats, RtcTransportEvent::RemoteCandidateAdded);
        Ok(())
    }

    pub(crate) fn local_candidates_snapshot(&self) -> Vec<XbxEngineIceCandidateDto> {
        let candidates = self
            .state
            .lock()
            .ok()
            .map(|mut state| {
                if state.local_candidates.is_empty() {
                    if let Some(offer_sdp) = state.local_offer_sdp.clone() {
                        // offer SDP 里已经带出的 candidate 不能等到后续事件再补，
                        // 否则 runtime 会错过第一次 ICE 交换窗口。
                        for candidate in extract_local_candidates_from_offer_sdp(&offer_sdp) {
                            let kind = classify_candidate_kind(&candidate.candidate);
                            state.record_local_candidate(candidate, kind);
                        }
                    }
                }
                state.local_candidates.clone()
            })
            .unwrap_or_default();
        crate::xbx_log_debug!(
            "[xbxengine][rtc-connection] local_candidates_snapshot count={}",
            candidates.len()
        );
        candidates
    }

    pub(crate) fn local_ice_gathering_complete(&self) -> bool {
        let complete = self
            .state
            .lock()
            .ok()
            .is_some_and(|state| state.local_ice_gathering_complete);
        crate::xbx_log_warn!(
            "[xbxengine][rtc-connection] local_ice_gathering_complete complete={complete}"
        );
        complete
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::RtcIceCandidateKind;
    use crate::transport::rtc::connection::should_skip_remote_candidate_for_family_mismatch;

    #[test]
    fn host_candidates_are_skipped_when_families_mismatch() {
        assert!(should_skip_remote_candidate_for_family_mismatch(
            &HashSet::from([false]),
            RtcIceCandidateKind::Host,
            true,
        ));
    }

    #[test]
    fn non_host_candidates_are_not_skipped() {
        assert!(!should_skip_remote_candidate_for_family_mismatch(
            &HashSet::from([false]),
            RtcIceCandidateKind::Srflx,
            true,
        ));
        assert!(!should_skip_remote_candidate_for_family_mismatch(
            &HashSet::from([false]),
            RtcIceCandidateKind::Relay,
            true,
        ));
    }
}
