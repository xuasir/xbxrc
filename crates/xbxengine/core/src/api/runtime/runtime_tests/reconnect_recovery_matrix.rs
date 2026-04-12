use super::fixtures::*;
use super::super::{
    XbxEngineRuntime,
    XbxEngineRuntimeConfig, XbxEngineRuntimeState,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use xbxengine_protocol::{
    XbxEngineHostRequestDto,
    XbxEngineTransportStateDto,
};

use crate::transport::rtc::facts::{
    SessionCommand, TransportCommand,
};
use crate::transport::rtc::session::actor::SessionPolicyHook;
use crate::transport::rtc::session::policy::RtcSessionPolicy;
use crate::transport::rtc::stream::video_source::test_fixtures::{
    run_local_ingress_replay_profile, LocalIngressReplayFixture,
};
use crate::{
    XbxEngineInputStatus, XbxEngineMediaNegotiation,
    XbxEngineMediaRuntimeStats,
};

#[test]
fn runtime_consumes_pending_transport_reconnect_candidate_once() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            inbound_video_packet_count_total: 500,
            latest_observation_label: Some("rtcConnectionRecovering".to_string()),
            latest_observation_summary: Some(
                "phase1 rtc lifecycle=Recovering state=Recovering recoverySignalRaised=true"
                    .to_string(),
            ),
            latest_video_escalation_observation: Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 42,
                reason: "transportExpiredDeadline".to_string(),
                action: "requestReconnectCandidate".to_string(),
                recovery_stage: "reconnecting".to_string(),
                recovery_chain_value: "health".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "reconnect-window".to_string(),
                observed_at_ms: now_ms,
            }),
            ..Default::default()
        },
    );
    *backend
        .pending_runtime_recovery_action
        .lock()
        .expect("lock pending runtime recovery action") = Some(
        crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: 42,
            reason: "transportExpiredDeadline".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
        },
    );
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();

    runtime.tick();
    let reconnect_request_count_after_first_tick = requests
        .borrow()
        .iter()
        .filter(|request| {
            matches!(
                request,
                XbxEngineHostRequestDto::ExchangeOffer { channel, restart, .. }
                if channel == "media" && *restart
            )
        })
        .count();
    assert_eq!(reconnect_request_count_after_first_tick, 1);
    assert_eq!(
        runtime.snapshot().last_recovery_reason.as_deref(),
        Some("transportReconnectCandidate:transportExpiredDeadline")
    );

    runtime.tick();
    let reconnect_request_count_after_second_tick = requests
        .borrow()
        .iter()
        .filter(|request| {
            matches!(
                request,
                XbxEngineHostRequestDto::ExchangeOffer { channel, restart, .. }
                if channel == "media" && *restart
            )
        })
        .count();
    assert_eq!(reconnect_request_count_after_second_tick, 1);
}

#[test]
fn runtime_consumes_pending_transport_reconnect_candidate_even_when_transport_is_connected() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            inbound_video_packet_count_total: 500,
            latest_video_escalation_observation: Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 77,
                reason: "transportAwaitRecoveryKeyframe".to_string(),
                action: "requestReconnectCandidate".to_string(),
                recovery_stage: "reconnecting".to_string(),
                recovery_chain_value: "anchor".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "reconnect-window".to_string(),
                observed_at_ms: now_ms,
            }),
            ..Default::default()
        },
    );
    *backend
        .pending_runtime_recovery_action
        .lock()
        .expect("lock pending runtime recovery action") = Some(
        crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: 77,
            reason: "transportAwaitRecoveryKeyframe".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
        },
    );
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();

    runtime.tick();
    let reconnect_request_count = requests
        .borrow()
        .iter()
        .filter(|request| {
            matches!(
                request,
                XbxEngineHostRequestDto::ExchangeOffer { channel, restart, .. }
                if channel == "media" && *restart
            )
        })
        .count();
    assert_eq!(reconnect_request_count, 1);
    assert_eq!(
        runtime.snapshot().last_recovery_reason.as_deref(),
        Some("transportReconnectCandidate:transportAwaitRecoveryKeyframe")
    );
}

