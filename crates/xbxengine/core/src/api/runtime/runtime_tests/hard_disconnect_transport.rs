use super::super::{XbxEngineRuntime, XbxEngineRuntimeConfig};
use super::fixtures::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use xbxengine_protocol::XbxEngineTransportStateDto;

use crate::transport::rtc::facts::{
    ConnectionLifecycleStateFact, SessionCommand, TransportCommand,
};
use crate::transport::rtc::projection::{
    BweProjection, ConnectionProjection, DiagnosticsProjection, MediaProjection,
    RecoveryProjection, TransportSnapshot,
};
use crate::transport::rtc::session::actor::SessionPolicyHook;
use crate::transport::rtc::session::policy::RtcSessionPolicy;
use crate::transport::rtc::stream::video_source::test_fixtures::{
    run_local_ingress_replay_profile, LocalIngressReplayFixture,
};
use crate::{XbxEngineInputStatus, XbxEngineMediaNegotiation, XbxEngineMediaRuntimeStats};

#[test]
fn runtime_home_hard_disconnect_candidate_reaches_reconnect_restart() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    {
        let mut stats = runtime_stats.lock().expect("runtime stats lock");
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home);
        stats.session_phase = Some("steady".to_string());
        stats.transport_recovery_epoch = 71;
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.host_no_pending_pressure_level = Some("normal".to_string());
        stats.host_no_pending_streak = 0;
        stats.latest_video_host_present_time_ms = Some(11_990.0);
        stats.latest_video_decode_ok_time_ms = Some(11_994.0);
        stats.latest_video_packet_arrival_time_ms = Some(11_996.0);
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1280),
            video_height: Some(720),
            mime_type: Some("video/H264".to_string()),
            transport_state: XbxEngineTransportStateDto::Connected,
            video_bytes_total: 360_000,
            video_packet_count_total: 2_800,
            audio_bytes_total: 64_000,
            observed_at_ms: 11_998.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 51,
            source_event: "frame-observed".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                chain_break_evidence: None,

                observed_at_ms: 11_998.0,
            },
            observed_at_ms: 11_998.0,
        });
    }
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
    let bridge = build_transport_session_bridge(
        runtime_stats.clone(),
        runtime_config.clone(),
        pending_runtime_recovery_action.clone(),
    );

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

    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let snapshot = TransportSnapshot::new(
        1,
        12_020.0,
        ConnectionProjection {
            lifecycle_state: ConnectionLifecycleStateFact::Disconnected,
            control_channel_open: false,
            latest_transport_path: Some("Direct".to_string()),
            latest_rtt_ms: Some(20.0),
            last_observed_at_ms: Some(12_020.0),
            ..ConnectionProjection::default()
        },
        MediaProjection {
            frame_count: 240,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("rtcControlChannelClosed".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(12_020.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let commands = transport_commands(policy.on_snapshot(&snapshot));
    let reconnect_candidate = commands
        .iter()
        .find(|command| {
            matches!(
                command,
                TransportCommand::RequestReconnectCandidate {
                    reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
                    ..
                }
            )
        })
        .cloned()
        .expect("home hard disconnect reconnect candidate");
    bridge.apply_transport_session_command(SessionCommand::Transport(reconnect_candidate));
    runtime.tick();

    assert_eq!(count_media_restart_requests(&requests), 1);
    assert_eq!(
        runtime.snapshot().last_recovery_action.as_deref(),
        Some("reconnect")
    );
    assert!(runtime
        .snapshot()
        .last_recovery_reason
        .as_deref()
        .is_some_and(|reason| reason.starts_with("transportReconnectCandidate:")));
}

#[tokio::test]
async fn runtime_cloud_replay_promotes_expired_deadline_to_transport_reconnect_and_exits_cleanly() {
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

    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let steady = transport_commands(policy.on_snapshot(&fixture.build_connected_snapshot(
        1,
        profile.baseline.now_ms,
        240,
        "none",
    )));
    assert!(steady.is_empty(), "unexpected steady commands: {steady:?}");

    let local_noise = transport_commands(policy.on_snapshot(&fixture.build_connected_snapshot(
        2,
        profile.baseline.now_ms + 10.0,
        241,
        "none",
    )));
    assert!(
        local_noise.is_empty(),
        "unexpected local noise commands: {local_noise:?}"
    );

    fixture.mark_transport_connectivity_degraded(profile.baseline.now_ms + 30.0);
    let expired_first = transport_commands(policy.on_snapshot(&fixture.build_connected_snapshot(
        3,
        profile.baseline.now_ms + 30.0,
        241,
        "transportExpiredDeadline",
    )));
    assert!(expired_first
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));

    tokio::time::sleep(Duration::from_millis(450)).await;
    let expired_second = transport_commands(policy.on_snapshot(&fixture.build_connected_snapshot(
        4,
        profile.baseline.now_ms + 450.0,
        241,
        "transportExpiredDeadline",
    )));
    let reconnect_candidate = expired_second
        .iter()
        .find(|command| {
            matches!(
                command,
                TransportCommand::RequestReconnectCandidate { reason, .. }
                    if reason == "transportExpiredDeadline"
            )
        })
        .cloned()
        .expect("expired deadline transport reconnect candidate");
    {
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("expired deadline decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportExpiredDeadline:transportExpiredDeadline"
        );
        assert_eq!(
            ledger.gate_result,
            "pass:reconnectGranted:connectivityEvidence"
        );
        assert_eq!(ledger.action_selected, "requestReconnectCandidate");
    }
    bridge.apply_transport_session_command(SessionCommand::Transport(reconnect_candidate));
    assert!(matches!(
        pending_runtime_recovery_action
            .lock()
            .expect("lock pending runtime recovery action")
            .as_ref(),
        Some(crate::XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            reason,
            reason_domain: crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport,
            ..
        }) if reason == "transportExpiredDeadline"
    ));

    runtime.tick();
    assert_eq!(count_media_restart_requests(&requests), 1);
    assert_eq!(
        runtime.snapshot().last_recovery_reason.as_deref(),
        Some("transportReconnectCandidate:transportExpiredDeadline")
    );

    fixture.mark_transport_recovered(profile.baseline.now_ms + 930.0);
    let recovered = transport_commands(policy.on_snapshot(&fixture.build_connected_snapshot(
        6,
        profile.baseline.now_ms + 960.0,
        260,
        "none",
    )));
    assert!(
        recovered.is_empty(),
        "unexpected commands after expired deadline recovery exit: {recovered:?}"
    );

    runtime.tick();
    assert_eq!(count_media_restart_requests(&requests), 1);
    assert!(pending_runtime_recovery_action
        .lock()
        .expect("lock pending runtime recovery action")
        .is_none());
}

#[tokio::test]
async fn runtime_home_render_deadline_jitter_replay_stays_local_and_never_reaches_reconnect() {
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
        stats.session_phase = Some("steady".to_string());
        stats.transport_recovery_epoch = 7;
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 5;
        stats.latest_video_host_present_time_ms = Some(profile.baseline.now_ms - 320.0);
        stats.latest_video_decode_ok_time_ms = Some(profile.baseline.now_ms - 12.0);
        stats.latest_video_packet_arrival_time_ms = Some(profile.baseline.now_ms - 8.0);
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(true);
        stats.host_mailbox_enqueue_count_total = 300;
        stats.host_mailbox_drop_count_total = 26;
        stats.video_pacer_submit_count_total = 320;
        stats.video_pacer_drop_count_total = 12;
        stats.video_renderer_submit_count_total = 300;
        stats.video_renderer_drop_count_total = 18;
        if let Some(track) = stats.latest_video_track_status.as_mut() {
            track.video_bytes_total = 512_000;
            track.video_packet_count_total = 4_000;
            track.audio_bytes_total = 96_000;
            track.observed_at_ms = profile.baseline.now_ms - 5.0;
        }
        if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
            timeline.source_event = "frame-observed".to_string();
            timeline.chain.state = "healthy".to_string();
            timeline.chain.reason = None;
            timeline.chain.observed_at_ms = profile.baseline.now_ms - 5.0;
            timeline.observed_at_ms = profile.baseline.now_ms - 5.0;
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

    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let commands = transport_commands(policy.on_snapshot(&fixture.build_connected_snapshot(
        1,
        profile.baseline.now_ms,
        96,
        "displaySupplyCritical",
    )));
    assert!(
        commands.is_empty(),
        "display domain critical should not emit media commands: {commands:?}"
    );
    for command in commands.iter().cloned() {
        bridge.apply_transport_session_command(SessionCommand::Transport(command));
    }

    {
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("home render jitter decision ledger");
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

#[tokio::test]
async fn runtime_cloud_startup_transport_progress_replay_does_not_reconnect_before_first_frame() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    {
        let mut stats = runtime_stats.lock().expect("runtime stats lock");
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.session_phase = Some("startup".to_string());
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connecting;
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connecting,
            video_bytes_total: 48_000,
            video_packet_count_total: 320,
            audio_bytes_total: 8_000,
            observed_at_ms: 9_800.0,
        });
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
    }
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
    let _bridge = build_transport_session_bridge(
        runtime_stats.clone(),
        runtime_config.clone(),
        pending_runtime_recovery_action.clone(),
    );
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

    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let first = transport_commands(policy.on_snapshot(&build_connecting_startup_snapshot(
        1, 10_000.0, "none", 185.0, 8_500.0,
    )));
    assert!(first
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));

    {
        let mut stats = runtime_stats.lock().expect("runtime stats lock");
        if let Some(track) = stats.latest_video_track_status.as_mut() {
            track.video_bytes_total += 12_000;
            track.video_packet_count_total += 80;
            track.observed_at_ms = 21_500.0;
        }
    }
    let second = transport_commands(policy.on_snapshot(&build_connecting_startup_snapshot(
        2, 21_600.0, "none", 190.0, 8_200.0,
    )));
    assert!(second
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));

    {
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("cloud startup decision ledger");
        assert_eq!(ledger.action_selected, "none");
        assert_eq!(ledger.gate_result, "no-signal");
    }

    runtime.tick();
    assert_eq!(count_media_restart_requests(&requests), 0);
    assert!(pending_runtime_recovery_action
        .lock()
        .expect("lock pending runtime recovery action")
        .is_none());
}

