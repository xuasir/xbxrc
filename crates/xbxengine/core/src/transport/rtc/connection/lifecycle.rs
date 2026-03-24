use std::sync::{Arc, Mutex};

use rtc::peer_connection::state::RTCPeerConnectionState;
use rtc::sansio::Protocol;
use xbxengine_protocol::{XbxEngineIceCandidateDto, XbxEngineTransportStateDto};

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::connection::classify_candidate_kind;
use crate::transport::rtc::connection::map_connection_lifecycle_state_fact;
use crate::transport::rtc::events::{RtcConnectionLifecycleState, RtcTransportEvent};
use crate::transport::rtc::facts::{PeerFact, TransportFact};
use crate::transport::rtc::stats::{apply_transport_event, now_ms_f64};
use crate::XbxEngineMediaRuntimeStats;

use super::runtime_state::RtcConnectionRuntimeState;
use super::service::RTC_RECONNECT_GRACE_MS;
use super::RtcConnectionService;

impl RtcConnectionService {
    pub(super) fn handle_peer_connection_state_change(
        &mut self,
        state: RTCPeerConnectionState,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) {
        match state {
            RTCPeerConnectionState::Connected => {
                self.lifecycle_state = RtcConnectionLifecycleState::Connected;
                self.lifecycle_state_since_ms = now_ms_f64();
                self.lifecycle_observation_id = self.lifecycle_observation_id.saturating_add(1);
                self.publish_lifecycle_observation(
                    runtime_stats,
                    RtcConnectionLifecycleState::Connected,
                    "phase1 rtc peer connection connected",
                    None,
                );
            }
            RTCPeerConnectionState::Connecting => {
                self.lifecycle_state = RtcConnectionLifecycleState::Connecting;
                self.lifecycle_state_since_ms = now_ms_f64();
                self.publish_lifecycle_observation(
                    runtime_stats,
                    RtcConnectionLifecycleState::Connecting,
                    "phase1 rtc peer connection connecting",
                    None,
                );
            }
            RTCPeerConnectionState::Disconnected => {
                self.lifecycle_state = RtcConnectionLifecycleState::Disconnected;
                self.lifecycle_state_since_ms = now_ms_f64();
                self.publish_lifecycle_observation(
                    runtime_stats,
                    RtcConnectionLifecycleState::Disconnected,
                    "phase1 rtc peer connection disconnected",
                    Some("transport disconnected".to_string()),
                );
            }
            RTCPeerConnectionState::Failed => {
                self.schedule_immediate_reconnect(
                    runtime_stats,
                    "rtcPeerConnectionFailed",
                    "phase1 rtc peer connection failed",
                    "peer connection failed",
                );
            }
            RTCPeerConnectionState::Closed => {
                self.schedule_immediate_reconnect(
                    runtime_stats,
                    "rtcPeerConnectionClosed",
                    "phase1 rtc peer connection closed",
                    "peer connection closed",
                );
            }
            _ => {
                self.lifecycle_state = RtcConnectionLifecycleState::Connecting;
                self.lifecycle_state_since_ms = now_ms_f64();
                self.publish_lifecycle_observation(
                    runtime_stats,
                    RtcConnectionLifecycleState::Connecting,
                    "phase1 rtc peer connection state changed",
                    Some(format!("rtc peer connection state changed: {state}")),
                );
            }
        }
    }

    pub(super) fn schedule_immediate_reconnect(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        label: &str,
        summary: &str,
        reason: &str,
    ) {
        let observed_at_ms = now_ms_f64();
        self.lifecycle_state = RtcConnectionLifecycleState::Recovering;
        self.lifecycle_state_since_ms = observed_at_ms;
        self.lifecycle_observation_id = self.lifecycle_observation_id.saturating_add(1);
        // 记录是否新建 recovery action，方便上层区分“瞬态 closed”还是“已进入恢复编排”。
        self.publish_lifecycle_observation(
            runtime_stats,
            RtcConnectionLifecycleState::Recovering,
            label,
            Some(format!(
                "{summary} reason={reason} recoveryActionCreated=true observationId={}",
                self.lifecycle_observation_id
            )),
        );
    }

    pub(super) fn maybe_schedule_delayed_reconnect(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) {
        let should_recover = matches!(
            self.lifecycle_state,
            RtcConnectionLifecycleState::Disconnected | RtcConnectionLifecycleState::Failed
        ) && now_ms_f64() - self.lifecycle_state_since_ms
            >= RTC_RECONNECT_GRACE_MS;
        if !should_recover {
            return;
        }
        let reason = match self.lifecycle_state {
            RtcConnectionLifecycleState::Disconnected => "peer connection disconnected",
            RtcConnectionLifecycleState::Failed => "peer connection failed",
            _ => "peer connection recovered",
        };
        self.schedule_immediate_reconnect(
            runtime_stats,
            "rtcConnectionRecovering",
            "phase1 rtc connection entering recovering",
            reason,
        );
    }