#[test]
fn runtime_rejects_pending_reconnect_candidate_with_local_domain_reason() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            inbound_video_packet_count_total: 500,
            latest_video_escalation_observation: Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 91,
                reason: "localBackpressureDeltaGap".to_string(),
                action: "requestReconnectCandidate".to_string(),
                recovery_stage: "reconnecting".to_string(),
                recovery_chain_value: "health".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "reconnect-window".to_string(),
                observed_at_ms: now_ms,
            }),
            ..Default::default()
        },
    );
    *backend
        .pending_runtime_recovery_action
        .lock()
        .expect("lock pending runtime recovery action") = Some(
        crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: 91,
            reason: "localBackpressureDeltaGap".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::Local,
        },
    );
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();

    runtime.tick();

    let reconnect_request_count = requests
        .borrow()
        .iter()
        .filter(|request| {
            matches!(
                request,
                XbxEngineHostRequestDto::ExchangeOffer { channel, restart, .. }
                if channel == "media" && *restart
            )
        })
        .count();
    assert_eq!(reconnect_request_count, 0);
    assert_eq!(
        runtime.snapshot().last_recovery_action.as_deref(),
        Some("reconnectCandidateRejectedByDomainGate")
    );
    assert_eq!(
        runtime.snapshot().last_recovery_reason.as_deref(),
        Some(
            "transportReconnectCandidateRejected:domain=local observationId=91 reason=localBackpressureDeltaGap"
        )
    );
}

#[test]
fn runtime_allows_pending_reconnect_candidate_with_peer_connectivity_reason() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Disconnected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            inbound_video_packet_count_total: 500,
            latest_observation_label: Some("rtcConnectionRecovering".to_string()),
            latest_observation_summary: Some("peer state transitioned to closed".to_string()),
            ..Default::default()
        },
    );
    *backend
        .pending_runtime_recovery_action
        .lock()
        .expect("lock pending runtime recovery action") = Some(
        crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: 92,
            reason: "peer-closed".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
        },
    );
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();

    runtime.tick();

    let reconnect_request_count = requests
        .borrow()
        .iter()
        .filter(|request| {
            matches!(
                request,
                XbxEngineHostRequestDto::ExchangeOffer { channel, restart, .. }
                if channel == "media" && *restart
            )
        })
        .count();
    assert_eq!(reconnect_request_count, 1);
    assert_eq!(
        runtime.snapshot().last_recovery_reason.as_deref(),
        Some("transportReconnectCandidate:peer-closed")
    );
}

#[test]
fn reconnect_candidate_domain_gate_uses_strong_type_contract() {
    assert!(crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport
        .allows_runtime_reconnect_candidate());
    assert!(!crate::XbxEngineRecoveryReasonDomain::Local.allows_runtime_reconnect_candidate());
    assert!(!crate::XbxEngineRecoveryReasonDomain::Unknown.allows_runtime_reconnect_candidate());
}

#[test]
fn runtime_rejects_pending_reconnect_candidate_with_display_supply_critical_local_domain() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            inbound_video_packet_count_total: 500,
            latest_video_escalation_observation: Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 93,
                reason: "displaySupplyCritical".to_string(),
                action: "requestReconnectCandidate".to_string(),
                recovery_stage: "reconnecting".to_string(),
                recovery_chain_value: "health".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "reconnect-window".to_string(),
                observed_at_ms: now_ms,
            }),
            ..Default::default()
        },
    );
    *backend
        .pending_runtime_recovery_action
        .lock()
        .expect("lock pending runtime recovery action") = Some(
        crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: 93,
            reason: "displaySupplyCritical".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::Local,
        },
    );
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();

    runtime.tick();

    let reconnect_request_count = requests
        .borrow()
        .iter()
        .filter(|request| {
            matches!(
                request,
                XbxEngineHostRequestDto::ExchangeOffer { channel, restart, .. }
                if channel == "media" && *restart
            )
        })
        .count();
    assert_eq!(reconnect_request_count, 0);
    assert_eq!(
        runtime.snapshot().last_recovery_reason.as_deref(),
        Some(
            "transportReconnectCandidateRejected:domain=local observationId=93 reason=displaySupplyCritical"
        )
    );
}