#[test]
fn runtime_local_host_stall_reset_retries_after_cooldown_when_display_tick_advances() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let call_order = Arc::new(Mutex::new(Vec::new()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    {
        let mut stats = runtime_stats.lock().expect("runtime stats lock");
        stats.video_owner_reason = Some("hostPresentStalled".to_string());
        stats.video_renderer_stalled = Some(true);
        stats.host_display_tick_epoch = 10;
        stats.host_frame_present_epoch = 3;
    }
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
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
    backend.pending_runtime_recovery_action = pending_runtime_recovery_action;
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()).with_call_order(call_order.clone()),
        TestEventSink::new(events),
        backend,
    );
    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();

    runtime.tick();
    {
        let mut stats = runtime_stats.lock().expect("runtime stats lock");
        // cooldown 内即使 tick 前进，也不应重复 reset
        stats.host_display_tick_epoch = 11;
    }
    runtime.tick();
    std::thread::sleep(Duration::from_millis(2_600));
    {
        let mut stats = runtime_stats.lock().expect("runtime stats lock");
        // cooldown 后且 display tick 再次前进，应允许重试 reset（present epoch 保持不变）
        stats.host_display_tick_epoch = 12;
        stats.host_frame_present_epoch = 3;
    }
    runtime.tick();

    let order = call_order.lock().expect("call order lock");
    let reset_count = order
        .iter()
        .filter(|entry| **entry == "resetNativePresenterHostStall")
        .count();
    assert_eq!(reset_count, 2, "expected two host-stall resets: {order:?}");
}

