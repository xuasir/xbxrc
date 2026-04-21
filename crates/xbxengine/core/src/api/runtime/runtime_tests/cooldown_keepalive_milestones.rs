use super::super::{XbxEngineRuntime, XbxEngineRuntimeConfig, XbxEngineRuntimeState};
use super::fixtures::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use xbxengine_protocol::{
    XbxEngineHostRequestDto, XbxEnginePresentationMilestoneDto, XbxEngineRuntimeEventDto,
    XbxEngineTransportStateDto,
};

use crate::{XbxEngineInputStatus, XbxEngineMediaNegotiation, XbxEngineMediaRuntimeStats};

#[test]
fn runtime_applies_transport_reconnect_candidate_cooldown_and_retries_after_window() {
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
            ..Default::default()
        },
    );
    *backend
        .pending_runtime_recovery_action
        .lock()
        .expect("lock pending runtime recovery action") = Some(
        crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: 88,
            reason: "transportExpiredDeadline".to_string(),
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
    runtime.snapshot.last_recovery_action = Some("reconnect".to_string());
    runtime.snapshot.last_recovery_action_at_ms = Some(now_ms);

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
    assert_eq!(reconnect_request_count_after_first_tick, 0);
    assert!(pending
        .lock()
        .expect("lock pending runtime recovery action")
        .is_some());
    assert_eq!(
        runtime.snapshot().last_recovery_reason.as_deref(),
        Some("transportReconnectCandidateDeferred:cooldown")
    );

    runtime.snapshot.last_recovery_action_at_ms = Some(now_ms - 6_100.0);
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
fn runtime_stops_reconnect_loop_when_keepalive_reports_session_not_active() {
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
    let fail_request_kind = Rc::new(RefCell::new(Some("KeepAliveRemoteSession")));
    let host_bridge = TestHostBridge::with_failures(requests.clone(), fail_request_kind)
        .with_keepalive_failure_message(
            "keepAliveRemoteSession:streaming:HTTP 410 SessionNotActive",
        );
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        host_bridge,
        TestEventSink::new(events.clone()),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");

    runtime.tick();

    assert_eq!(runtime.state(), &XbxEngineRuntimeState::Stopped);
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        XbxEngineRuntimeEventDto::ErrorReported { code, message }
        if code == "recoverTransportReconnectSessionNotActive"
            && message.contains("HTTP 410")
    )));
}

#[test]
fn runtime_cloud_hard_disconnect_keepalive_session_not_active_stops_cleanly() {
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
            session_target_type: Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud),
            session_phase: Some("startup".to_string()),
            transport_state: XbxEngineTransportStateDto::Disconnected,
            latest_video_packet_arrival_time_ms: Some(now_ms - 2_000.0),
            inbound_video_packet_count_total: 500,
            latest_observation_label: Some("rtcConnectionRecovering".to_string()),
            latest_observation_summary: Some(
                "phase1 rtc lifecycle=Recovering state=Disconnected recoverySignalRaised=true"
                    .to_string(),
            ),
            ..Default::default()
        },
    );
    *backend
        .pending_runtime_recovery_action
        .lock()
        .expect("lock pending runtime recovery action") = Some(
        crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: 77,
            reason: "rtcConnectionRecovering".to_string(),
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
        },
    );
    let fail_request_kind = Rc::new(RefCell::new(Some("KeepAliveRemoteSession")));
    let host_bridge = TestHostBridge::with_failures(requests.clone(), fail_request_kind)
        .with_keepalive_failure_message(
            "keepAliveRemoteSession:streaming:HTTP 410 SessionNotActive",
        );
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        host_bridge,
        TestEventSink::new(events.clone()),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");

    runtime.tick();

    assert_eq!(runtime.state(), &XbxEngineRuntimeState::Stopped);
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        XbxEngineRuntimeEventDto::ErrorReported { code, message }
        if code == "recoverTransportReconnectSessionNotActive"
            && message.contains("HTTP 410")
    )));
    assert_eq!(count_media_restart_requests(&requests), 0);
}

#[test]
fn runtime_stops_when_session_is_kicked_for_closed_game() {
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
            latest_observation_label: Some("rtcSessionKickedForClosedGame".to_string()),
            latest_observation_summary: Some(
                "phase1 rtc inbound message kick reason=KickForClosedGame observationId=66"
                    .to_string(),
            ),
            ..Default::default()
        },
    );
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()),
        TestEventSink::new(events.clone()),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();

    runtime.tick();

    assert_eq!(runtime.state(), &XbxEngineRuntimeState::Stopped);
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        XbxEngineRuntimeEventDto::ErrorReported { code, message }
        if code == "recoverTransportSessionKickedForClosedGame"
            && message.contains("KickForClosedGame")
    )));
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
}