#[test]
fn runtime_allows_pending_reconnect_candidate_with_liveness_timeout_connectivity_domain() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Disconnected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            inbound_video_packet_count_total: 500,
            latest_observation_label: Some("rtcConnectionRecovering".to_string()),
            latest_observation_summary: Some("liveness timeout escalated to reconnect".to_string()),
            ..Default::default()
        },
    );
    *backend
        .pending_runtime_recovery_action
        .lock()
        .expect("lock pending runtime recovery action") = Some(
        crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: 94,
            reason: "livenessNoProgressTimeout".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
        },
    );
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();

    runtime.tick();

    let reconnect_request_count = requests
        .borrow()
        .iter()
        .filter(|request| {
            matches!(
                request,
                XbxEngineHostRequestDto::ExchangeOffer { channel, restart, .. }
                if channel == "media" && *restart
            )
        })
        .count();
    assert_eq!(reconnect_request_count, 1);
    assert_eq!(
        runtime.snapshot().last_recovery_reason.as_deref(),
        Some("transportReconnectCandidate:livenessNoProgressTimeout")
    );
}

#[test]
fn runtime_defers_pending_transport_reconnect_candidate_while_reconnecting() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connecting,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            inbound_video_packet_count_total: 500,
            ..Default::default()
        },
    );
    *backend
        .pending_runtime_recovery_action
        .lock()
        .expect("lock pending runtime recovery action") = Some(
        crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: 77,
            reason: "transportAwaitRecoveryKeyframe".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
        },
    );
    let pending = backend.pending_runtime_recovery_action.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();
    runtime.state = XbxEngineRuntimeState::Reconnecting;

    runtime.tick();

    let reconnect_request_count = requests
        .borrow()
        .iter()
        .filter(|request| {
            matches!(
                request,
                XbxEngineHostRequestDto::ExchangeOffer { channel, restart, .. }
                if channel == "media" && *restart
            )
        })
        .count();
    assert_eq!(reconnect_request_count, 0);
    assert!(pending
        .lock()
        .expect("lock pending runtime recovery action")
        .is_some());
    assert_eq!(
        runtime.snapshot().last_recovery_reason.as_deref(),
        Some("transportReconnectCandidateDeferred:reconnecting")
    );
}

#[test]
fn runtime_pending_reconnect_candidate_matrix_separates_local_ingress_from_transport_connectivity()
{
    let cases = [
        PendingReconnectCandidateMatrixCase {
            observation_id: 191,
            reason: "localBackpressureDeltaGap",
            reason_domain: crate::XbxEngineRecoveryReasonDomain::Local,
            transport_state: XbxEngineTransportStateDto::Connected,
            observation_kind: PendingReconnectCandidateObservationKind::VideoEscalation {
                recovery_chain_value: "health",
            },
            expected_reconnect_request_count: 0,
            expected_last_action: Some("reconnectCandidateRejectedByDomainGate"),
            expected_last_reason:
                "transportReconnectCandidateRejected:domain=local observationId=191 reason=localBackpressureDeltaGap",
        },
        PendingReconnectCandidateMatrixCase {
            observation_id: 192,
            reason: "peer-closed",
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
            transport_state: XbxEngineTransportStateDto::Disconnected,
            observation_kind: PendingReconnectCandidateObservationKind::RuntimeObservation {
                label: "rtcConnectionRecovering",
                summary: "peer state transitioned to closed",
            },
            expected_reconnect_request_count: 1,
            expected_last_action: Some("reconnect"),
            expected_last_reason: "transportReconnectCandidate:peer-closed",
        },
    ];

    for case in &cases {
        let outcome = drive_pending_reconnect_candidate_matrix_case(case);
        assert_eq!(
            outcome.reconnect_request_count, case.expected_reconnect_request_count,
            "unexpected reconnect request count for {}",
            case.reason
        );
        assert_eq!(
            outcome.last_recovery_action.as_deref(),
            case.expected_last_action,
            "unexpected recovery action for {}",
            case.reason
        );
        assert_eq!(
            outcome.last_recovery_reason.as_deref(),
            Some(case.expected_last_reason),
            "unexpected recovery reason for {}",
            case.reason
        );
    }
}