#[test]
fn runtime_local_host_stall_reset_skips_when_present_loop_not_running() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let call_order = Arc::new(Mutex::new(Vec::new()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    {
        let mut stats = runtime_stats.lock().expect("runtime stats lock");
        stats.video_owner_reason = Some("hostPresentStalled".to_string());
        stats.video_renderer_stalled = Some(true);
        stats.host_display_tick_epoch = 0;
        stats.host_frame_present_epoch = 0;
    }
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
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
    backend.pending_runtime_recovery_action = pending_runtime_recovery_action;
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()).with_call_order(call_order.clone()),
        TestEventSink::new(events),
        backend,
    );
    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();

    runtime.tick();

    let order = call_order.lock().expect("call order lock");
    let reset_count = order
        .iter()
        .filter(|entry| **entry == "resetNativePresenterHostStall")
        .count();
    assert_eq!(reset_count, 0, "unexpected host-stall reset: {order:?}");
}

#[test]
fn runtime_local_host_stall_reset_still_runs_when_renderer_submit_is_fresh() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let call_order = Arc::new(Mutex::new(Vec::new()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    {
        let mut stats = runtime_stats.lock().expect("runtime stats lock");
        stats.video_owner_reason = Some("hostPresentStalled".to_string());
        stats.video_renderer_stalled = Some(true);
        stats.host_display_tick_epoch = 64;
        stats.host_frame_present_epoch = 10;
        stats.video_renderer_submit_count_total = 1;
    }
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
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
    backend.pending_runtime_recovery_action = pending_runtime_recovery_action;
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()).with_call_order(call_order.clone()),
        TestEventSink::new(events),
        backend,
    );
    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();

    runtime.tick();

    let order = call_order.lock().expect("call order lock");
    let reset_count = order
        .iter()
        .filter(|entry| **entry == "resetNativePresenterHostStall")
        .count();
    assert_eq!(reset_count, 1, "expected host-stall reset: {order:?}");
}