    pub(super) fn publish_lifecycle_observation(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        lifecycle_state: RtcConnectionLifecycleState,
        label: &str,
        extra_summary: Option<String>,
    ) {
        let summary = match extra_summary {
            Some(extra) => format!(
                "phase1 rtc lifecycle={:?} state={:?} {extra}",
                lifecycle_state, self.lifecycle_state
            ),
            None => format!(
                "phase1 rtc lifecycle={:?} state={:?}",
                lifecycle_state, self.lifecycle_state
            ),
        };
        apply_transport_event(
            runtime_stats,
            lifecycle_state.transport_state(),
            label,
            &summary,
        );
        self.push_transport_fact(TransportFact::Peer(PeerFact::ConnectionStateChanged {
            state: map_connection_lifecycle_state_fact(lifecycle_state),
            observed_at_ms: now_ms_f64(),
        }));
    }

    pub(super) fn mark_recovering_from_fault(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        label: &str,
        summary: &str,
        lifecycle_state: RtcConnectionLifecycleState,
        reason: String,
    ) {
        let observed_at_ms = now_ms_f64();
        self.lifecycle_state = lifecycle_state;
        self.lifecycle_state_since_ms = observed_at_ms;
        self.lifecycle_observation_id = self.lifecycle_observation_id.saturating_add(1);
        self.publish_lifecycle_observation(
            runtime_stats,
            RtcConnectionLifecycleState::Recovering,
            label,
            Some(format!(
                "{summary} reason={} recoveryActionCreated=true observationId={}",
                reason, self.lifecycle_observation_id
            )),
        );
    }