#[test]
fn runtime_pending_reconnect_candidate_matrix_keeps_display_local_but_allows_liveness_transport() {
    let cases = [
        PendingReconnectCandidateMatrixCase {
            observation_id: 193,
            reason: "displaySupplyCritical",
            reason_domain: crate::XbxEngineRecoveryReasonDomain::Local,
            transport_state: XbxEngineTransportStateDto::Connected,
            observation_kind: PendingReconnectCandidateObservationKind::VideoEscalation {
                recovery_chain_value: "health",
            },
            expected_reconnect_request_count: 0,
            expected_last_action: Some("reconnectCandidateRejectedByDomainGate"),
            expected_last_reason:
                "transportReconnectCandidateRejected:domain=local observationId=193 reason=displaySupplyCritical",
        },
        PendingReconnectCandidateMatrixCase {
            observation_id: 194,
            reason: "livenessNoProgressTimeout",
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
            transport_state: XbxEngineTransportStateDto::Disconnected,
            observation_kind: PendingReconnectCandidateObservationKind::RuntimeObservation {
                label: "rtcConnectionRecovering",
                summary: "liveness timeout escalated to reconnect",
            },
            expected_reconnect_request_count: 1,
            expected_last_action: Some("reconnect"),
            expected_last_reason: "transportReconnectCandidate:livenessNoProgressTimeout",
        },
    ];

    for case in &cases {
        let outcome = drive_pending_reconnect_candidate_matrix_case(case);
        assert_eq!(
            outcome.reconnect_request_count, case.expected_reconnect_request_count,
            "unexpected reconnect request count for {}",
            case.reason
        );
        assert_eq!(
            outcome.last_recovery_action.as_deref(),
            case.expected_last_action,
            "unexpected recovery action for {}",
            case.reason
        );
        assert_eq!(
            outcome.last_recovery_reason.as_deref(),
            Some(case.expected_last_reason),
            "unexpected recovery reason for {}",
            case.reason
        );
    }
}

#[test]
fn runtime_pending_reconnect_candidate_matrix_keeps_transport_await_local_but_allows_deadline_transport(
) {
    let cases = [
        PendingReconnectCandidateMatrixCase {
            observation_id: 195,
            reason: "transportAwaitRecoveryKeyframe",
            reason_domain: crate::XbxEngineRecoveryReasonDomain::Local,
            transport_state: XbxEngineTransportStateDto::Connected,
            observation_kind: PendingReconnectCandidateObservationKind::VideoEscalation {
                recovery_chain_value: "anchor",
            },
            expected_reconnect_request_count: 0,
            expected_last_action: Some("reconnectCandidateRejectedByDomainGate"),
            expected_last_reason:
                "transportReconnectCandidateRejected:domain=local observationId=195 reason=transportAwaitRecoveryKeyframe",
        },
        PendingReconnectCandidateMatrixCase {
            observation_id: 196,
            reason: "transportExpiredDeadline",
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
            transport_state: XbxEngineTransportStateDto::Disconnected,
            observation_kind: PendingReconnectCandidateObservationKind::RuntimeObservation {
                label: "rtcConnectionRecovering",
                summary: "transport deadline escalated to reconnect",
            },
            expected_reconnect_request_count: 1,
            expected_last_action: Some("reconnect"),
            expected_last_reason: "transportReconnectCandidate:transportExpiredDeadline",
        },
    ];

    for case in &cases {
        let outcome = drive_pending_reconnect_candidate_matrix_case(case);
        assert_eq!(
            outcome.reconnect_request_count, case.expected_reconnect_request_count,
            "unexpected reconnect request count for {}",
            case.reason
        );
        assert_eq!(
            outcome.last_recovery_action.as_deref(),
            case.expected_last_action,
            "unexpected recovery action for {}",
            case.reason
        );
        assert_eq!(
            outcome.last_recovery_reason.as_deref(),
            Some(case.expected_last_reason),
            "unexpected recovery reason for {}",
            case.reason
        );
    }
}

#[test]
fn runtime_rejects_replayed_local_pending_reconnect_candidates_without_request_storm() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            inbound_video_packet_count_total: 500,
            latest_video_escalation_observation: Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 201,
                reason: "transportAwaitRecoveryKeyframe".to_string(),
                action: "requestReconnectCandidate".to_string(),
                recovery_stage: "rebuilding-supply".to_string(),
                recovery_chain_value: "anchor".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "transport-await-window".to_string(),
                observed_at_ms: now_ms,
            }),
            ..Default::default()
        },
    );
    *backend
        .pending_runtime_recovery_action
        .lock()
        .expect("lock pending runtime recovery action") = Some(
        crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: 201,
            reason: "transportAwaitRecoveryKeyframe".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::Local,
        },
    );
    let pending = backend.pending_runtime_recovery_action.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();

    runtime.tick();
    *pending
        .lock()
        .expect("lock pending runtime recovery action") = Some(
        crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: 202,
            reason: "transportAwaitRecoveryKeyframe".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::Local,
        },
    );
    runtime.tick();

    let reconnect_request_count = requests
        .borrow()
        .iter()
        .filter(|request| {
            matches!(
                request,
                XbxEngineHostRequestDto::ExchangeOffer { channel, restart, .. }
                if channel == "media" && *restart
            )
        })
        .count();
    assert_eq!(reconnect_request_count, 0);
    assert_eq!(
        runtime.snapshot().last_recovery_action.as_deref(),
        Some("reconnectCandidateRejectedByDomainGate")
    );
    assert_eq!(
        runtime.snapshot().last_recovery_reason.as_deref(),
        Some(
            "transportReconnectCandidateRejected:domain=local observationId=202 reason=transportAwaitRecoveryKeyframe"
        )
    );
    assert!(pending
        .lock()
        .expect("lock pending runtime recovery action")
        .is_none());
}