#[test]
fn runtime_waits_for_real_frame_before_populating_first_frame_timestamps() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1920,
            video_height: 1080,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_frame: None,
            ..Default::default()
        },
    );
    let runtime_stats = backend.runtime_stats.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests),
        TestEventSink::new(events.clone()),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");

    assert_eq!(runtime.snapshot().first_frame_packet_arrival_time_ms, None);
    assert_eq!(runtime.snapshot().frame_decoded_time_ms, None);
    assert!(!events.borrow().iter().any(|event| matches!(
        event,
        XbxEngineRuntimeEventDto::StatsVideoFrameRendered { .. }
    )));

    let frame_time_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    overwrite_runtime_stats(
        &runtime_stats,
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            latest_video_frame: Some(crate::XbxEngineVideoFrameStats {
                width: 1920,
                height: 1080,
                frame_seq: 1,
                fps: 60.0,
                rendered_at_ms: frame_time_ms,
            }),
            ..Default::default()
        },
    );

    runtime.tick();

    assert_eq!(
        runtime.snapshot().first_frame_packet_arrival_time_ms,
        Some(frame_time_ms)
    );
    assert_eq!(
        runtime.snapshot().frame_decoded_time_ms,
        Some(frame_time_ms)
    );
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        XbxEngineRuntimeEventDto::StatsVideoFrameRendered {
            first_frame_packet_arrival_time_ms,
            frame_decoded_time_ms,
            renderer_frame_time_ms,
        }
            if *first_frame_packet_arrival_time_ms == frame_time_ms
                && *frame_decoded_time_ms == frame_time_ms
                && *renderer_frame_time_ms == frame_time_ms
    )));
}

#[test]
fn runtime_emits_connected_presentation_milestone_after_control_and_ingress_ready() {
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
            video_width: 1920,
            video_height: 1080,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            message_handshake_acked_at_ms: Some(now_ms - 100.0),
            control_ready_at_ms: Some(now_ms - 80.0),
            latest_video_packet_arrival_time_ms: Some(now_ms - 30.0),
            ..Default::default()
        },
    );
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests),
        TestEventSink::new(events.clone()),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");

    assert_eq!(
        runtime.snapshot().presentation_milestone,
        Some(XbxEnginePresentationMilestoneDto::Connected)
    );
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        XbxEngineRuntimeEventDto::PresentationMilestoneChanged {
            milestone: XbxEnginePresentationMilestoneDto::Connected,
            ..
        }
    )));
}

#[test]
fn runtime_emits_media_ready_from_host_present_even_when_renderer_shadow_stalled() {
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
            video_width: 1920,
            video_height: 1080,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            message_handshake_acked_at_ms: Some(now_ms - 100.0),
            control_ready_at_ms: Some(now_ms - 80.0),
            latest_video_packet_arrival_time_ms: Some(now_ms - 30.0),
            latest_video_host_present_time_ms: Some(now_ms - 16.0),
            video_present_fps: 60.0,
            video_decoder_stalled: Some(false),
            video_renderer_stalled: Some(true),
            host_no_pending_pressure_level: Some("normal".to_string()),
            ..Default::default()
        },
    );
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests),
        TestEventSink::new(events.clone()),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");

    assert_eq!(
        runtime.snapshot().presentation_milestone,
        Some(XbxEnginePresentationMilestoneDto::MediaReady)
    );
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        XbxEngineRuntimeEventDto::PresentationMilestoneChanged {
            milestone: XbxEnginePresentationMilestoneDto::MediaReady,
            ..
        }
    )));
}

#[test]
fn runtime_syncs_keyframe_request_count_from_media_stats() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let backend = ScriptedMediaBackend::new(
        XbxEngineMediaNegotiation {
            local_offer_sdp: "offer".to_string(),
            local_candidates: Vec::new(),
            surface_id: "surface:viewport-1".to_string(),
            video_width: 1920,
            video_height: 1080,
            first_frame_packet_arrival_time_ms: None,
            frame_decoded_time_ms: None,
            frame_rendered_time_ms: None,
            input_status: XbxEngineInputStatus::default(),
        },
        XbxEngineMediaRuntimeStats {
            transport_state: XbxEngineTransportStateDto::Connected,
            video_pli_request_count_total: 3,
            ..Default::default()
        },
    );
    let runtime_stats = backend.runtime_stats.clone();
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests),
        TestEventSink::new(events),
        backend,
    );

    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    assert_eq!(runtime.snapshot().recovery_keyframe_request_count, 3);

    runtime_stats
        .lock()
        .expect("lock runtime stats")
        .video_pli_request_count_total = 5;
    runtime.tick();

    assert_eq!(runtime.snapshot().recovery_keyframe_request_count, 5);
}