    pub(super) fn publish_ice_snapshot(
        &self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        label: &str,
    ) {
        if let Ok(state) = self.state.lock() {
            RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
                stats.latest_observation_label = Some(label.to_string());
                stats.latest_observation_summary = Some(format!(
                    "phase1 rtc ice {}",
                    state.candidate_snapshot_summary()
                ));
            });
        }
    }

    pub(super) fn publish_event(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        event: RtcTransportEvent,
    ) {
        let ice_snapshot_summary = self
            .state
            .lock()
            .ok()
            .map(|state| state.candidate_snapshot_summary());
        match event {
            RtcTransportEvent::ConnectionLifecycleChanged(state) => {
                apply_transport_event(
                    runtime_stats,
                    state.transport_state(),
                    state.observation_label(),
                    &format!("phase1 rtc connection lifecycle changed: {:?}", state),
                );
                self.push_transport_fact(TransportFact::Peer(PeerFact::ConnectionStateChanged {
                    state: map_connection_lifecycle_state_fact(state),
                    observed_at_ms: now_ms_f64(),
                }));
            }
            RtcTransportEvent::LocalOfferCreated => apply_transport_event(
                runtime_stats,
                XbxEngineTransportStateDto::Connecting,
                "rtcOfferCreated",
                &format!(
                    "phase1 rtc local offer created{}",
                    ice_snapshot_summary
                        .as_ref()
                        .map(|summary| format!(" ice={summary}"))
                        .unwrap_or_default()
                ),
            ),
            RtcTransportEvent::RemoteDescriptionApplied => apply_transport_event(
                runtime_stats,
                XbxEngineTransportStateDto::Connecting,
                "rtcRemoteDescriptionApplied",
                &format!(
                    "phase1 rtc remote description applied{}",
                    ice_snapshot_summary
                        .as_ref()
                        .map(|summary| format!(" ice={summary}"))
                        .unwrap_or_default()
                ),
            ),
            RtcTransportEvent::RemoteCandidateAdded => apply_transport_event(
                runtime_stats,
                XbxEngineTransportStateDto::Connecting,
                "rtcRemoteCandidateAdded",
                &format!(
                    "phase1 rtc remote candidate appended{}",
                    ice_snapshot_summary
                        .as_ref()
                        .map(|summary| format!(" ice={summary}"))
                        .unwrap_or_default()
                ),
            ),
            RtcTransportEvent::TransportStopped => apply_transport_event(
                runtime_stats,
                XbxEngineTransportStateDto::Closed,
                "rtcTransportStopped",
                "phase1 rtc transport stopped",
            ),
        }
    }

    pub(super) fn drain_peer_events(
        &mut self,
        runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), crate::XbxEngineRuntimeError> {
        if self.peer_connection.is_none() {
            return Ok(());
        }
        let mut pending_data_channel_events = Vec::new();
        let mut pending_ice_events = Vec::new();
        let mut pending_connection_states = Vec::new();
        let mut saw_local_candidate_update = false;
        let mut saw_local_gathering_complete = false;
        loop {
            let mut saw_event = false;
            {
                let Some(peer_connection) = self.peer_connection.as_mut() else {
                    return Ok(());
                };
                while let Some(event) = peer_connection.poll_event() {
                    saw_event = true;
                    match event {
                        rtc::peer_connection::event::RTCPeerConnectionEvent::OnIceCandidateEvent(
                            ice_event,
                        ) => {
                            pending_ice_events.push(ice_event);
                            saw_local_candidate_update = true;
                        }
                        rtc::peer_connection::event::RTCPeerConnectionEvent::OnIceGatheringStateChangeEvent(
                            rtc::peer_connection::state::RTCIceGatheringState::Complete,
                        ) => {
                            // 先单独记录 gathering 完成，再统一刷新快照，避免事件处理顺序互相干扰。
                            crate::xbx_log_warn!(
                                "[xbxengine][rtc-connection] ice gathering state complete observed"
                            );
                            if let Ok(mut state) = self.state.lock() {
                                state.record_local_end_of_candidates();
                            }
                            saw_local_gathering_complete = true;
                        }
                        rtc::peer_connection::event::RTCPeerConnectionEvent::OnConnectionStateChangeEvent(
                            state,
                        ) => {
                            pending_connection_states.push(state);
                        }
                        rtc::peer_connection::event::RTCPeerConnectionEvent::OnDataChannel(
                            dc_event,
                        ) => {
                            pending_data_channel_events.push(dc_event);
                        }
                        _ => {}
                    }
                }
            }
            for ice_event in pending_ice_events.drain(..) {
                self.record_local_candidate_event(ice_event, runtime_stats)?;
            }
            for state in pending_connection_states.drain(..) {
                self.handle_peer_connection_state_change(state, runtime_stats);
            }
            for event in pending_data_channel_events.drain(..) {
                self.apply_data_channel_event(event, runtime_stats)?;
            }
            if !saw_event {
                break;
            }
        }
        if saw_local_gathering_complete {
            self.publish_ice_snapshot(runtime_stats, "rtcLocalIceGatheringComplete");
        } else if saw_local_candidate_update {
            self.publish_ice_snapshot(runtime_stats, "rtcLocalIceCandidateObserved");
        }
        Ok(())
    }

    fn record_local_candidate_event(
        &self,
        ice_event: rtc::peer_connection::event::RTCPeerConnectionIceEvent,
        _runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>,
    ) -> Result<(), crate::XbxEngineRuntimeError> {
        let mut candidate = ice_event.candidate.to_json().map_err(|err| {
            crate::XbxEngineRuntimeError::new(format!("xbxEngineRtcCandidateToJsonFailed: {err}"))
        })?;
        if candidate
            .sdp_mid
            .as_deref()
            .is_none_or(|mid| mid.trim().is_empty())
        {
            candidate.sdp_mid = Some("0".to_string());
        }
        if candidate.sdp_mline_index.is_none() {
            candidate.sdp_mline_index = Some(0);
        }
        let dto = XbxEngineIceCandidateDto {
            candidate: candidate.candidate,
            sdp_m_line_index: candidate.sdp_mline_index,
            sdp_mid: candidate.sdp_mid,
        };
        let kind = classify_candidate_kind(&dto.candidate);
        if let Ok(mut state) = self.state.lock() {
            state.record_local_candidate(dto, kind);
        } else {
            return Err(crate::XbxEngineRuntimeError::new(
                "xbxEngineRtcConnectionStateLockFailed",
            ));
        }
        Ok(())
    }

    pub(crate) fn stop(&mut self, runtime_stats: &Arc<Mutex<XbxEngineMediaRuntimeStats>>) {
        if let Some(peer_connection) = self.peer_connection.as_mut() {
            if let Err(err) = peer_connection.close() {
                RuntimeStatsSink::new(runtime_stats.clone()).update(|stats| {
                    stats.latest_observation_label =
                        Some("rtcPeerConnectionCloseFailed".to_string());
                    stats.latest_observation_summary =
                        Some(format!("phase1 rtc peer connection close failed: {err}"));
                });
            }
        }
        self.peer_connection = None;
        self.io_runtime.stop();
        self.control_service.close_control_channel();
        self.control_service.close_message_channel();
        self.control_service.clear_pending_replay_actions();
        if let Ok(mut stats) = runtime_stats.lock() {
            stats.message_handshake_acked_at_ms = None;
            stats.control_ready_at_ms = None;
        }
        self.lifecycle_state = RtcConnectionLifecycleState::Closed;
        self.lifecycle_state_since_ms = now_ms_f64();
        self.lifecycle_observation_id = self.lifecycle_observation_id.saturating_add(1);
        if let Ok(mut state) = self.state.lock() {
            *state = RtcConnectionRuntimeState::default();
        }
        self.delayed_gamepad_added_due_at_ms = None;
        self.delayed_keyframe_prime_due_at_ms = None;
        self.pending_media_ingress_packets.clear();
        self.publish_event(runtime_stats, RtcTransportEvent::TransportStopped);
        self.last_transport_metrics_sample_at_ms = 0.0;
        self.last_transport_metrics_sample_inbound_video_bytes_total = 0;
    }

    pub(super) fn push_transport_fact(&mut self, fact: TransportFact) {
        self.pending_transport_facts.push(fact);
    }
}