#[test]
fn runtime_accepts_transport_candidate_after_local_candidate_was_rejected() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            inbound_video_packet_count_total: 500,
            latest_video_escalation_observation: Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 211,
                reason: "transportAwaitRecoveryKeyframe".to_string(),
                action: "requestReconnectCandidate".to_string(),
                recovery_stage: "rebuilding-supply".to_string(),
                recovery_chain_value: "anchor".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "transport-await-window".to_string(),
                observed_at_ms: now_ms,
            }),
            ..Default::default()
        },
    );
    *backend
        .pending_runtime_recovery_action
        .lock()
        .expect("lock pending runtime recovery action") = Some(
        crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: 211,
            reason: "transportAwaitRecoveryKeyframe".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::Local,
        },
    );
    let pending = backend.pending_runtime_recovery_action.clone();
    let runtime_stats = backend.runtime_stats.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();

    runtime.tick();
    assert_eq!(
        runtime.snapshot().last_recovery_action.as_deref(),
        Some("reconnectCandidateRejectedByDomainGate")
    );

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_state = XbxEngineTransportStateDto::Disconnected;
        stats.latest_observation_label = Some("rtcConnectionRecovering".to_string());
        stats.latest_observation_summary =
            Some("transport deadline escalated to reconnect".to_string());
    }
    *pending
        .lock()
        .expect("lock pending runtime recovery action") = Some(
        crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: 212,
            reason: "transportExpiredDeadline".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
        },
    );
    runtime.tick();

    let reconnect_request_count = requests
        .borrow()
        .iter()
        .filter(|request| {
            matches!(
                request,
                XbxEngineHostRequestDto::ExchangeOffer { channel, restart, .. }
                if channel == "media" && *restart
            )
        })
        .count();
    assert_eq!(reconnect_request_count, 1);
    assert_eq!(
        runtime.snapshot().last_recovery_action.as_deref(),
        Some("reconnect")
    );
    assert_eq!(
        runtime.snapshot().last_recovery_reason.as_deref(),
        Some("transportReconnectCandidate:transportExpiredDeadline")
    );
}

#[test]
fn runtime_accepts_transport_severe_candidate_after_local_display_candidate_was_rejected() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            inbound_video_packet_count_total: 500,
            latest_video_escalation_observation: Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 221,
                reason: "displaySupplyCritical".to_string(),
                action: "requestReconnectCandidate".to_string(),
                recovery_stage: "reconnecting".to_string(),
                recovery_chain_value: "health".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "reconnect-window".to_string(),
                observed_at_ms: now_ms,
            }),
            ..Default::default()
        },
    );
    *backend
        .pending_runtime_recovery_action
        .lock()
        .expect("lock pending runtime recovery action") = Some(
        crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: 221,
            reason: "displaySupplyCritical".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::Local,
        },
    );
    let pending = backend.pending_runtime_recovery_action.clone();
    let runtime_stats = backend.runtime_stats.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();

    runtime.tick();
    assert_eq!(
        runtime.snapshot().last_recovery_action.as_deref(),
        Some("reconnectCandidateRejectedByDomainGate")
    );

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_state = XbxEngineTransportStateDto::Disconnected;
        stats.latest_observation_label = Some("rtcConnectionRecovering".to_string());
        stats.latest_observation_summary =
            Some("transport severe deadline escalated to reconnect".to_string());
    }
    *pending
        .lock()
        .expect("lock pending runtime recovery action") = Some(
        crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: 222,
            reason: "transportSevereDeadline".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
        },
    );
    runtime.tick();

    let reconnect_request_count = requests
        .borrow()
        .iter()
        .filter(|request| {
            matches!(
                request,
                XbxEngineHostRequestDto::ExchangeOffer { channel, restart, .. }
                if channel == "media" && *restart
            )
        })
        .count();
    assert_eq!(reconnect_request_count, 1);
    assert_eq!(
        runtime.snapshot().last_recovery_action.as_deref(),
        Some("reconnect")
    );
    assert_eq!(
        runtime.snapshot().last_recovery_reason.as_deref(),
        Some("transportReconnectCandidate:transportSevereDeadline")
    );
}