#[test]
fn runtime_display_supply_degraded_triggers_local_reset_from_owner_reason() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let call_order = Arc::new(Mutex::new(Vec::new()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    {
        let mut stats = runtime_stats.lock().expect("runtime stats lock");
        stats.video_owner_reason = Some("displaySupplyDegraded".to_string());
        stats.video_renderer_stalled = Some(false);
        stats.host_display_tick_epoch = 20;
        stats.host_frame_present_epoch = 9;
        stats.video_renderer_submit_count_total = 1;
    }
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
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
    backend.pending_runtime_recovery_action = pending_runtime_recovery_action;
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()).with_call_order(call_order.clone()),
        TestEventSink::new(events),
        backend,
    );
    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();

    runtime.tick();
    let order = call_order.lock().expect("call order lock");
    assert!(
        order
            .iter()
            .any(|entry| *entry == "resetNativePresenterDisplayRecovery"),
        "display recovery reset should follow owner reason: {order:?}"
    );
}

#[test]
fn runtime_display_supply_critical_triggers_local_reset_from_owner_reason() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let call_order = Arc::new(Mutex::new(Vec::new()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    {
        let mut stats = runtime_stats.lock().expect("runtime stats lock");
        stats.video_owner_reason = Some("displaySupplyCritical".to_string());
        stats.video_renderer_stalled = Some(false);
        stats.host_display_tick_epoch = 30;
        stats.host_frame_present_epoch = 10;
        stats.video_renderer_submit_count_total = 1;
    }
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
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
    backend.pending_runtime_recovery_action = pending_runtime_recovery_action;
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()).with_call_order(call_order.clone()),
        TestEventSink::new(events),
        backend,
    );
    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();

    runtime.tick();
    let order = call_order.lock().expect("call order lock");
    assert!(
        order
            .iter()
            .any(|entry| *entry == "resetNativePresenterDisplayRecovery"),
        "display recovery reset should follow owner reason: {order:?}"
    );
}

#[test]
fn runtime_display_supply_critical_without_fresh_post_decode_evidence_skips_local_reset() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let call_order = Arc::new(Mutex::new(Vec::new()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    {
        let mut stats = runtime_stats.lock().expect("runtime stats lock");
        stats.video_owner_reason = Some("displaySupplyCritical".to_string());
        stats.video_renderer_stalled = Some(false);
        stats.host_display_tick_epoch = 30;
        stats.host_frame_present_epoch = 10;
        stats.video_renderer_submit_count_total = 0;
        stats.latest_host_mailbox_submit_time_ms = None;
    }
    let pending_runtime_recovery_action = Arc::new(Mutex::new(None));
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
    backend.pending_runtime_recovery_action = pending_runtime_recovery_action;
    let mut runtime = XbxEngineRuntime::with_media_backend(
        XbxEngineRuntimeConfig::default(),
        TestHostBridge::new(requests.clone()).with_call_order(call_order.clone()),
        TestEventSink::new(events),
        backend,
    );
    runtime
        .start(session(), viewport(), 1.0, None, None)
        .expect("runtime start should succeed");
    requests.borrow_mut().clear();

    runtime.tick();
    let order = call_order.lock().expect("call order lock");
    assert!(
        order
            .iter()
            .all(|entry| *entry != "resetNativePresenterDisplayRecovery"),
        "display recovery reset should require fresh post-decode evidence: {order:?}"
    );
    assert_eq!(
        runtime.snapshot().last_recovery_reason.as_deref(),
        Some("displayRecoverySkipped:stalePostDecodeEvidence")
    );
}