#[test]
fn runtime_accepts_recovering_candidate_after_local_transport_await_candidate_was_rejected() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 20.0),
            inbound_video_packet_count_total: 500,
            latest_video_escalation_observation: Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 231,
                reason: "transportAwaitRecoveryKeyframe".to_string(),
                action: "requestReconnectCandidate".to_string(),
                recovery_stage: "rebuilding-supply".to_string(),
                recovery_chain_value: "anchor".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "transport-await-window".to_string(),
                observed_at_ms: now_ms,
            }),
            ..Default::default()
        },
    );
    *backend
        .pending_runtime_recovery_action
        .lock()
        .expect("lock pending runtime recovery action") = Some(
        crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: 231,
            reason: "transportAwaitRecoveryKeyframe".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::Local,
        },
    );
    let pending = backend.pending_runtime_recovery_action.clone();
    let runtime_stats = backend.runtime_stats.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();

    runtime.tick();
    assert_eq!(
        runtime.snapshot().last_recovery_action.as_deref(),
        Some("reconnectCandidateRejectedByDomainGate")
    );

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_state = XbxEngineTransportStateDto::Disconnected;
        stats.latest_observation_label = Some("rtcConnectionRecovering".to_string());
        stats.latest_observation_summary = Some(
            "phase1 rtc lifecycle=Recovering state=Recovering recoverySignalRaised=true"
                .to_string(),
        );
    }
    *pending
        .lock()
        .expect("lock pending runtime recovery action") = Some(
        crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: 232,
            reason: "rtcConnectionRecovering".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
        },
    );
    runtime.tick();

    let reconnect_request_count = requests
        .borrow()
        .iter()
        .filter(|request| {
            matches!(
                request,
                XbxEngineHostRequestDto::ExchangeOffer { channel, restart, .. }
                if channel == "media" && *restart
            )
        })
        .count();
    assert_eq!(reconnect_request_count, 1);
    assert_eq!(
        runtime.snapshot().last_recovery_action.as_deref(),
        Some("reconnect")
    );
    assert_eq!(
        runtime.snapshot().last_recovery_reason.as_deref(),
        Some("transportReconnectCandidate:rtcConnectionRecovering")
    );
}

#[tokio::test]
async fn runtime_cloud_recovery_replay_accepts_transport_reconnect_after_local_noise_rejection_and_exits_cleanly(
) {
    let repair_limit = LocalIngressReplayFixture::new(1).repair_backlog_limit();
    let profile = repair_overflow_runtime_replay_profile(repair_limit);
    let fixture = run_local_ingress_replay_profile(&profile).await;
    let runtime_stats = fixture.runtime_stats();
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
    let bridge = build_transport_session_bridge(
        runtime_stats.clone(),
        runtime_config.clone(),
        pending_runtime_recovery_action.clone(),
    );

    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats::default(),
    );
    backend.runtime_stats = runtime_stats.clone();
    backend.pending_runtime_recovery_action = pending_runtime_recovery_action.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()),
        TestEventSink::new(events),
        backend,
    );
    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();

    bridge.apply_transport_session_command(SessionCommand::Transport(
        TransportCommand::RequestReconnectCandidate {
            observation_id: 501,
            reason: "transportAwaitRecoveryKeyframe".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::Local,
        },
    ));
    runtime.tick();
    assert_eq!(
        runtime.snapshot().last_recovery_action.as_deref(),
        Some("reconnectCandidateRejectedByDomainGate")
    );
    let initial_restart_count = count_media_restart_requests(&requests);
    assert_eq!(initial_restart_count, 0);

    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let local_recover = transport_commands(policy.on_snapshot(&fixture.build_connected_snapshot(
        1,
        profile.baseline.now_ms,
        240,
        "transportAwaitRecoveryKeyframe",
    )));
    assert!(local_recover.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestKeyframe { reason, .. }
                if reason == "transportAwaitRecoveryKeyframe"
        )
    }));
    assert!(local_recover
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    {
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_owner_state.as_deref(), Some("priming"));
        assert_eq!(stats.video_owner_reason.as_deref(), Some("priming"));
        assert_eq!(stats.video_owner_source.as_deref(), Some("steady"));
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("local recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportAwaitRecoveryKeyframe:transportAwaitRecoveryKeyframe"
        );
        assert_eq!(ledger.gate_result, "pass:localProbe");
        assert_eq!(ledger.action_selected, "requestKeyframe");
        assert_ne!(ledger.state_after, "reconnecting");
    }

    fixture.mark_transport_connectivity_degraded(profile.baseline.now_ms + 30.0);
    let reconnect_commands = transport_commands(policy.on_snapshot(&build_recovering_snapshot(
        &fixture,
        2,
        profile.baseline.now_ms + 420.0,
        240,
        "rtcConnectionRecovering",
    )));
    let reconnect_candidate = reconnect_commands
        .iter()
        .find(|command| {
            matches!(
                command,
                TransportCommand::RequestReconnectCandidate { reason, .. }
                    if reason == "rtcConnectionRecovering"
            )
        })
        .cloned()
        .expect("recovering transport reconnect candidate");
    {
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(
            stats.video_owner_state.as_deref(),
            Some("rebuilding-supply")
        );
        assert_eq!(
            stats.video_owner_reason.as_deref(),
            Some("transportAwaitRecoveryKeyframe")
        );
        assert_eq!(stats.video_owner_source.as_deref(), Some("anchor"));
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("reconnect decision ledger");
        assert_eq!(
            ledger.input_signal,
            "rtcConnectionRecovering:rtcConnectionRecovering"
        );
        assert_eq!(
            ledger.gate_result,
            "pass:reconnectGranted:connectivityEvidence"
        );
        assert_eq!(ledger.action_selected, "requestReconnectCandidate");
        assert_eq!(ledger.state_after, "reconnecting");
        assert!(ledger.budget_before.is_some());
        assert!(ledger.budget_after.is_some());
    }
    bridge.apply_transport_session_command(SessionCommand::Transport(reconnect_candidate));
    {
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let escalation = stats
            .latest_video_escalation_observation
            .as_ref()
            .expect("staged reconnect escalation");
        assert_eq!(escalation.reason, "rtcConnectionRecovering");
        assert_eq!(escalation.action, "requestReconnectCandidate");
        assert_eq!(escalation.recovery_stage, "reconnecting");
    }
    assert!(matches!(
        pending_runtime_recovery_action
            .lock()
            .expect("lock pending runtime recovery action")
            .as_ref(),
        Some(crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            reason,
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
            ..
        }) if reason == "rtcConnectionRecovering"
    ));

    runtime.tick();
    let reconnect_request_count = count_media_restart_requests(&requests);
    assert_eq!(reconnect_request_count, 1);
    assert_eq!(
        runtime.snapshot().last_recovery_action.as_deref(),
        Some("reconnect")
    );
    assert_eq!(
        runtime.snapshot().last_recovery_reason.as_deref(),
        Some("transportReconnectCandidate:rtcConnectionRecovering")
    );

    fixture.mark_transport_recovered(profile.baseline.now_ms + 900.0);
    let recovered_commands = transport_commands(policy.on_snapshot(
        &fixture.build_connected_snapshot(3, profile.baseline.now_ms + 930.0, 260, "none"),
    ));
    assert!(
        recovered_commands.is_empty(),
        "unexpected commands after recovery exit: {recovered_commands:?}"
    );
    runtime.tick();
    let reconnect_request_count_after_recovered = count_media_restart_requests(&requests);
    assert_eq!(reconnect_request_count_after_recovered, 1);
    assert!(pending_runtime_recovery_action
        .lock()
        .expect("lock pending runtime recovery action")
        .is_none());
}

#[tokio::test]
async fn runtime_home_clean_anchor_short_jitter_replay_never_reaches_reconnect() {
    let repair_limit = LocalIngressReplayFixture::new(1).repair_backlog_limit();
    let profile = repair_overflow_runtime_replay_profile(repair_limit);
    let fixture = run_local_ingress_replay_profile(&profile).await;
    fixture.seed_healthy_policy_baseline_for_target(
        xbxengine_protocol::XbxEngineTargetTypeDto::Home,
        profile.baseline.now_ms,
        profile.baseline.frame_rtp_timestamp,
    );
    let runtime_stats = fixture.runtime_stats();
    {
        let mut stats = runtime_stats.lock().expect("runtime stats lock");
        stats.transport_recovery_epoch = 12;
        stats.video_anchor_clean_epoch = Some(12);
        stats.video_anchor_clean_observed_at_ms = Some(profile.baseline.now_ms - 3.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-keyframe-submitted".to_string());
        stats.host_no_pending_pressure_level = Some("high".to_string());
        stats.host_no_pending_streak = 3;
        stats.latest_video_host_present_time_ms = Some(profile.baseline.now_ms - 8.0);
        stats.latest_video_decode_ok_time_ms = Some(profile.baseline.now_ms - 5.0);
        stats.latest_video_packet_arrival_time_ms = Some(profile.baseline.now_ms - 4.0);
        if let Some(track) = stats.latest_video_track_status.as_mut() {
            track.video_bytes_total = 240_000;
            track.video_packet_count_total = 1_920;
            track.audio_bytes_total = 32_000;
            track.observed_at_ms = profile.baseline.now_ms - 3.0;
        }
        if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
            timeline.source_event = "frame-observed".to_string();
            timeline.chain.state = "healthy".to_string();
            timeline.chain.reason = None;
            timeline.chain.observed_at_ms = profile.baseline.now_ms - 3.0;
            timeline.observed_at_ms = profile.baseline.now_ms - 3.0;
        }
    }
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
    let bridge = build_transport_session_bridge(
        runtime_stats.clone(),
        runtime_config.clone(),
        pending_runtime_recovery_action.clone(),
    );
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1280,
            video_height: 720,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats::default(),
    );
    backend.runtime_stats = runtime_stats.clone();
    backend.pending_runtime_recovery_action = pending_runtime_recovery_action.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()),
        TestEventSink::new(events),
        backend,
    );
    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();

    {
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(
            stats
                .latest_video_frame_drop
                .as_ref()
                .map(|drop| drop.reason.as_str()),
            Some("localBackpressureRepairOverflow")
        );
    }

    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let first = transport_commands(policy.on_snapshot(&fixture.build_connected_snapshot(
        1,
        profile.baseline.now_ms,
        144,
        "adapterIdleTimeout",
    )));
    assert!(
        first.is_empty(),
        "home clean-anchor short jitter first hit should stay absorbed: {first:?}"
    );
    for command in first.iter().cloned() {
        bridge.apply_transport_session_command(SessionCommand::Transport(command));
    }

    {
        let mut stats = runtime_stats.lock().expect("runtime stats lock");
        stats.host_no_pending_streak = 5;
        stats.latest_video_host_present_time_ms = Some(profile.baseline.now_ms + 32.0);
        stats.latest_video_decode_ok_time_ms = Some(profile.baseline.now_ms + 34.0);
        stats.latest_video_packet_arrival_time_ms = Some(profile.baseline.now_ms + 35.0);
        if let Some(track) = stats.latest_video_track_status.as_mut() {
            track.video_bytes_total += 14_000;
            track.video_packet_count_total += 96;
            track.observed_at_ms = profile.baseline.now_ms + 36.0;
        }
        if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
            timeline.observation_id += 1;
            timeline.observed_at_ms = profile.baseline.now_ms + 36.0;
            timeline.chain.observed_at_ms = profile.baseline.now_ms + 36.0;
        }
    }
    let second = transport_commands(policy.on_snapshot(&fixture.build_connected_snapshot(
        2,
        profile.baseline.now_ms + 38.0,
        145,
        "adapterIdleTimeout",
    )));
    assert!(
        second.is_empty(),
        "home clean-anchor short jitter should stay absorbed while progress returns: {second:?}"
    );
    for command in second.iter().cloned() {
        bridge.apply_transport_session_command(SessionCommand::Transport(command));
    }

    {
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("home absorbed decision ledger");
        assert_eq!(ledger.input_signal, "none");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    }

    runtime.tick();
    assert_eq!(count_media_restart_requests(&requests), 0);
    assert!(pending_runtime_recovery_action
        .lock()
        .expect("lock pending runtime recovery action")
        .is_none());
}

