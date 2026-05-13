use super::super::RtcSessionPolicy;
use crate::api::backend::{XbxEngineMediaRuntimeStats, XbxEngineVideoTwccObservation};
use crate::api::runtime::XbxEngineRuntimeConfig;
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::facts::{ConnectionLifecycleStateFact, TransportCommand};
use crate::transport::rtc::policy::video_scheduling_owner::VideoSchedulingOwnerState;
use crate::transport::rtc::projection::{
    BweProjection, ConnectionProjection, DiagnosticsProjection, MediaProjection,
    RecoveryProjection, TransportSnapshot,
};
use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationReason};
use crate::transport::rtc::session::actor::SessionPolicyHook;
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::{Duration, Instant};

use super::harness::{
    assert_recovery_family_hold_semantics, set_input_rumble_burst, transport_commands,
    RecoveryIntegrationHarness,
};

#[test]
fn recovery_integration_transport_await_exits_after_completion_evidence() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let first = harness.apply(
        900.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        12,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.transport_recovery_epoch = 1;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 24;
            stats.latest_video_host_present_time_ms = Some(now_ms - 260.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 180.0);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 155_000,
                video_packet_count_total: 1_200,
                audio_bytes_total: 42_000,
                observed_at_ms: 900.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 900.0,
                    },
                    observed_at_ms: 900.0,
                });
        },
    );
    assert!(first.iter().any(
        |command| matches!(command, TransportCommand::RequestPli { reason, .. } | TransportCommand::RequestFir { reason, .. } if reason == "transportAwaitRecoveryAnchor")
    ));

    let second = harness.apply(
        930.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        13,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = None;
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 9.0);
            stats.video_anchor_clean_epoch = Some(stats.transport_recovery_epoch);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 10.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.observed_at_ms = 930.0;
            }
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total = 188_000;
                track.video_packet_count_total = 1_360;
                track.observed_at_ms = 930.0;
            }
        },
    );
    assert!(
        second.is_empty(),
        "unexpected commands after recovery completion: {second:?}"
    );
    harness.with_stats(|stats| {
        assert_eq!(
            stats.video_owner_state.as_deref(),
            Some("rebuilding-supply")
        );
    });
}

#[test]
fn recovery_integration_stale_transport_await_after_completion_evidence_stays_no_signal() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let _ = harness.apply(
        900.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        12,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.transport_recovery_epoch = 1;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 24;
            stats.latest_video_host_present_time_ms = Some(now_ms - 260.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 180.0);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 155_000,
                video_packet_count_total: 1_200,
                audio_bytes_total: 42_000,
                observed_at_ms: 900.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 900.0,
                    },
                    observed_at_ms: 900.0,
                });
        },
    );

    let commands = harness.apply_with_recovery_observed_at(
        1_260.0,
        1_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        20,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 10.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 12.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(stats.transport_recovery_epoch);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 14.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.observed_at_ms = 1_000.0;
            }
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total = 188_000;
                track.video_packet_count_total = 1_360;
                track.observed_at_ms = 1_000.0;
            }
        },
    );

    assert!(
        commands.is_empty(),
        "unexpected commands after stale transportAwait replay: {commands:?}"
    );
    harness.with_stats(|stats| {
        assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
    });
}

#[test]
fn recovery_integration_recent_transport_await_exits_when_non_idr_has_committed_delta_continuation()
{
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let first = harness.apply(
        900.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        12,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.transport_recovery_epoch = 1;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 24;
            stats.latest_video_host_present_time_ms = Some(now_ms - 260.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 180.0);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 155_000,
                video_packet_count_total: 1_200,
                audio_bytes_total: 42_000,
                observed_at_ms: 900.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 900.0,
                    },
                    observed_at_ms: 900.0,
                });
        },
    );
    assert!(first.iter().any(
        |command| matches!(command, TransportCommand::RequestPli { reason, .. } | TransportCommand::RequestFir { reason, .. } if reason == "transportAwaitRecoveryAnchor")
    ));

    let second = harness.apply(
        930.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        13,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 9.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 11.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(stats.transport_recovery_epoch);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 12.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.latest_h264_inspection_observation =
                Some(crate::XbxEngineH264InspectionObservation {
                    observation_id: 7,
                    frame_rtp_timestamp: Some(0x1020_3040),
                    nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
                    nal_count: 1,
                    vcl_nal_count: 1,
                    has_inband_sps: false,
                    has_inband_pps: false,
                    committed_sps_present: true,
                    committed_pps_present: true,
                    slice_headers_valid: true,
                    delta_continuation_ready: true,
                    parameter_sets_changed: false,
                    config_changed: false,
                    is_idr: false,
                    sample_width: None,
                    sample_height: None,
                    bootstrap_ready: false,
                    bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
                    admission_accepted: true,
                    observed_at_ms: now_ms - 1.0,

                    ..Default::default()
                });
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-complete-candidate".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.observed_at_ms = 930.0;
            }
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total = 188_000;
                track.video_packet_count_total = 1_360;
                track.observed_at_ms = 930.0;
            }
        },
    );

    assert!(
        second.is_empty(),
        "unexpected commands after committed delta continuation became available: {second:?}"
    );
    harness.with_stats(|stats| {
        assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
        assert_eq!(stats.video_owner_reason.as_deref(), Some("steady"));
        assert_eq!(stats.recovery_transport_await_unresolved, Some(false));
        assert_eq!(stats.recovery_ingress_waiting, Some(false));
        assert_eq!(stats.recovery_exit_gate.as_deref(), Some("ready"));
    });
}

#[test]
fn recovery_integration_same_unresolved_gap_transport_await_reuses_in_flight_family() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let first = harness.apply(
        10_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        240,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 71;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 160;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_400.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 380.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 420_000,
                video_packet_count_total: 3_840,
                audio_bytes_total: 64_000,
                observed_at_ms: now_ms - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 41,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: now_ms - 2.0,
                    },
                    observed_at_ms: now_ms - 2.0,
                });
        },
    );
    assert!(first.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestPli { reason, .. } | TransportCommand::RequestFir { reason, .. }
                if reason == "transportAwaitRecoveryAnchor"
        )
    }));

    for observed_at_ms in [10_650.0, 11_100.0] {
        let commands = harness.apply(
            observed_at_ms,
            ConnectionLifecycleStateFact::Connected,
            "transportAwaitRecoveryAnchor",
            240,
            |stats| {
                let now_ms = crate::transport::rtc::stats::now_ms_f64();
                stats.session_phase = Some("recovering".to_string());
                stats.transport_recovery_epoch = 71;
                stats.host_no_pending_pressure_level = Some("critical".to_string());
                stats.host_no_pending_streak = 224;
                stats.latest_video_host_present_time_ms = Some(now_ms - 2_200.0);
                stats.latest_video_decode_ok_time_ms = Some(now_ms - 420.0);
                stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
                stats.video_decoder_stalled = Some(false);
                stats.video_renderer_stalled = Some(true);
                stats.latest_video_escalation_observation =
                    Some(crate::XbxEngineVideoEscalationObservation {
                        observation_id: 41,
                        reason: "transportAwaitRecoveryAnchor".to_string(),
                        action: "requestDecoderReset".to_string(),
                        recovery_stage: "rebuilding-supply".to_string(),
                        recovery_chain_value: "anchor".to_string(),
                        recovery_failure_cost: "high".to_string(),
                        recovery_window_source: "transport-await-window".to_string(),
                        observed_at_ms: now_ms - 90.0,
                    });
                stats.latest_keyframe_request_episode =
                    Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                        episode_id: 41,
                        request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        request_kind: Some("control".to_string()),
                        status: "sent".to_string(),
                        status_detail: None,
                        requested_at_ms: now_ms - 700.0,
                        sent_at_ms: Some(now_ms - 690.0),
                        deadline_at_ms: Some(now_ms + 300.0),
                        transport_detail: None,
                        first_video_packet_at_ms: None,
                        first_video_packet_rtp_timestamp: None,
                        first_video_packet_is_keyframe: None,
                        first_keyframe_packet_at_ms: None,
                        first_keyframe_decoded_at_ms: None,
                        response_rtp_timestamp: None,
                        response_frame_seq: None,
                        response_verdict: Some("pending".to_string()),
                        lifecycle_phase: None,
                        retired_at_ms: None,
                    });
                if let Some(track) = stats.latest_video_track_status.as_mut() {
                    track.video_bytes_total += 8_000;
                    track.video_packet_count_total += 60;
                    track.observed_at_ms = now_ms - 2.0;
                }
                if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                    timeline.observation_id = 41;
                    timeline.source_event = "frame-await-recovery-anchor".to_string();
                    timeline.chain.state = "recovering".to_string();
                    timeline.chain.reason = Some("transportAwaitRecoveryAnchor".to_string());
                    timeline.chain.observed_at_ms = now_ms - 2.0;
                    timeline.observed_at_ms = now_ms - 2.0;
                }
            },
        );
        assert!(
            commands.is_empty(),
            "same unresolved gap should not emit another recovery command at ts={observed_at_ms}: {commands:?}"
        );
        harness.with_stats(|stats| {
            let ledger = stats
                .latest_recovery_decision_ledger
                .as_ref()
                .expect("recovery decision ledger");
            assert_eq!(
                ledger.input_signal,
                "transportAwaitRecoveryAnchor:transportAwaitRecoveryAnchor"
            );
            assert_recovery_family_hold_semantics(
                ledger.gate_result.as_str(),
                ledger.action_selected.as_str(),
            );
        });
    }
}

#[test]
fn recovery_integration_passive_anchor_surface_still_feeds_transport_await_family_hold() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let first = harness.apply(
        16_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        260,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 75;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 180;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_600.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 420.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 420_000,
                video_packet_count_total: 3_840,
                audio_bytes_total: 64_000,
                observed_at_ms: now_ms - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 51,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: now_ms - 2.0,
                    },
                    observed_at_ms: now_ms - 2.0,
                });
        },
    );
    assert!(first.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestPli { reason, .. } | TransportCommand::RequestFir { reason, .. }
                if reason == "transportAwaitRecoveryAnchor"
        )
    }));

    let second = harness.apply(
        16_030.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        260,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 75;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 220;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_900.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 460.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.latest_video_escalation_observation =
                Some(crate::XbxEngineVideoEscalationObservation {
                    observation_id: 52,
                    reason: "transportAwaitRecoveryAnchor".to_string(),
                    action: "requestPli".to_string(),
                    recovery_stage: "rebuilding-supply".to_string(),
                    recovery_chain_value: "anchor".to_string(),
                    recovery_failure_cost: "high".to_string(),
                    recovery_window_source: "transport-await-window".to_string(),
                    observed_at_ms: now_ms - 10.0,
                });
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 4_000;
                track.video_packet_count_total += 32;
                track.observed_at_ms = now_ms - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id = 52;
                timeline.source_event = "frame-await-recovery-anchor".to_string();
                timeline.chain.state = "recovering".to_string();
                timeline.chain.reason = Some("transportAwaitRecoveryAnchor".to_string());
                timeline.chain.observed_at_ms = now_ms - 2.0;
                timeline.observed_at_ms = now_ms - 2.0;
            }
        },
    );
    assert!(
        second.is_empty(),
        "passive anchor surface should feed family hold instead of emitting a duplicate command: {second:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert!(
            matches!(
                ledger.input_signal.as_str(),
                "transportAwaitRecoveryAnchor:transportAwaitRecoveryAnchor"
                    | "transportAwaitRecoveryAnchor:recoverySustaining"
            ),
            "unexpected input signal: {}",
            ledger.input_signal
        );
        assert_ne!(ledger.gate_result, "no-signal");
        assert_recovery_family_hold_semantics(
            ledger.gate_result.as_str(),
            ledger.action_selected.as_str(),
        );
    });
}

#[test]
fn recovery_integration_transport_await_reopens_after_clean_anchor_and_new_recovery_epoch() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let _ = harness.apply(
        12_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        280,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 72;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 180;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_500.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 420.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 520_000,
                video_packet_count_total: 4_420,
                audio_bytes_total: 72_000,
                observed_at_ms: now_ms - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 51,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: now_ms - 2.0,
                    },
                    observed_at_ms: now_ms - 2.0,
                });
        },
    );

    let held = harness.apply(
        12_700.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        280,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.transport_recovery_epoch = 72;
            stats.latest_video_escalation_observation =
                Some(crate::XbxEngineVideoEscalationObservation {
                    observation_id: 51,
                    reason: "transportAwaitRecoveryAnchor".to_string(),
                    action: "requestDecoderReset".to_string(),
                    recovery_stage: "rebuilding-supply".to_string(),
                    recovery_chain_value: "anchor".to_string(),
                    recovery_failure_cost: "high".to_string(),
                    recovery_window_source: "transport-await-window".to_string(),
                    observed_at_ms: now_ms - 80.0,
                });
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 51,
                    request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                    request_kind: Some("control".to_string()),
                    status: "requested".to_string(),
                    status_detail: None,
                    requested_at_ms: now_ms - 700.0,
                    sent_at_ms: Some(now_ms - 690.0),
                    deadline_at_ms: Some(now_ms + 300.0),
                    transport_detail: None,
                    first_video_packet_at_ms: None,
                    first_video_packet_rtp_timestamp: None,
                    first_video_packet_is_keyframe: None,
                    first_keyframe_packet_at_ms: None,
                    first_keyframe_decoded_at_ms: None,
                    response_rtp_timestamp: None,
                    response_frame_seq: None,
                    response_verdict: Some("pending".to_string()),
                    lifecycle_phase: None,
                    retired_at_ms: None,
                });
        },
    );
    assert!(
        held.is_empty(),
        "unexpected commands while same recovery episode remains in-flight: {held:?}"
    );

    let _reopened = harness.apply(
        13_020.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        289,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 73;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 210;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_800.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 520.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = Some(72);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 220.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.latest_video_escalation_observation = None;
            stats.latest_keyframe_request_episode = None;
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 16_000;
                track.video_packet_count_total += 110;
                track.observed_at_ms = now_ms - 2.0;
            }
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 52,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: now_ms - 2.0,
                    },
                    observed_at_ms: now_ms - 2.0,
                });
        },
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert!(
            matches!(
                ledger.input_signal.as_str(),
                "transportAwaitRecoveryAnchor:transportAwaitRecoveryAnchor"
                    | "transportAwaitRecoveryAnchor:recoverySustaining"
            ),
            "unexpected input signal: {}",
            ledger.input_signal
        );
        assert_ne!(ledger.action_selected, "none");
        assert_ne!(ledger.gate_result, "coalesced:keyframeInFlight");
        assert_ne!(ledger.gate_result, "coalesced:decoderResetInFlight");
    });
}

#[test]
fn recovery_integration_home_local_display_recovery_then_stale_transport_await_replay_stays_absorbed(
) {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let baseline = harness.apply(
        7_000.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        48,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 21;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(6_992.0);
            stats.latest_video_decode_ok_time_ms = Some(6_996.0);
            stats.latest_video_packet_arrival_time_ms = Some(6_997.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(21);
            stats.video_anchor_clean_observed_at_ms = Some(6_998.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 320_000,
                video_packet_count_total: 2_400,
                audio_bytes_total: 48_000,
                observed_at_ms: 6_998.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: 6_998.0,
                    },
                    observed_at_ms: 6_998.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let burst = harness.apply(
        7_120.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        49,
        |stats| {
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 132;
            stats.latest_video_host_present_time_ms = Some(6_306.0);
            stats.latest_video_decode_ok_time_ms = Some(7_108.0);
            stats.latest_video_packet_arrival_time_ms = Some(7_118.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 18_000;
                track.video_packet_count_total += 120;
                track.observed_at_ms = 7_118.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-complete-candidate".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 7_118.0;
                timeline.observed_at_ms = 7_118.0;
            }
        },
    );
    assert!(
        burst.iter().all(|command| {
            !matches!(
                command,
                TransportCommand::RequestPli { .. }
                    | TransportCommand::RequestDecoderReset { .. }
                    | TransportCommand::RequestReconnectCandidate { .. }
            )
        }),
        "local display degraded should not emit media commands: {burst:?}"
    );

    let recovered = harness.apply(
        7_190.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        54,
        |stats| {
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(7_182.0);
            stats.latest_video_decode_ok_time_ms = Some(7_186.0);
            stats.latest_video_packet_arrival_time_ms = Some(7_188.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(21);
            stats.video_anchor_clean_observed_at_ms = Some(7_189.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 20_000;
                track.video_packet_count_total += 132;
                track.observed_at_ms = 7_189.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 7_189.0;
                timeline.observed_at_ms = 7_189.0;
            }
        },
    );
    assert!(
        recovered.is_empty(),
        "local display recovery should settle without extra commands: {recovered:?}"
    );

    let replay = harness.apply_with_recovery_observed_at(
        7_214.0,
        6_990.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        57,
        |stats| {
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(7_205.0);
            stats.latest_video_decode_ok_time_ms = Some(7_208.0);
            stats.latest_video_packet_arrival_time_ms = Some(7_210.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(21);
            stats.video_anchor_clean_observed_at_ms = Some(7_212.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 24_000;
                track.video_packet_count_total += 160;
                track.observed_at_ms = 7_212.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 7_212.0;
                timeline.observed_at_ms = 7_212.0;
            }
        },
    );

    assert!(
        replay.is_empty(),
        "stale transportAwait replay after host recovery should stay absorbed: {replay:?}"
    );
    harness.with_stats(|stats| {
        assert_ne!(
            stats.video_owner_state.as_deref(),
            Some("rebuilding-supply")
        );
        assert_ne!(
            stats.video_owner_reason.as_deref(),
            Some("transportAwaitRecoveryAnchor")
        );
    });
}

#[test]
fn recovery_integration_ramp_up_absorbs_display_idle_and_short_transport_await_before_stable() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let recovering = harness.apply(
        12_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        300,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 91;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 188;
            stats.latest_video_host_present_time_ms = Some(now_ms - 980.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 18.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 4.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 680_000,
                video_packet_count_total: 5_640,
                audio_bytes_total: 112_000,
                observed_at_ms: now_ms - 4.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: now_ms - 4.0,
                    },
                    observed_at_ms: now_ms - 4.0,
                });
        },
    );
    assert!(recovering.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestPli { reason, .. } | TransportCommand::RequestFir { reason, .. }
                if reason == "transportAwaitRecoveryAnchor"
        )
    }));

    let ramp_display = harness.apply(
        12_040.0,
        ConnectionLifecycleStateFact::Connected,
        "displaySupplyDegraded",
        304,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("recovering".to_string());
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 84;
            stats.latest_video_host_present_time_ms = Some(now_ms - 240.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 8.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(91);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 1.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.latest_video_escalation_observation = None;
            stats.latest_keyframe_request_episode = None;
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 22_000;
                track.video_packet_count_total += 164;
                track.observed_at_ms = now_ms - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-complete-candidate".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = now_ms - 2.0;
                timeline.observed_at_ms = now_ms - 2.0;
            }
        },
    );
    assert!(
        ramp_display.is_empty(),
        "ramp-up display pressure should be absorbed locally: {ramp_display:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_ne!(ledger.action_selected, "requestReconnectCandidate");
    });

    let ramp_idle = harness.apply(
        12_090.0,
        ConnectionLifecycleStateFact::Connected,
        "adapterIdleTimeout",
        307,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("recovering".to_string());
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 36;
            stats.latest_video_host_present_time_ms = Some(now_ms - 78.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 8.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_anchor_clean_epoch = Some(91);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 2.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 18_000;
                track.video_packet_count_total += 136;
                track.observed_at_ms = now_ms - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = now_ms - 2.0;
                timeline.observed_at_ms = now_ms - 2.0;
            }
        },
    );
    assert!(
        ramp_idle.is_empty(),
        "ramp-up adapter idle noise should stay absorbed: {ramp_idle:?}"
    );
    harness.with_stats(|stats| {
        assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
        assert_eq!(stats.video_owner_reason.as_deref(), Some("steady"));
    });

    let short_transport = harness.apply_with_recovery_observed_at(
        12_130.0,
        12_126.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        311,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("recovering".to_string());
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 6;
            stats.latest_video_host_present_time_ms = Some(now_ms - 58.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 8.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_anchor_clean_epoch = Some(91);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 2.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 14_000;
                track.video_packet_count_total += 104;
                track.observed_at_ms = now_ms - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = now_ms - 2.0;
                timeline.observed_at_ms = now_ms - 2.0;
            }
        },
    );
    assert!(
        short_transport.is_empty(),
        "short transportAwait in ramp-up should not reopen recovery: {short_transport:?}"
    );
    harness.with_stats(|stats| {
        assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
        assert_eq!(stats.video_owner_reason.as_deref(), Some("steady"));
    });

    let settled = harness.apply(
        12_220.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        320,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 8.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 4.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_anchor_clean_epoch = Some(91);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 1.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 26_000;
                track.video_packet_count_total += 188;
                track.observed_at_ms = now_ms - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = now_ms - 2.0;
                timeline.observed_at_ms = now_ms - 2.0;
            }
        },
    );
    assert!(
        settled.is_empty(),
        "ramp-up should settle into stable without extra commands: {settled:?}"
    );
    harness.with_stats(|stats| {
        assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
    });
}

#[test]
fn recovery_integration_ramp_up_still_reescalates_on_severe_transport_await() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let _ = harness.apply(
        13_000.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        360,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 92;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 48;
            stats.latest_video_host_present_time_ms = Some(12_940.0);
            stats.latest_video_decode_ok_time_ms = Some(12_996.0);
            stats.latest_video_packet_arrival_time_ms = Some(12_998.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(92);
            stats.video_anchor_clean_observed_at_ms = Some(12_999.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 820_000,
                video_packet_count_total: 6_700,
                audio_bytes_total: 140_000,
                observed_at_ms: 12_998.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: 12_998.0,
                    },
                    observed_at_ms: 12_998.0,
                });
        },
    );

    let severe = harness.apply_with_recovery_observed_at(
        13_080.0,
        13_080.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        361,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 210;
            stats.latest_video_host_present_time_ms = Some(11_780.0);
            stats.latest_video_decode_ok_time_ms = Some(12_700.0);
            stats.latest_video_packet_arrival_time_ms = Some(13_078.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 8_000;
                track.video_packet_count_total += 42;
                track.observed_at_ms = 13_078.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-await-recovery-anchor".to_string();
                timeline.chain.state = "recovering".to_string();
                timeline.chain.reason = Some("transportAwaitRecoveryAnchor".to_string());
                timeline.chain.observed_at_ms = 13_078.0;
                timeline.observed_at_ms = 13_078.0;
            }
        },
    );

    assert!(severe.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestPli { reason, .. } | TransportCommand::RequestFir { reason, .. }
                if reason == "transportAwaitRecoveryAnchor"
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert!(
            matches!(
                ledger.input_signal.as_str(),
                "transportAwaitRecoveryAnchor:transportAwaitRecoveryAnchor"
                    | "transportAwaitRecoveryAnchor:recoverySustaining"
            ),
            "unexpected input signal: {}",
            ledger.input_signal
        );
        assert_eq!(ledger.gate_result, "pass:localProbe");
        assert_ne!(ledger.action_selected, "none");
    });
}

#[test]
fn recovery_integration_degraded_serving_does_not_close_as_recovered() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let baseline = harness.apply(
        8_000.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        120,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 31;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 8.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 4.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(31);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 3.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 360_000,
                video_packet_count_total: 3_840,
                audio_bytes_total: 64_000,
                observed_at_ms: now_ms - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: now_ms - 2.0,
                    },
                    observed_at_ms: now_ms - 2.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let degraded = harness.apply(
        8_120.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        121,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 96;
            stats.latest_video_host_present_time_ms = Some(now_ms - 720.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 280.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 180.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 18_000;
                track.video_packet_count_total += 120;
                track.observed_at_ms = now_ms - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-complete-candidate".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = now_ms - 2.0;
                timeline.observed_at_ms = now_ms - 2.0;
            }
        },
    );
    assert!(degraded
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        assert_ne!(stats.video_owner_state.as_deref(), Some("stable-serving"));
    });

    let replay = harness.apply_with_recovery_observed_at(
        8_260.0,
        8_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        122,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 110;
            stats.latest_video_host_present_time_ms = Some(now_ms - 760.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 300.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 190.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 16_000;
                track.video_packet_count_total += 108;
                track.observed_at_ms = now_ms - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-complete-candidate".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = now_ms - 2.0;
                timeline.observed_at_ms = now_ms - 2.0;
            }
        },
    );

    assert!(replay
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        // transport-await replay 可能被吸收为 no-signal，此时 ledger 会收口到 stable。
        assert!(
            matches!(ledger.state_after.as_str(), "stable" | "recovering"),
            "unexpected state_after: {}",
            ledger.state_after
        );
    });
}

#[test]
fn recovery_integration_steady_serving_ignores_stale_transport_await_diagnosis() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let baseline = harness.apply(
        2_000.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        90,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 18;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 8.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 4.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(18);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 3.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 420_000,
                video_packet_count_total: 5_040,
                audio_bytes_total: 110_000,
                observed_at_ms: now_ms - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: now_ms - 2.0,
                    },
                    observed_at_ms: now_ms - 2.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let commands = harness.apply_with_recovery_observed_at(
        2_620.0,
        2_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        91,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 8.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 4.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(18);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 3.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total = 468_000;
                track.video_packet_count_total = 5_480;
                track.observed_at_ms = now_ms - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = now_ms - 2.0;
                timeline.observed_at_ms = now_ms - 2.0;
            }
        },
    );

    assert!(
        commands.is_empty(),
        "unexpected commands when stale transportAwait tries to reopen recovery: {commands:?}"
    );
    harness.with_stats(|stats| {
        assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
        assert_eq!(stats.video_owner_reason.as_deref(), Some("steady"));
    });
}

#[test]
fn recovery_integration_fresh_transport_await_does_not_override_stable_owner_without_clean_anchor()
{
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let _ = harness.apply(
        900.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        12,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.transport_recovery_epoch = 19;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 24;
            stats.latest_video_host_present_time_ms = Some(now_ms - 260.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 180.0);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 155_000,
                video_packet_count_total: 1_200,
                audio_bytes_total: 42_000,
                observed_at_ms: 900.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 900.0,
                    },
                    observed_at_ms: 900.0,
                });
        },
    );

    let healed = harness.apply(
        930.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        13,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 8.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 5.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-complete-candidate".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 930.0;
                timeline.observed_at_ms = 930.0;
            }
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total = 188_000;
                track.video_packet_count_total = 1_360;
                track.observed_at_ms = 930.0;
            }
        },
    );
    assert!(
        healed.is_empty(),
        "unexpected commands after owner healed: {healed:?}"
    );
    harness.with_stats(|stats| {
        assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
        assert_eq!(stats.video_owner_reason.as_deref(), Some("steady"));
    });

    let commands = harness.apply_with_recovery_observed_at(
        960.0,
        960.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        14,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 7.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 4.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-complete-candidate".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 960.0;
                timeline.observed_at_ms = 960.0;
            }
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total = 220_000;
                track.video_packet_count_total = 1_520;
                track.observed_at_ms = 960.0;
            }
        },
    );

    assert!(
        commands.is_empty(),
        "unexpected commands when fresh transportAwait conflicts with stable owner: {commands:?}"
    );
    harness.with_stats(|stats| {
        assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
        assert_eq!(stats.video_owner_reason.as_deref(), Some("steady"));
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_ne!(ledger.action_selected, "requestReconnectCandidate");
    });
}

#[test]
fn recovery_integration_local_display_stays_local_recovery() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let commands = harness.apply(
        760.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        24,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.transport_recovery_epoch = 3;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 260;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_200.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 600.0);
            stats.video_renderer_stalled = Some(true);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 128_000,
                video_packet_count_total: 1_024,
                audio_bytes_total: 32_000,
                observed_at_ms: 760.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: 760.0,
                    },
                    observed_at_ms: 760.0,
                });
        },
    );
    assert!(
        commands.iter().all(|command| {
            !matches!(
                command,
                TransportCommand::RequestPli { .. }
                    | TransportCommand::RequestDecoderReset { .. }
                    | TransportCommand::RequestReconnectCandidate { .. }
            )
        }),
        "local display critical should not emit media commands: {commands:?}"
    );
    harness.with_stats(|stats| {
        assert_eq!(stats.video_owner_state.as_deref(), Some("supply-starved"));
        assert_eq!(
            stats.video_owner_reason.as_deref(),
            Some("displaySupplyCritical")
        );
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.input_signal, "none");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });
}

#[test]
fn recovery_integration_stale_idle_is_absorbed_but_transport_failure_still_reconnects() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let healthy = harness.apply(
        1_000.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        31,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 7;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(988.0);
            stats.latest_video_decode_ok_time_ms = Some(992.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(7);
            stats.video_anchor_clean_observed_at_ms = Some(994.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 128_000,
                video_packet_count_total: 1_600,
                audio_bytes_total: 36_000,
                observed_at_ms: 995.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: 995.0,
                    },
                    observed_at_ms: 995.0,
                });
        },
    );
    assert!(healthy.is_empty());

    let stale_idle = harness.apply(
        1_008.0,
        ConnectionLifecycleStateFact::Connected,
        "adapterIdleTimeout",
        32,
        |_stats| {},
    );
    assert!(
        stale_idle.is_empty(),
        "unexpected stale idle commands: {stale_idle:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });

    let reconnect = harness.apply(
        1_200.0,
        ConnectionLifecycleStateFact::Disconnected,
        "rtcConnectionDisconnected",
        32,
        |stats| {
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.video_renderer_stalled = Some(true);
        },
    );
    assert!(reconnect.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestReconnectCandidate {
                reason,
                reason_domain,
                ..
            } if reason == "rtcConnectionDisconnected"
                && *reason_domain == crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport
        )
    }));
}

#[test]
fn recovery_integration_fresh_transport_await_absorption_does_not_block_following_transport_disconnect(
) {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let baseline = harness.apply(
        2_000.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        90,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 20;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 8.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 4.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 420_000,
                video_packet_count_total: 5_040,
                audio_bytes_total: 110_000,
                observed_at_ms: now_ms - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-complete-candidate".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: now_ms - 2.0,
                    },
                    observed_at_ms: now_ms - 2.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let absorbed = harness.apply_with_recovery_observed_at(
        2_030.0,
        2_030.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        91,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 8.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 4.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total = 452_000;
                track.video_packet_count_total = 5_260;
                track.observed_at_ms = now_ms - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-complete-candidate".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = now_ms - 2.0;
                timeline.observed_at_ms = now_ms - 2.0;
            }
        },
    );
    assert!(
        absorbed.iter().any(|command| {
            matches!(
                command,
                TransportCommand::RequestPli { reason, .. } | TransportCommand::RequestFir { reason, .. }
                    if reason == "transportAwaitRecoveryAnchor"
            )
        }) || absorbed.is_empty()
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert!(matches!(
            ledger.input_signal.as_str(),
            "transportAwaitRecoveryAnchor:transportAwaitRecoveryAnchor" | "none"
        ));
        assert_eq!(stats.video_owner_state.as_deref(), Some("stable-serving"));
    });

    let reconnect = harness.apply(
        2_120.0,
        ConnectionLifecycleStateFact::Disconnected,
        "rtcConnectionDisconnected",
        91,
        |stats| {
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.video_renderer_stalled = Some(true);
        },
    );
    assert!(reconnect.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestReconnectCandidate {
                reason,
                reason_domain,
                ..
            } if reason == "rtcConnectionDisconnected"
                && *reason_domain == crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport
        )
    }));
}

#[test]
fn recovery_integration_recovering_lifecycle_overrides_fresh_transport_await_absorption() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let commands = harness.apply_with_recovery_observed_at(
        2_400.0,
        2_400.0,
        ConnectionLifecycleStateFact::Recovering,
        "transportAwaitRecoveryAnchor",
        64,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 21;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 8.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 4.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 320_000,
                video_packet_count_total: 3_400,
                audio_bytes_total: 88_000,
                observed_at_ms: now_ms - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-complete-candidate".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: now_ms - 2.0,
                    },
                    observed_at_ms: now_ms - 2.0,
                });
        },
    );

    assert!(commands.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestReconnectCandidate {
                reason,
                reason_domain,
                ..
            } if reason == "rtcConnectionRecovering"
                && *reason_domain == crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "rtcConnectionRecovering:rtcConnectionRecovering"
        );
        assert_eq!(
            ledger.gate_result,
            "pass:reconnectGranted:connectivityEvidence"
        );
    });
}

#[test]
fn recovery_integration_transport_deadline_overrides_same_tick_local_display_recovery() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let commands = harness.apply(
        2_800.0,
        ConnectionLifecycleStateFact::Connected,
        "transportExpiredDeadline",
        72,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 22;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 280;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_300.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 620.0);
            stats.video_renderer_stalled = Some(true);
            stats.video_decoder_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 128_000,
                video_packet_count_total: 1_024,
                audio_bytes_total: 32_000,
                observed_at_ms: now_ms - 4.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: now_ms - 4.0,
                    },
                    observed_at_ms: now_ms - 4.0,
                });
        },
    );

    assert!(commands.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestPli { reason, .. } | TransportCommand::RequestFir { reason, .. }
                if reason == "displaySupplyCritical"
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportExpiredDeadline:transportExpiredDeadline"
        );
        assert_eq!(ledger.gate_result, "suppressed:waitForBurst");
    });
}

#[test]
fn recovery_integration_transport_severe_deadline_overrides_same_tick_local_display_recovery() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let commands = harness.apply(
        2_820.0,
        ConnectionLifecycleStateFact::Connected,
        "transportSevereDeadline",
        73,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 23;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 320;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_180.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 700.0);
            stats.video_renderer_stalled = Some(true);
            stats.video_decoder_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 160_000,
                video_packet_count_total: 1_280,
                audio_bytes_total: 38_000,
                observed_at_ms: now_ms - 5.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 2,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: now_ms - 5.0,
                    },
                    observed_at_ms: now_ms - 5.0,
                });
        },
    );

    assert!(commands.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestPli { reason, .. } | TransportCommand::RequestFir { reason, .. }
                if reason == "displaySupplyCritical"
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportSevereDeadline:transportSevereDeadline"
        );
        assert_recovery_family_hold_semantics(
            ledger.gate_result.as_str(),
            ledger.action_selected.as_str(),
        );
    });
}

#[test]
fn recovery_integration_transport_deadline_overrides_same_tick_local_transport_await() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let commands = harness.apply(
        2_860.0,
        ConnectionLifecycleStateFact::Connected,
        "transportExpiredDeadline",
        74,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 24;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 12;
            stats.latest_video_host_present_time_ms = Some(now_ms - 220.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 180.0);
            stats.video_renderer_stalled = Some(false);
            stats.video_decoder_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 192_000,
                video_packet_count_total: 1_540,
                audio_bytes_total: 44_000,
                observed_at_ms: now_ms - 4.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 3,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: now_ms - 3.0,
                    },
                    observed_at_ms: now_ms - 3.0,
                });
        },
    );

    assert!(commands.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestPli { reason, .. } | TransportCommand::RequestFir { reason, .. }
                if reason == "transportAwaitRecoveryAnchor"
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportExpiredDeadline:transportExpiredDeadline"
        );
        assert_eq!(ledger.gate_result, "pass");
    });
}

#[test]
fn recovery_integration_transport_severe_deadline_overrides_same_tick_local_transport_await() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let commands = harness.apply(
        2_900.0,
        ConnectionLifecycleStateFact::Connected,
        "transportSevereDeadline",
        75,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 25;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 8;
            stats.latest_video_host_present_time_ms = Some(now_ms - 260.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 210.0);
            stats.video_renderer_stalled = Some(false);
            stats.video_decoder_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 205_000,
                video_packet_count_total: 1_620,
                audio_bytes_total: 46_000,
                observed_at_ms: now_ms - 5.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 4,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: now_ms - 4.0,
                    },
                    observed_at_ms: now_ms - 4.0,
                });
        },
    );

    assert!(commands.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestPli { reason, .. } | TransportCommand::RequestFir { reason, .. }
                if reason == "transportAwaitRecoveryAnchor"
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportSevereDeadline:transportSevereDeadline"
        );
        assert_recovery_family_hold_semantics(
            ledger.gate_result.as_str(),
            ledger.action_selected.as_str(),
        );
    });
}

#[test]
fn recovery_integration_repeated_transport_severe_deadline_stays_local_without_connectivity_evidence(
) {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let first = harness.apply(
        2_940.0,
        ConnectionLifecycleStateFact::Connected,
        "transportSevereDeadline",
        76,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 26;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 340;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_260.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 760.0);
            stats.video_renderer_stalled = Some(true);
            stats.video_decoder_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 212_000,
                video_packet_count_total: 1_700,
                audio_bytes_total: 48_000,
                observed_at_ms: now_ms - 5.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 5,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: now_ms - 5.0,
                    },
                    observed_at_ms: now_ms - 5.0,
                });
        },
    );
    assert!(
        first.is_empty(),
        "first severe signal should stay in cooldown on first hit: {first:?}"
    );

    let second = harness.apply(
        2_970.0,
        ConnectionLifecycleStateFact::Connected,
        "transportSevereDeadline",
        76,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 360;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_320.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 810.0);
            stats.video_renderer_stalled = Some(true);
            stats.video_decoder_stalled = Some(false);
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id = 6;
                timeline.source_event = "frame-await-recovery-anchor".to_string();
                timeline.chain.state = "recovering".to_string();
                timeline.chain.reason = Some("transportAwaitRecoveryAnchor".to_string());
                timeline.chain.observed_at_ms = now_ms - 4.0;
                timeline.observed_at_ms = now_ms - 4.0;
            }
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.observed_at_ms = now_ms - 4.0;
            }
        },
    );

    assert!(second
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportSevereDeadline:transportSevereDeadline"
        );
        assert_eq!(
            ledger.gate_result,
            "suppressed:reconnectBlocked:transportGate:awaitingRecoveryChain"
        );
        assert_eq!(ledger.action_selected, "cooldownSuppressed");
    });
}

#[test]
fn recovery_integration_repeated_transport_expired_deadline_stays_local_without_connectivity_evidence(
) {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let mut last_commands = Vec::new();
    for idx in 0..3 {
        last_commands = harness.apply(
            3_020.0 + (idx as f64) * 420.0,
            ConnectionLifecycleStateFact::Connected,
            "transportExpiredDeadline",
            80,
            |stats| {
                let now_ms = crate::transport::rtc::stats::now_ms_f64();
                stats.session_phase = Some("steady".to_string());
                stats.transport_recovery_epoch = 27;
                stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
                stats.host_no_pending_pressure_level = Some("critical".to_string());
                stats.host_no_pending_streak = 380;
                stats.latest_video_host_present_time_ms = Some(now_ms - 1_420.0);
                stats.latest_video_decode_ok_time_ms = Some(now_ms - 920.0);
                stats.video_renderer_stalled = Some(true);
                stats.video_decoder_stalled = Some(false);
                stats.video_anchor_clean_epoch = None;
                stats.video_anchor_clean_observed_at_ms = None;
                stats.video_anchor_clean_source_event = None;
                stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                    state: "remoteTrackAttached".to_string(),
                    video_width: Some(1920),
                    video_height: Some(1080),
                    mime_type: Some("video/H264".to_string()),
                    transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                    video_bytes_total: 240_000,
                    video_packet_count_total: 1_900,
                    audio_bytes_total: 60_000,
                    observed_at_ms: now_ms - 2.0,
                });
                stats.latest_video_timeline_observation =
                    Some(crate::XbxEngineVideoTimelineObservation {
                        observation_id: 7 + idx as u64,
                        source_event: "frame-await-recovery-anchor".to_string(),
                        gap: None,
                        frame: None,
                        chain: crate::XbxEngineVideoTimelineChainSnapshot {
                            state: "recovering".to_string(),
                            reason: Some("transportAwaitRecoveryAnchor".to_string()),
                            chain_break_evidence: None,

                            observed_at_ms: now_ms - 2.0,
                        },
                        observed_at_ms: now_ms - 2.0,
                    });
            },
        );
        if idx < 2 {
            sleep(Duration::from_millis(450));
        }
    }

    assert!(last_commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportExpiredDeadline:transportExpiredDeadline"
        );
        assert_eq!(
            ledger.gate_result,
            "suppressed:reconnectBlocked:transportGate:awaitingRecoveryChain"
        );
        assert_eq!(ledger.action_selected, "cooldownSuppressed");
    });
}

#[test]
fn recovery_integration_transport_sample_loss_overrides_same_tick_local_display_recovery() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let commands = harness.apply(
        3_120.0,
        ConnectionLifecycleStateFact::Connected,
        "transportSampleLoss",
        82,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 28;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 340;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_280.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 760.0);
            stats.video_renderer_stalled = Some(true);
            stats.video_decoder_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 208_000,
                video_packet_count_total: 1_650,
                audio_bytes_total: 50_000,
                observed_at_ms: now_ms - 4.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 10,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: now_ms - 4.0,
                    },
                    observed_at_ms: now_ms - 4.0,
                });
        },
    );

    assert!(commands.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestPli { reason, .. } | TransportCommand::RequestFir { reason, .. } if reason == "displaySupplyCritical"
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportSampleLoss:transportSampleLoss"
        );
    });
}

#[test]
fn recovery_integration_transport_sample_loss_overrides_same_tick_local_transport_await() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let commands = harness.apply(
        3_180.0,
        ConnectionLifecycleStateFact::Connected,
        "transportSampleLoss",
        83,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 29;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 12;
            stats.latest_video_host_present_time_ms = Some(now_ms - 260.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 220.0);
            stats.video_renderer_stalled = Some(false);
            stats.video_decoder_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 214_000,
                video_packet_count_total: 1_710,
                audio_bytes_total: 52_000,
                observed_at_ms: now_ms - 4.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 11,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: now_ms - 3.0,
                    },
                    observed_at_ms: now_ms - 3.0,
                });
        },
    );

    assert!(commands.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestPli { reason, .. } | TransportCommand::RequestFir { reason, .. }
                if reason == "transportAwaitRecoveryAnchor"
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportSampleLoss:transportSampleLoss"
        );
    });
}

#[test]
fn recovery_integration_transport_recovered_late_overrides_same_tick_local_display_recovery() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let commands = harness.apply(
        3_240.0,
        ConnectionLifecycleStateFact::Connected,
        "transportRecoveredLate",
        84,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 30;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 300;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_160.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 700.0);
            stats.video_renderer_stalled = Some(true);
            stats.video_decoder_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 206_000,
                video_packet_count_total: 1_600,
                audio_bytes_total: 48_000,
                observed_at_ms: now_ms - 4.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 12,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: now_ms - 4.0,
                    },
                    observed_at_ms: now_ms - 4.0,
                });
        },
    );

    assert!(commands.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestPli { reason, .. } | TransportCommand::RequestFir { reason, .. } if reason == "displaySupplyCritical"
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportRecoveredLate:transportRecoveredLate"
        );
    });
}

#[test]
fn recovery_integration_transport_recovered_late_overrides_same_tick_local_transport_await() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let commands = harness.apply(
        3_300.0,
        ConnectionLifecycleStateFact::Connected,
        "transportRecoveredLate",
        85,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 31;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 16;
            stats.latest_video_host_present_time_ms = Some(now_ms - 240.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 210.0);
            stats.video_renderer_stalled = Some(false);
            stats.video_decoder_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 218_000,
                video_packet_count_total: 1_730,
                audio_bytes_total: 54_000,
                observed_at_ms: now_ms - 4.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 13,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: now_ms - 3.0,
                    },
                    observed_at_ms: now_ms - 3.0,
                });
        },
    );

    assert!(commands.iter().all(|command| {
        !matches!(
            command,
            TransportCommand::RequestPli { reason, .. } | TransportCommand::RequestFir { reason, .. }
                if reason == "transportAwaitRecoveryAnchor"
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportRecoveredLate:transportRecoveredLate"
        );
    });
}

#[test]
fn active_adapter_idle_timeout_is_suppressed_when_render_output_is_still_fresh() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_phase = Some("steady".to_string());
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.transport_recovery_epoch = 4;
        stats.latest_video_host_present_time_ms = Some(930.0);
        stats.latest_video_decode_ok_time_ms = Some(948.0);
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
        stats.video_anchor_clean_epoch = Some(4);
        stats.video_anchor_clean_observed_at_ms = Some(940.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-anchor-submitted".to_string());
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.last_observed_at_ms = Some(1_000.0);
    let snapshot = TransportSnapshot::new(
        1,
        1_000.0,
        connection,
        MediaProjection {
            frame_count: 24,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("adapterIdleTimeout".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(1_000.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let commands = transport_commands(policy.on_snapshot(&snapshot));
    assert!(commands.is_empty(), "unexpected commands: {commands:?}");
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(ledger.input_signal, "none");
    assert_eq!(ledger.gate_result, "no-signal");
    assert_eq!(ledger.action_selected, "none");
}

#[test]
fn active_adapter_idle_timeout_still_reaches_recovery_path() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.last_observed_at_ms = Some(1_000.0);
    let snapshot = TransportSnapshot::new(
        1,
        1_000.0,
        connection,
        MediaProjection::default(),
        RecoveryProjection {
            latest_diagnosis_label: Some("adapterIdleTimeout".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(1_000.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let commands = transport_commands(policy.on_snapshot(&snapshot));
    assert!(commands.is_empty(), "unexpected commands: {commands:?}");
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(ledger.input_signal, "none");
    assert_eq!(ledger.gate_result, "no-signal");
    assert_eq!(ledger.action_selected, "none");
}

#[test]
fn realtime_adapter_idle_timeout_is_absorbed_when_render_output_is_fresh() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_phase = Some("steady".to_string());
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.transport_recovery_epoch = 7;
        stats.host_no_pending_pressure_level = Some("normal".to_string());
        stats.host_no_pending_streak = 0;
        stats.latest_video_host_present_time_ms = Some(996.0);
        stats.latest_video_decode_ok_time_ms = Some(997.0);
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
        stats.video_anchor_clean_epoch = Some(7);
        stats.video_anchor_clean_observed_at_ms = Some(995.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-anchor-submitted".to_string());
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 256_000,
            video_packet_count_total: 3_200,
            audio_bytes_total: 64_000,
            observed_at_ms: 998.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-observed".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                chain_break_evidence: None,

                observed_at_ms: 998.0,
            },
            observed_at_ms: 998.0,
        });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(18.0);
    connection.last_observed_at_ms = Some(1_000.0);

    let healthy_snapshot = TransportSnapshot::new(
        1,
        1_000.0,
        connection.clone(),
        MediaProjection {
            frame_count: 48,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(999.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let _ = transport_commands(policy.on_snapshot(&healthy_snapshot));

    let snapshot = TransportSnapshot::new(
        2,
        1_000.0,
        connection,
        MediaProjection {
            frame_count: 49,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("adapterIdleTimeout".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(1_000.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let commands = transport_commands(policy.on_snapshot(&snapshot));
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestDecoderReset { .. })));
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_ne!(ledger.action_selected, "requestDecoderReset");
    assert_ne!(ledger.action_selected, "requestReconnectCandidate");
    assert_ne!(ledger.state_after, "active-recovery");
}

#[test]
fn recovery_integration_local_ingress_drop_stays_no_signal_under_healthy_baseline() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let baseline = harness.apply(
        1_200.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        40,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 9;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(1_190.0);
            stats.latest_video_decode_ok_time_ms = Some(1_194.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(9);
            stats.video_anchor_clean_observed_at_ms = Some(1_196.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 256_000,
                video_packet_count_total: 2_400,
                audio_bytes_total: 64_000,
                observed_at_ms: 1_198.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: 1_198.0,
                    },
                    observed_at_ms: 1_198.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let commands = harness.apply(
        1_230.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        41,
        |stats| {
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(1_220.0);
            stats.latest_video_decode_ok_time_ms = Some(1_224.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(9);
            stats.video_anchor_clean_observed_at_ms = Some(1_226.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total = 288_000;
                track.video_packet_count_total = 2_640;
                track.observed_at_ms = 1_228.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 1_228.0;
                timeline.observed_at_ms = 1_228.0;
            }
            stats.latest_video_frame_drop = Some(crate::XbxEngineVideoFrameDropObservation {
                observation_id: 77,
                reason: "localBackpressureRepairOverflow".to_string(),
                stage: Some("ingress".to_string()),
                action: Some("drop".to_string()),
                detail: Some("repairQueueDropOldest:priority-repair".to_string()),
                frame_rtp_timestamp: Some(90_000),
                frame_seq: None,
                frame_recovery_disposition: None,
                frame_unrecoverable_reason: Some("localBackpressure".to_string()),
                frame_budget: None,
                replacement_decision: None,
                observed_at_ms: 1_229.0,
                width: 0,
                height: 0,
                is_keyframe: false,
                queue_depth: 6,
            });
        },
    );

    assert!(commands.is_empty(), "unexpected commands: {commands:?}");
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        assert_eq!(
            stats
                .latest_video_frame_drop
                .as_ref()
                .map(|drop| drop.reason.as_str()),
            Some("localBackpressureRepairOverflow")
        );
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.input_signal, "none");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });
}

#[test]
fn recovery_integration_local_ingress_and_stale_idle_stay_no_signal_under_healthy_baseline() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let baseline = harness.apply(
        1_300.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        52,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 11;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(1_292.0);
            stats.latest_video_decode_ok_time_ms = Some(1_296.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(11);
            stats.video_anchor_clean_observed_at_ms = Some(1_297.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 320_000,
                video_packet_count_total: 3_040,
                audio_bytes_total: 80_000,
                observed_at_ms: 1_298.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: 1_298.0,
                    },
                    observed_at_ms: 1_298.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let commands = harness.apply(
        1_332.0,
        ConnectionLifecycleStateFact::Connected,
        "adapterIdleTimeout",
        53,
        |stats| {
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(1_324.0);
            stats.latest_video_decode_ok_time_ms = Some(1_328.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(11);
            stats.video_anchor_clean_observed_at_ms = Some(1_327.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total = 352_000;
                track.video_packet_count_total = 3_280;
                track.observed_at_ms = 1_330.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 1_330.0;
                timeline.observed_at_ms = 1_330.0;
            }
            stats.latest_video_frame_drop = Some(crate::XbxEngineVideoFrameDropObservation {
                observation_id: 117,
                reason: "localBackpressureRepairOverflow".to_string(),
                stage: Some("ingress".to_string()),
                action: Some("drop".to_string()),
                detail: Some("repairQueueDropOldest:priority-repair".to_string()),
                frame_rtp_timestamp: Some(91_000),
                frame_seq: None,
                frame_recovery_disposition: None,
                frame_unrecoverable_reason: Some("localBackpressure".to_string()),
                frame_budget: None,
                replacement_decision: None,
                observed_at_ms: 1_331.0,
                width: 0,
                height: 0,
                is_keyframe: false,
                queue_depth: 6,
            });
        },
    );

    assert!(
        commands.is_empty(),
        "unexpected stale idle + local ingress commands: {commands:?}"
    );
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        assert_eq!(
            stats
                .latest_video_frame_drop
                .as_ref()
                .map(|drop| drop.reason.as_str()),
            Some("localBackpressureRepairOverflow")
        );
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.input_signal, "none");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });
}

#[test]
fn connected_track_attached_without_host_feedback_does_not_escalate_adapter_idle_timeout_during_priming_window(
) {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 64_000,
            video_packet_count_total: 800,
            audio_bytes_total: 16_000,
            observed_at_ms: 1_000.0,
        });
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(24.0);
    connection.last_observed_at_ms = Some(5_000.0);
    let snapshot = TransportSnapshot::new(
        1,
        5_000.0,
        connection,
        MediaProjection::default(),
        RecoveryProjection {
            latest_diagnosis_label: Some("adapterIdleTimeout".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(5_000.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let commands = transport_commands(policy.on_snapshot(&snapshot));
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestDecoderReset { .. })));
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_ne!(ledger.action_selected, "requestDecoderReset");
    assert_ne!(ledger.action_selected, "requestReconnectCandidate");
    assert_ne!(ledger.state_after, "active-recovery");
}

#[test]
fn connected_track_attached_without_host_feedback_stays_out_of_expensive_recovery_after_priming_window(
) {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 96_000,
            video_packet_count_total: 1_200,
            audio_bytes_total: 24_000,
            observed_at_ms: 1_000.0,
        });
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(24.0);
    connection.last_observed_at_ms = Some(37_000.0);
    let snapshot = TransportSnapshot::new(
        1,
        37_000.0,
        connection,
        MediaProjection::default(),
        RecoveryProjection {
            latest_diagnosis_label: Some("adapterIdleTimeout".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(37_000.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let commands = transport_commands(policy.on_snapshot(&snapshot));
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestDecoderReset { .. })));
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_ne!(ledger.action_selected, "requestReconnectCandidate");
    assert_ne!(ledger.action_selected, "requestDecoderReset");
    assert_ne!(ledger.state_after, "active-recovery");
    assert_ne!(ledger.state_after, "recovery-eligible");
}

#[test]
fn connected_track_attached_without_first_frame_feedback_does_not_escalate_transport_await_during_priming_window(
) {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_phase = Some("priming".to_string());
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(2560),
            video_height: Some(1440),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 20_662,
            video_packet_count_total: 23,
            audio_bytes_total: 989,
            observed_at_ms: 1_000.0,
        });
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(24.0);
    connection.last_observed_at_ms = Some(1_100.0);
    let snapshot = TransportSnapshot::new(
        1,
        1_100.0,
        connection,
        MediaProjection::default(),
        RecoveryProjection {
            latest_diagnosis_label: Some("transportAwaitRecoveryAnchor".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(1_100.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let commands = transport_commands(policy.on_snapshot(&snapshot));
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestDecoderReset { .. })));
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_ne!(ledger.action_selected, "requestDecoderReset");
    assert_ne!(ledger.action_selected, "requestReconnectCandidate");
    assert_ne!(ledger.state_after, "active-recovery");
}

#[test]
fn connected_track_attached_without_first_frame_feedback_does_not_escalate_bootstrap_missing_sps_during_priming_window(
) {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let commands = harness.apply(
        1_100.0,
        ConnectionLifecycleStateFact::Connected,
        "bootstrapMissingSps",
        0,
        |stats| {
            stats.session_phase = Some("priming".to_string());
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 180;
            stats.host_display_tick_epoch = 120;
            stats.host_frame_present_epoch = 0;
            stats.host_cadence_phase = Some("priming".to_string());
            stats.host_mailbox_enqueue_count_total = 0;
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(2560),
                video_height: Some(1440),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 20_662,
                video_packet_count_total: 23,
                audio_bytes_total: 989,
                observed_at_ms: 1_000.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-inspection-rejected-await-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("bootstrapMissingSps".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 1_098.0,
                    },
                    observed_at_ms: 1_098.0,
                });
            stats.latest_h264_inspection_observation =
                Some(crate::XbxEngineH264InspectionObservation {
                    observation_id: 1,
                    frame_rtp_timestamp: Some(1),
                    nal_types: vec!["idr".to_string()],
                    nal_count: 1,
                    vcl_nal_count: 1,
                    has_inband_sps: false,
                    has_inband_pps: false,
                    committed_sps_present: false,
                    committed_pps_present: false,
                    slice_headers_valid: true,
                    delta_continuation_ready: false,
                    parameter_sets_changed: false,
                    config_changed: false,
                    is_idr: true,
                    sample_width: None,
                    sample_height: None,
                    bootstrap_ready: false,
                    bootstrap_reject_reason: Some("bootstrapMissingSps".to_string()),
                    admission_accepted: false,
                    observed_at_ms: 1_099.0,

                    ..Default::default()
                });
        },
    );
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestDecoderReset { .. })));
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_ne!(ledger.action_selected, "requestDecoderReset");
        assert_ne!(ledger.action_selected, "requestReconnectCandidate");
        assert_ne!(ledger.state_after, "active-recovery");
        assert_eq!(stats.video_owner_state.as_deref(), Some("priming"));
    });
}

#[test]
fn connected_track_started_without_first_frame_feedback_does_not_escalate_bootstrap_missing_sps_during_priming_window(
) {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let commands = harness.apply(
        1_100.0,
        ConnectionLifecycleStateFact::Connected,
        "bootstrapMissingSps",
        61,
        |stats| {
            stats.session_phase = Some("priming".to_string());
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 180;
            stats.host_display_tick_epoch = 120;
            stats.host_frame_present_epoch = 0;
            stats.host_cadence_phase = Some("priming".to_string());
            stats.host_mailbox_enqueue_count_total = 0;
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 69_029,
                video_packet_count_total: 61,
                audio_bytes_total: 49_859,
                observed_at_ms: 1_000.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-inspection-rejected-await-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("bootstrapMissingSps".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 1_098.0,
                    },
                    observed_at_ms: 1_098.0,
                });
        },
    );
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestDecoderReset { .. })));
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_ne!(ledger.action_selected, "requestDecoderReset");
        assert_ne!(ledger.action_selected, "requestReconnectCandidate");
        assert_ne!(ledger.state_after, "active-recovery");
        assert_eq!(stats.video_owner_state.as_deref(), Some("priming"));
    });
}

#[test]
fn connected_track_attached_without_first_frame_feedback_bootstrap_missing_sps_stays_out_of_recovery_mainline_before_first_frame(
) {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    harness.policy.stream_started_at = std::time::Instant::now() - Duration::from_millis(40_000);

    let commands = harness.apply(
        40_100.0,
        ConnectionLifecycleStateFact::Connected,
        "bootstrapMissingSps",
        0,
        |stats| {
            stats.session_phase = Some("priming".to_string());
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 180;
            stats.host_display_tick_epoch = 120;
            stats.host_frame_present_epoch = 0;
            stats.host_cadence_phase = Some("priming".to_string());
            stats.host_mailbox_enqueue_count_total = 0;
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(2560),
                video_height: Some(1440),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 20_662,
                video_packet_count_total: 23,
                audio_bytes_total: 989,
                observed_at_ms: 40_098.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-inspection-rejected-await-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("bootstrapMissingSps".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 40_098.0,
                    },
                    observed_at_ms: 40_098.0,
                });
        },
    );

    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestDecoderReset { .. })));
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_ne!(ledger.action_selected, "requestDecoderReset");
        assert_ne!(ledger.action_selected, "requestReconnectCandidate");
        assert_eq!(stats.video_owner_state.as_deref(), Some("priming"));
        assert_ne!(ledger.state_after, "recovery-eligible");
        assert_ne!(ledger.state_after, "active-recovery");
    });
}

#[test]
fn connected_track_attached_without_first_frame_feedback_does_not_escalate_display_supply_degraded_during_priming_window(
) {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let commands = harness.apply(
        1_100.0,
        ConnectionLifecycleStateFact::Connected,
        "displaySupplyDegraded",
        0,
        |stats| {
            stats.session_phase = Some("priming".to_string());
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 132;
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(2560),
                video_height: Some(1440),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 20_662,
                video_packet_count_total: 23,
                audio_bytes_total: 989,
                observed_at_ms: 1_000.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-complete-candidate".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: 1_098.0,
                    },
                    observed_at_ms: 1_098.0,
                });
            stats.latest_h264_inspection_observation =
                Some(crate::XbxEngineH264InspectionObservation {
                    observation_id: 1,
                    frame_rtp_timestamp: Some(1),
                    nal_types: vec!["idr".to_string()],
                    nal_count: 1,
                    vcl_nal_count: 1,
                    has_inband_sps: false,
                    has_inband_pps: false,
                    committed_sps_present: false,
                    committed_pps_present: false,
                    slice_headers_valid: true,
                    delta_continuation_ready: false,
                    parameter_sets_changed: false,
                    config_changed: false,
                    is_idr: true,
                    sample_width: None,
                    sample_height: None,
                    bootstrap_ready: false,
                    bootstrap_reject_reason: Some("bootstrapMissingSps".to_string()),
                    admission_accepted: false,
                    observed_at_ms: 1_099.0,

                    ..Default::default()
                });
        },
    );

    assert!(commands.is_empty(), "unexpected commands: {commands:?}");
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });
}

#[test]
fn connected_track_attached_without_first_frame_feedback_stays_in_local_acquisition_after_priming_window(
) {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_phase = Some("priming".to_string());
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(2560),
            video_height: Some(1440),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 20_662,
            video_packet_count_total: 23,
            audio_bytes_total: 989,
            observed_at_ms: 1_000.0,
        });
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(24.0);
    connection.last_observed_at_ms = Some(37_000.0);
    let snapshot = TransportSnapshot::new(
        1,
        37_000.0,
        connection,
        MediaProjection::default(),
        RecoveryProjection {
            latest_diagnosis_label: Some("transportAwaitRecoveryAnchor".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(37_000.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let commands = transport_commands(policy.on_snapshot(&snapshot));
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestDecoderReset { .. })));
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_ne!(ledger.action_selected, "requestReconnectCandidate");
    assert_ne!(ledger.action_selected, "requestDecoderReset");
    assert_ne!(ledger.state_after, "active-recovery");
    assert_ne!(ledger.state_after, "recovery-eligible");
}

#[test]
fn non_idr_with_recovery_keyframe_requested_enters_transport_await_chain_after_priming_window() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_phase = Some("steady".to_string());
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.host_display_tick_epoch = 64;
        stats.host_frame_present_epoch = 32;
        stats.host_mailbox_enqueue_count_total = 256;
        stats.latest_video_host_present_time_ms = Some(36_992.0);
        stats.latest_video_decode_ok_time_ms = Some(36_994.0);
        stats.video_anchor_clean_epoch = None;
        stats.video_anchor_clean_observed_at_ms = None;
        stats.video_anchor_clean_source_event = None;
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 48_000,
            video_packet_count_total: 128,
            audio_bytes_total: 8_000,
            observed_at_ms: 36_998.0,
        });
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
        stats.latest_h264_inspection_observation =
            Some(crate::XbxEngineH264InspectionObservation {
                observation_id: 9,
                frame_rtp_timestamp: Some(777),
                nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
                nal_count: 1,
                vcl_nal_count: 1,
                has_inband_sps: false,
                has_inband_pps: false,
                committed_sps_present: true,
                committed_pps_present: true,
                slice_headers_valid: true,
                delta_continuation_ready: false,
                parameter_sets_changed: false,
                config_changed: false,
                is_idr: false,
                sample_width: Some(1920),
                sample_height: Some(1080),
                bootstrap_ready: false,
                bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
                admission_accepted: false,
                observed_at_ms: 36_999.0,

                ..Default::default()
            });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(24.0);
    connection.last_observed_at_ms = Some(37_000.0);
    let snapshot = TransportSnapshot::new(
        1,
        37_000.0,
        connection,
        MediaProjection::default(),
        RecoveryProjection {
            // `RecoveryKeyframeRequested` 在 pipeline hint 映射后应统一进入该诊断标签，
            // 这里锁住它不会回退成 `no-signal`。
            latest_diagnosis_label: Some("transportAwaitRecoveryAnchor".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(37_000.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    let commands = transport_commands(policy.on_snapshot(&snapshot));
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(
        ledger.input_signal,
        "transportAwaitRecoveryAnchor:transportAwaitRecoveryAnchor"
    );
    assert!(
        ledger.gate_result != "no-signal",
        "commands={commands:?}, gate_result={}, action_selected={}",
        ledger.gate_result,
        ledger.action_selected
    );
    assert_ne!(ledger.action_selected, "none");
}

#[test]
fn connected_track_attached_without_first_frame_feedback_bootstrap_missing_sps_remains_probation_after_first_frame_grace(
) {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    harness.policy.stream_started_at = Instant::now() - Duration::from_millis(9_000);

    let first = harness.apply(
        1_100.0,
        ConnectionLifecycleStateFact::Connected,
        "bootstrapMissingSps",
        0,
        |stats| {
            stats.session_phase = Some("priming".to_string());
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 180;
            stats.host_display_tick_epoch = 120;
            stats.host_frame_present_epoch = 0;
            stats.host_cadence_phase = Some("priming".to_string());
            stats.host_mailbox_enqueue_count_total = 0;
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(2560),
                video_height: Some(1440),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 20_662,
                video_packet_count_total: 23,
                audio_bytes_total: 989,
                observed_at_ms: 1_000.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-inspection-rejected-await-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("bootstrapMissingSps".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 1_098.0,
                    },
                    observed_at_ms: 1_098.0,
                });
            stats.latest_h264_inspection_observation =
                Some(crate::XbxEngineH264InspectionObservation {
                    observation_id: 1,
                    frame_rtp_timestamp: Some(1),
                    nal_types: vec!["idr".to_string()],
                    nal_count: 1,
                    vcl_nal_count: 1,
                    has_inband_sps: false,
                    has_inband_pps: false,
                    committed_sps_present: false,
                    committed_pps_present: false,
                    slice_headers_valid: true,
                    delta_continuation_ready: false,
                    parameter_sets_changed: false,
                    config_changed: false,
                    is_idr: true,
                    sample_width: None,
                    sample_height: None,
                    bootstrap_ready: false,
                    bootstrap_reject_reason: Some("bootstrapMissingSps".to_string()),
                    admission_accepted: false,
                    observed_at_ms: 1_099.0,

                    ..Default::default()
                });
        },
    );
    assert!(first
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestDecoderReset { .. })));
    assert!(first
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));

    let commands = harness.apply(
        1_140.0,
        ConnectionLifecycleStateFact::Connected,
        "bootstrapMissingSps",
        0,
        |stats| {
            stats.session_phase = Some("priming".to_string());
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 184;
            stats.host_display_tick_epoch = 124;
            stats.host_frame_present_epoch = 0;
            stats.host_cadence_phase = Some("priming".to_string());
            stats.host_mailbox_enqueue_count_total = 0;
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(2560),
                video_height: Some(1440),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 20_962,
                video_packet_count_total: 27,
                audio_bytes_total: 1_004,
                observed_at_ms: 1_040.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 2,
                    source_event: "frame-inspection-rejected-await-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("bootstrapMissingSps".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 1_139.0,
                    },
                    observed_at_ms: 1_139.0,
                });
            stats.latest_h264_inspection_observation =
                Some(crate::XbxEngineH264InspectionObservation {
                    observation_id: 2,
                    frame_rtp_timestamp: Some(2),
                    nal_types: vec!["idr".to_string()],
                    nal_count: 1,
                    vcl_nal_count: 1,
                    has_inband_sps: false,
                    has_inband_pps: false,
                    committed_sps_present: false,
                    committed_pps_present: false,
                    slice_headers_valid: true,
                    delta_continuation_ready: false,
                    parameter_sets_changed: false,
                    config_changed: false,
                    is_idr: true,
                    sample_width: None,
                    sample_height: None,
                    bootstrap_ready: false,
                    bootstrap_reject_reason: Some("bootstrapMissingSps".to_string()),
                    admission_accepted: false,
                    observed_at_ms: 1_139.0,

                    ..Default::default()
                });
        },
    );

    let (input_signal, gate_result, action_selected, owner_state, state_after) = harness
        .with_stats(|stats| {
            let ledger = stats
                .latest_recovery_decision_ledger
                .as_ref()
                .expect("recovery decision ledger");
            (
                ledger.input_signal.clone(),
                ledger.gate_result.clone(),
                ledger.action_selected.clone(),
                stats.video_owner_state.clone(),
                ledger.state_after.clone(),
            )
        });
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestDecoderReset { .. })),
        "unexpected decoder reset: {commands:?}, input_signal={input_signal}, gate_result={gate_result}, action_selected={action_selected}, owner_state={owner_state:?}, state_after={state_after}"
    );
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })),
        "unexpected reconnect: {commands:?}, input_signal={input_signal}, gate_result={gate_result}, action_selected={action_selected}, owner_state={owner_state:?}, state_after={state_after}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_ne!(ledger.action_selected, "requestDecoderReset");
        assert_ne!(ledger.action_selected, "requestReconnectCandidate");
        assert_ne!(ledger.state_after, "recovery-eligible");
        assert_ne!(ledger.state_after, "active-recovery");
        assert_eq!(stats.video_owner_state.as_deref(), Some("priming"));
    });
}

#[test]
fn pre_first_frame_bootstrap_probes_do_not_enter_active_recovery_before_first_frame_feedback() {
    for reason in [
        "bootstrapMissingSps",
        "bootstrapMissingPps",
        "inspectionRejectInvalidSliceHeader",
        "NonIdrVcl",
    ] {
        let mut harness = RecoveryIntegrationHarness::new(Some(
            xbxengine_protocol::XbxEngineTargetTypeDto::Cloud,
        ));
        harness.policy.stream_started_at = Instant::now() - Duration::from_millis(40_000);

        let commands = harness.apply(
            40_100.0,
            ConnectionLifecycleStateFact::Connected,
            reason,
            0,
            |stats| {
                stats.session_phase = Some("priming".to_string());
                stats.host_no_pending_pressure_level = Some("critical".to_string());
                stats.host_no_pending_streak = 180;
                stats.host_display_tick_epoch = 120;
                stats.host_frame_present_epoch = 0;
                stats.host_cadence_phase = Some("priming".to_string());
                stats.host_mailbox_enqueue_count_total = 0;
                stats.video_decoder_stalled = Some(false);
                stats.video_renderer_stalled = Some(false);
                stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                    state: "remoteTrackAttached".to_string(),
                    video_width: Some(2560),
                    video_height: Some(1440),
                    mime_type: Some("video/H264".to_string()),
                    transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                    video_bytes_total: 20_662,
                    video_packet_count_total: 23,
                    audio_bytes_total: 989,
                    observed_at_ms: 40_098.0,
                });
                stats.latest_video_timeline_observation =
                    Some(crate::XbxEngineVideoTimelineObservation {
                        observation_id: 1,
                        source_event: "frame-inspection-rejected-await-anchor".to_string(),
                        gap: None,
                        frame: None,
                        chain: crate::XbxEngineVideoTimelineChainSnapshot {
                            state: "recovering".to_string(),
                            reason: Some(reason.to_string()),
                            chain_break_evidence: None,

                            observed_at_ms: 40_098.0,
                        },
                        observed_at_ms: 40_098.0,
                    });
                stats.latest_h264_inspection_observation =
                    Some(crate::XbxEngineH264InspectionObservation {
                        observation_id: 1,
                        frame_rtp_timestamp: Some(1),
                        nal_types: if reason == "NonIdrVcl" {
                            vec!["SliceLayerWithoutPartitioningNonIdr".to_string()]
                        } else {
                            vec!["idr".to_string()]
                        },
                        nal_count: 1,
                        vcl_nal_count: 1,
                        has_inband_sps: false,
                        has_inband_pps: false,
                        committed_sps_present: reason == "NonIdrVcl",
                        committed_pps_present: reason == "NonIdrVcl",
                        slice_headers_valid: reason != "inspectionRejectInvalidSliceHeader",
                        delta_continuation_ready: false,
                        parameter_sets_changed: false,
                        config_changed: false,
                        is_idr: reason != "NonIdrVcl",
                        sample_width: None,
                        sample_height: None,
                        bootstrap_ready: false,
                        bootstrap_reject_reason: Some(reason.to_string()),
                        admission_accepted: false,
                        observed_at_ms: 40_099.0,

                        ..Default::default()
                    });
            },
        );

        assert!(
            commands
                .iter()
                .all(|command| !matches!(command, TransportCommand::RequestDecoderReset { .. })),
            "unexpected decoder reset for reason={reason}: {commands:?}"
        );
        assert!(
            commands.iter().all(|command| !matches!(
                command,
                TransportCommand::RequestReconnectCandidate { .. }
            )),
            "unexpected reconnect for reason={reason}: {commands:?}"
        );
        harness.with_stats(|stats| {
            let ledger = stats
                .latest_recovery_decision_ledger
                .as_ref()
                .expect("recovery decision ledger");
            assert_ne!(
                ledger.action_selected, "requestDecoderReset",
                "reason={reason}"
            );
            assert_ne!(
                ledger.action_selected, "requestReconnectCandidate",
                "reason={reason}"
            );
            assert_ne!(ledger.state_after, "recovery-eligible", "reason={reason}");
            assert_ne!(ledger.state_after, "active-recovery", "reason={reason}");
        });
    }
}

#[test]
fn pre_first_frame_transport_await_stall_does_not_upgrade_to_reset_or_reconnect() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    harness.policy.stream_started_at = Instant::now() - Duration::from_millis(40_000);

    let commands = harness.apply(
        41_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        0,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 220;
            stats.host_display_tick_epoch = 120;
            stats.host_frame_present_epoch = 0;
            stats.host_cadence_phase = Some("priming".to_string());
            stats.host_mailbox_enqueue_count_total = 0;
            stats.video_decoder_stalled = Some(true);
            stats.video_renderer_stalled = Some(true);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(2560),
                video_height: Some(1440),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 20_662,
                video_packet_count_total: 23,
                audio_bytes_total: 989,
                observed_at_ms: 36_000.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 11,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("awaitingRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 36_000.0,
                    },
                    observed_at_ms: 36_000.0,
                });
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 11,
                    request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                    request_kind: Some("pli".to_string()),
                    status: "sent".to_string(),
                    status_detail: None,
                    requested_at_ms: 40_900.0,
                    sent_at_ms: Some(40_901.0),
                    deadline_at_ms: Some(41_500.0),
                    transport_detail: None,
                    first_video_packet_at_ms: None,
                    first_video_packet_rtp_timestamp: None,
                    first_video_packet_is_keyframe: None,
                    first_keyframe_packet_at_ms: None,
                    first_keyframe_decoded_at_ms: None,
                    response_rtp_timestamp: None,
                    response_frame_seq: None,
                    response_verdict: Some("pending".to_string()),
                    lifecycle_phase: None,
                    retired_at_ms: None,
                });
        },
    );

    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    assert!(commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestDecoderReset { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_ne!(ledger.action_selected, "requestReconnectCandidate");
        assert_ne!(ledger.action_selected, "requestDecoderReset");
    });
}

#[test]
fn trace_1775292592042_short_idle_timeout_burst_is_absorbed_while_present_progress_continues() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_phase = Some("steady".to_string());
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.transport_recovery_epoch = 11;
        stats.host_no_pending_pressure_level = Some("high".to_string());
        stats.host_no_pending_streak = 3;
        stats.latest_video_host_present_time_ms = Some(1_000.0);
        stats.latest_video_decode_ok_time_ms = Some(1_001.0);
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
        stats.video_anchor_clean_epoch = Some(11);
        stats.video_anchor_clean_observed_at_ms = Some(999.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-anchor-submitted".to_string());
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 320_000,
            video_packet_count_total: 3_600,
            audio_bytes_total: 48_000,
            observed_at_ms: 1_002.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-observed".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                chain_break_evidence: None,

                observed_at_ms: 1_002.0,
            },
            observed_at_ms: 1_002.0,
        });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(21.0);
    connection.last_observed_at_ms = Some(1_008.0);

    let first = TransportSnapshot::new(
        1,
        1_008.0,
        connection.clone(),
        MediaProjection {
            frame_count: 120,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("adapterIdleTimeout".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(1_008.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let first_commands = transport_commands(policy.on_snapshot(&first));
    assert!(
        first_commands.is_empty(),
        "trace 1775292592042 的轻抖动首拍不应升级: {first_commands:?}"
    );

    if let Ok(mut stats) = runtime_stats.lock() {
        stats.host_no_pending_streak = 5;
        stats.latest_video_host_present_time_ms = Some(1_036.0);
        stats.latest_video_decode_ok_time_ms = Some(1_038.0);
        if let Some(track) = stats.latest_video_track_status.as_mut() {
            track.video_bytes_total += 24_000;
            track.video_packet_count_total += 220;
            track.observed_at_ms = 1_039.0;
        }
        if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
            timeline.observation_id += 1;
            timeline.observed_at_ms = 1_039.0;
            timeline.chain.observed_at_ms = 1_039.0;
        }
    }
    connection.last_observed_at_ms = Some(1_040.0);
    let second = TransportSnapshot::new(
        2,
        1_040.0,
        connection,
        MediaProjection {
            frame_count: 121,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("adapterIdleTimeout".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(1_040.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let second_commands = transport_commands(policy.on_snapshot(&second));
    assert!(
            second_commands.is_empty(),
            "trace 1775292592042 的短促 idle burst 在 present/decode 持续前进时应继续吸收: {second_commands:?}"
        );

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(ledger.input_signal, "none");
    assert_eq!(ledger.gate_result, "no-signal");
    assert_eq!(ledger.action_selected, "none");
}

#[test]
fn unstable_hold_requires_consecutive_confirmation_before_emit() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    if let Ok(mut config) = runtime_config.lock() {
        config.webrtc.bwe_mode = "twcc-gcc".to_string();
        config.webrtc.video_pipeline.feedback_interval_ms = 100;
    }
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.session_phase = Some("steady".to_string());
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        stats.host_no_pending_pressure_level = Some("normal".to_string());
        stats.host_no_pending_streak = 0;
        stats.latest_video_host_present_time_ms = Some(now_ms - 12.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 10.0);
        stats.video_anchor_clean_epoch = Some(0);
        stats.video_anchor_clean_observed_at_ms = Some(now_ms - 8.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-anchor-submitted".to_string());
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: None,
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 64_000,
            video_packet_count_total: 1_200,
            audio_bytes_total: 32_000,
            observed_at_ms: now_ms,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-observed".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                chain_break_evidence: None,

                observed_at_ms: now_ms - 6.0,
            },
            observed_at_ms: now_ms - 6.0,
        });
        stats.latest_video_twcc_observation = Some(XbxEngineVideoTwccObservation {
            observation_id: 1,
            source: "local-feedback".to_string(),
            feedback_packet_count: 1,
            covered_sequence_start: 1,
            covered_sequence_end: 2,
            covered_sequence_span: 2,
            observed_packet_count: 1,
            observed_byte_count: 1200,
            coverage_ratio: None,
            ledger_hit_ratio: None,
            feedback_interval_ms: None,
            arrival_span_ms: None,
            receive_bitrate_kbps: Some(0.0),
            twcc_sample_valid: true,

            twcc_invalid_reason: None,

            quality: crate::XbxEngineTwccObservationQuality::Stable,
            delivery_ratio: 1.0,
            packet_loss_ratio: 0.0,
            observed_at_ms: 1.0,
        });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    policy.last_sent_remb_kbps = 25_000;

    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.latest_transport_path = Some("Direct".to_string());
    let snapshot_first = TransportSnapshot::new(
        1,
        1.0,
        connection.clone(),
        MediaProjection::default(),
        RecoveryProjection::default(),
        BweProjection {
            latest_rtt_ms: Some(180.0),
            latest_loss_ratio_1s: Some(0.0),
            latest_actual_video_bitrate_kbps: Some(1_000.0),
            latest_observed_remb_kbps: Some(25_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(1.0),
            target_remb_kbps: Some(25_000),
            last_observed_at_ms: Some(1.0),
        },
        DiagnosticsProjection::default(),
    );
    let first_commands = transport_commands(policy.on_snapshot(&snapshot_first));
    assert!(first_commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::SetTargetRembKbps { .. })));

    let snapshot_second = TransportSnapshot::new(
        2,
        2.0,
        connection,
        MediaProjection::default(),
        RecoveryProjection::default(),
        BweProjection {
            latest_rtt_ms: Some(180.0),
            latest_loss_ratio_1s: Some(0.0),
            latest_actual_video_bitrate_kbps: Some(1_000.0),
            latest_observed_remb_kbps: Some(25_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(2.0),
            target_remb_kbps: Some(25_000),
            last_observed_at_ms: Some(2.0),
        },
        DiagnosticsProjection::default(),
    );
    let second_commands = transport_commands(policy.on_snapshot(&snapshot_second));
    assert!(second_commands.iter().any(|command| {
        matches!(
            command,
            TransportCommand::SetTargetRembKbps { reason, .. }
                if reason.contains("unstable-hold")
        )
    }));
}

#[test]
fn recovery_integration_home_clean_anchor_short_jitter_keeps_steady_serving() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let baseline = harness.apply(
        2_000.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        180,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 21;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(1_992.0);
            stats.latest_video_decode_ok_time_ms = Some(1_996.0);
            stats.latest_video_packet_arrival_time_ms = Some(1_997.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(21);
            stats.video_anchor_clean_observed_at_ms = Some(1_998.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1280),
                video_height: Some(720),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 180_000,
                video_packet_count_total: 1_600,
                audio_bytes_total: 32_000,
                observed_at_ms: 1_998.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: 1_998.0,
                    },
                    observed_at_ms: 1_998.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let jitter = harness.apply(
        2_030.0,
        ConnectionLifecycleStateFact::Connected,
        "adapterIdleTimeout",
        181,
        |stats| {
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 2;
            stats.latest_video_host_present_time_ms = Some(2_022.0);
            stats.latest_video_decode_ok_time_ms = Some(2_025.0);
            stats.latest_video_packet_arrival_time_ms = Some(2_026.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(21);
            stats.video_anchor_clean_observed_at_ms = Some(2_024.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 18_000;
                track.video_packet_count_total += 120;
                track.observed_at_ms = 2_028.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 2_028.0;
                timeline.observed_at_ms = 2_028.0;
            }
        },
    );

    assert!(
        jitter.is_empty(),
        "home clean-anchor short jitter should stay absorbed: {jitter:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.action_selected, "none");
    });
}

#[test]
fn recovery_integration_home_render_deadline_jitter_stays_local_display_path() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let commands = harness.apply(
        2_400.0,
        ConnectionLifecycleStateFact::Connected,
        "displaySupplyCritical",
        96,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 7;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 5;
            stats.latest_video_host_present_time_ms = Some(2_080.0);
            stats.latest_video_decode_ok_time_ms = Some(2_388.0);
            stats.latest_video_packet_arrival_time_ms = Some(2_392.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.host_mailbox_enqueue_count_total = 300;
            stats.host_mailbox_drop_count_total = 26;
            stats.video_pacer_submit_count_total = 320;
            stats.video_pacer_drop_count_total = 12;
            stats.video_renderer_submit_count_total = 300;
            stats.video_renderer_drop_count_total = 18;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1280),
                video_height: Some(720),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 512_000,
                video_packet_count_total: 4_000,
                audio_bytes_total: 96_000,
                observed_at_ms: 2_395.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: 2_395.0,
                    },
                    observed_at_ms: 2_395.0,
                });
        },
    );

    assert!(
        commands.iter().all(|command| {
            !matches!(
                command,
                TransportCommand::RequestPli { .. }
                    | TransportCommand::RequestDecoderReset { .. }
                    | TransportCommand::RequestReconnectCandidate { .. }
            )
        }),
        "home render deadline jitter should stay in local display domain: {commands:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.input_signal, "none");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });
}

#[test]
fn cloud_startup_transport_progress_does_not_reconnect_before_first_frame() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
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
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connecting;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(185.0);
    connection.last_observed_at_ms = Some(10_000.0);

    let first = TransportSnapshot::new(
        1,
        10_000.0,
        connection.clone(),
        MediaProjection::default(),
        RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(10_000.0),
            ..Default::default()
        },
        BweProjection {
            latest_rtt_ms: Some(185.0),
            latest_loss_ratio_1s: Some(0.0),
            latest_actual_video_bitrate_kbps: Some(8_500.0),
            latest_observed_remb_kbps: Some(12_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(10_000.0),
            target_remb_kbps: Some(12_000),
            last_observed_at_ms: Some(10_000.0),
        },
        DiagnosticsProjection::default(),
    );
    let first_commands = transport_commands(policy.on_snapshot(&first));
    assert!(first_commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));

    if let Ok(mut stats) = runtime_stats.lock() {
        if let Some(track) = stats.latest_video_track_status.as_mut() {
            track.video_bytes_total += 12_000;
            track.video_packet_count_total += 80;
            track.observed_at_ms = 21_500.0;
        }
    }
    connection.last_observed_at_ms = Some(21_600.0);
    let second = TransportSnapshot::new(
        2,
        21_600.0,
        connection,
        MediaProjection::default(),
        RecoveryProjection {
            latest_diagnosis_label: Some("none".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(21_600.0),
            ..Default::default()
        },
        BweProjection {
            latest_rtt_ms: Some(190.0),
            latest_loss_ratio_1s: Some(0.0),
            latest_actual_video_bitrate_kbps: Some(8_200.0),
            latest_observed_remb_kbps: Some(12_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(21_600.0),
            target_remb_kbps: Some(12_000),
            last_observed_at_ms: Some(21_600.0),
        },
        DiagnosticsProjection::default(),
    );
    let second_commands = transport_commands(policy.on_snapshot(&second));
    assert!(
        second_commands
            .iter()
            .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })),
        "cloud startup with transport progress should not reconnect before first frame: {second_commands:?}"
    );
}

#[test]
fn recovery_integration_cloud_reconnect_then_clean_recovery_exit_does_not_reenter_storm() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let reconnect = harness.apply(
        4_000.0,
        ConnectionLifecycleStateFact::Disconnected,
        "rtcControlChannelClosed",
        64,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 32;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 320;
            stats.latest_video_host_present_time_ms = Some(2_400.0);
            stats.latest_video_decode_ok_time_ms = Some(3_100.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 260_000,
                video_packet_count_total: 2_100,
                audio_bytes_total: 48_000,
                observed_at_ms: 3_995.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 3_995.0,
                    },
                    observed_at_ms: 3_995.0,
                });
        },
    );
    assert!(reconnect
        .iter()
        .any(|command| { matches!(command, TransportCommand::RequestReconnectCandidate { .. }) }));

    let recovered = harness.apply(
        4_220.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        82,
        |stats| {
            stats.transport_recovery_epoch = 33;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(4_210.0);
            stats.latest_video_decode_ok_time_ms = Some(4_214.0);
            stats.latest_video_packet_arrival_time_ms = Some(4_216.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(33);
            stats.video_anchor_clean_observed_at_ms = Some(4_218.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
                track.video_bytes_total += 42_000;
                track.video_packet_count_total += 240;
                track.observed_at_ms = 4_218.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 4_218.0;
                timeline.observed_at_ms = 4_218.0;
            }
        },
    );
    assert!(
        recovered.is_empty(),
        "clean recovery exit should not emit extra commands: {recovered:?}"
    );

    let replay = harness.apply_with_recovery_observed_at(
        4_260.0,
        4_020.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        86,
        |stats| {
            stats.transport_recovery_epoch = 33;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(4_252.0);
            stats.latest_video_decode_ok_time_ms = Some(4_255.0);
            stats.latest_video_packet_arrival_time_ms = Some(4_256.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(33);
            stats.video_anchor_clean_observed_at_ms = Some(4_258.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.observed_at_ms = 4_258.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 4_258.0;
                timeline.observed_at_ms = 4_258.0;
            }
        },
    );
    assert!(
        replay.is_empty(),
        "stale transportAwait replay after clean recovery should stay absorbed: {replay:?}"
    );
    harness.with_stats(|stats| {
        assert_ne!(
            stats.video_owner_state.as_deref(),
            Some("rebuilding-supply")
        );
        assert_ne!(
            stats.video_owner_reason.as_deref(),
            Some("transportAwaitRecoveryAnchor")
        );
    });
}

#[test]
fn recovery_integration_home_short_idle_blackhole_is_absorbed_until_progress_returns() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let first = harness.apply(
        5_000.0,
        ConnectionLifecycleStateFact::Connected,
        "adapterIdleTimeout",
        144,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 12;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 3;
            stats.latest_video_host_present_time_ms = Some(4_992.0);
            stats.latest_video_decode_ok_time_ms = Some(4_995.0);
            stats.latest_video_packet_arrival_time_ms = Some(4_996.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(12);
            stats.video_anchor_clean_observed_at_ms = Some(4_997.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1280),
                video_height: Some(720),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 240_000,
                video_packet_count_total: 1_920,
                audio_bytes_total: 32_000,
                observed_at_ms: 4_997.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: 4_997.0,
                    },
                    observed_at_ms: 4_997.0,
                });
        },
    );
    assert!(
        first.is_empty(),
        "home short idle burst first hit should be absorbed: {first:?}"
    );

    let second = harness.apply(
        5_038.0,
        ConnectionLifecycleStateFact::Connected,
        "adapterIdleTimeout",
        145,
        |stats| {
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 5;
            stats.latest_video_host_present_time_ms = Some(5_032.0);
            stats.latest_video_decode_ok_time_ms = Some(5_034.0);
            stats.latest_video_packet_arrival_time_ms = Some(5_035.0);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 14_000;
                track.video_packet_count_total += 96;
                track.observed_at_ms = 5_036.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.observed_at_ms = 5_036.0;
                timeline.chain.observed_at_ms = 5_036.0;
            }
        },
    );
    assert!(
        second.is_empty(),
        "home short idle burst should stay absorbed while progress resumes: {second:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.input_signal, "none");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });
}

#[test]
fn recovery_integration_home_connected_ingress_without_output_progress_reenters_local_transport_recovery(
) {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let baseline = harness.apply(
        5_400.0,
        ConnectionLifecycleStateFact::Connected,
        "adapterIdleTimeout",
        152,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 18;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 2;
            stats.latest_video_host_present_time_ms = Some(5_392.0);
            stats.latest_video_decode_ok_time_ms = Some(5_395.0);
            stats.latest_video_packet_arrival_time_ms = Some(5_396.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(18);
            stats.video_anchor_clean_observed_at_ms = Some(5_397.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1280),
                video_height: Some(720),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 260_000,
                video_packet_count_total: 2_020,
                audio_bytes_total: 36_000,
                observed_at_ms: 5_397.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: 5_397.0,
                    },
                    observed_at_ms: 5_397.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let stalled = harness.apply(
        6_120.0,
        ConnectionLifecycleStateFact::Connected,
        "adapterIdleTimeout",
        153,
        |stats| {
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 380;
            stats.latest_video_host_present_time_ms = Some(5_392.0);
            stats.latest_video_decode_ok_time_ms = Some(5_395.0);
            stats.latest_video_packet_arrival_time_ms = Some(6_116.0);
            stats.video_decoder_stalled = Some(true);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 24_000;
                track.video_packet_count_total += 160;
                track.observed_at_ms = 6_118.0;
            }
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 2,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 6_118.0,
                    },
                    observed_at_ms: 6_118.0,
                });
        },
    );

    assert!(stalled
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert!(
            matches!(
                ledger.input_signal.as_str(),
                "transportAwaitRecoveryAnchor:transportAwaitRecoveryAnchor"
                    | "transportAwaitRecoveryAnchor:recoverySustaining"
            ),
            "unexpected input signal: {}",
            ledger.input_signal
        );
        assert_eq!(ledger.gate_result, "pass:localProbe");
        assert_eq!(ledger.action_selected, "requestPli");
    });
}

#[test]
fn recovery_integration_cloud_stale_transport_await_replay_reenters_local_recovery_when_output_stalls(
) {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let reconnect = harness.apply(
        8_000.0,
        ConnectionLifecycleStateFact::Disconnected,
        "rtcControlChannelClosed",
        120,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 52;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 300;
            stats.latest_video_host_present_time_ms = Some(6_600.0);
            stats.latest_video_decode_ok_time_ms = Some(7_200.0);
            stats.latest_video_packet_arrival_time_ms = Some(7_250.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 280_000,
                video_packet_count_total: 2_180,
                audio_bytes_total: 48_000,
                observed_at_ms: 7_995.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 7_995.0,
                    },
                    observed_at_ms: 7_995.0,
                });
        },
    );
    assert!(reconnect
        .iter()
        .any(|command| matches!(command, TransportCommand::RequestReconnectCandidate { .. })));

    let replay = harness.apply_with_recovery_observed_at(
        8_740.0,
        8_020.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        124,
        |stats| {
            stats.transport_recovery_epoch = 53;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 420;
            stats.latest_video_host_present_time_ms = Some(8_110.0);
            stats.latest_video_decode_ok_time_ms = Some(8_130.0);
            stats.latest_video_packet_arrival_time_ms = Some(8_736.0);
            stats.video_decoder_stalled = Some(true);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = Some(53);
            stats.video_anchor_clean_observed_at_ms = Some(8_160.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
                track.video_bytes_total += 18_000;
                track.video_packet_count_total += 120;
                track.observed_at_ms = 8_738.0;
            }
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 2,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 8_738.0,
                    },
                    observed_at_ms: 8_738.0,
                });
        },
    );

    assert!(
        replay.iter().any(|command| {
            matches!(
                command,
                TransportCommand::RequestPli { reason, .. } | TransportCommand::RequestFir { reason, .. }
                    if reason == "transportAwaitRecoveryAnchor"
            )
        }) || replay.is_empty()
    );
    assert!(replay
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        if replay.is_empty() {
            assert!(matches!(
                ledger.input_signal.as_str(),
                "none" | "rtcConnectionRecovering:rtcConnectionDisconnected"
            ));
            if ledger.input_signal == "none" {
                assert_eq!(ledger.gate_result, "no-signal");
                assert_eq!(ledger.action_selected, "none");
            }
        } else {
            assert!(matches!(
                ledger.input_signal.as_str(),
                "transportAwaitRecoveryAnchor:transportAwaitRecoveryAnchor"
                    | "rtcConnectionRecovering:rtcConnectionDisconnected"
            ));
            if ledger.input_signal == "transportAwaitRecoveryAnchor:transportAwaitRecoveryAnchor" {
                assert_eq!(ledger.gate_result, "pass:localProbe");
                assert_eq!(ledger.action_selected, "requestPli");
            }
        }
    });
}

#[test]
fn recovery_integration_fresh_transport_await_absorption_expires_once_output_stalls() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let baseline = harness.apply(
        9_200.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        188,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 61;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 8.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 4.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(61);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 2.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 520_000,
                video_packet_count_total: 4_320,
                audio_bytes_total: 82_000,
                observed_at_ms: now_ms - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: now_ms - 2.0,
                    },
                    observed_at_ms: now_ms - 2.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let absorbed = harness.apply_with_recovery_observed_at(
        9_240.0,
        9_240.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        189,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 8.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 4.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 18_000;
                track.video_packet_count_total += 140;
                track.observed_at_ms = now_ms - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id = 2;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = now_ms - 2.0;
                timeline.observed_at_ms = now_ms - 2.0;
            }
        },
    );
    assert!(
        absorbed.is_empty(),
        "fresh transportAwait with healthy output should stay absorbed: {absorbed:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("absorbed decision ledger");
        assert_eq!(ledger.input_signal, "none");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });

    let stalled = harness.apply(
        9_620.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        193,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 360;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_320.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 820.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 2.0);
            stats.video_decoder_stalled = Some(true);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = Some(61);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 420.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 20_000;
                track.video_packet_count_total += 160;
                track.observed_at_ms = now_ms - 2.0;
            }
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 3,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: now_ms - 2.0,
                    },
                    observed_at_ms: now_ms - 2.0,
                });
        },
    );

    assert!(stalled
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    assert!(stalled
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestDecoderReset { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("stalled decision ledger");
        assert!(
            matches!(
                ledger.input_signal.as_str(),
                "transportAwaitRecoveryAnchor:transportAwaitRecoveryAnchor"
                    | "transportAwaitRecoveryAnchor:recoverySustaining"
                    | "none"
            ),
            "unexpected input signal: {}",
            ledger.input_signal
        );
        if ledger.input_signal == "none" {
            assert_eq!(ledger.gate_result, "no-signal");
            assert_eq!(ledger.action_selected, "none");
        } else {
            assert_ne!(ledger.state_after, "active-recovery");
            assert_ne!(ledger.state_after, "recovery-eligible");
            if ledger.action_selected == "requestPli" {
                assert_eq!(ledger.gate_result, "pass:localProbe");
                assert_eq!(ledger.state_after, "local-self-healing");
            }
        }
        assert_ne!(stats.video_owner_state.as_deref(), Some("stable-serving"));
    });
}

#[test]
fn recovery_integration_home_stale_transport_await_absorption_expires_once_output_stalls() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let baseline = harness.apply(
        9_800.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        210,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 72;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(9_792.0);
            stats.latest_video_decode_ok_time_ms = Some(9_796.0);
            stats.latest_video_packet_arrival_time_ms = Some(9_797.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(72);
            stats.video_anchor_clean_observed_at_ms = Some(9_798.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 560_000,
                video_packet_count_total: 4_860,
                audio_bytes_total: 96_000,
                observed_at_ms: 9_798.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: 9_798.0,
                    },
                    observed_at_ms: 9_798.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let absorbed = harness.apply_with_recovery_observed_at(
        10_040.0,
        9_800.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        211,
        |stats| {
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(10_032.0);
            stats.latest_video_decode_ok_time_ms = Some(10_036.0);
            stats.latest_video_packet_arrival_time_ms = Some(10_038.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(72);
            stats.video_anchor_clean_observed_at_ms = Some(10_038.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 20_000;
                track.video_packet_count_total += 150;
                track.observed_at_ms = 10_038.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 10_038.0;
                timeline.observed_at_ms = 10_038.0;
            }
        },
    );
    assert!(
        absorbed.is_empty(),
        "stale transportAwait should stay absorbed while output is still fresh: {absorbed:?}"
    );

    let replay = harness.apply_with_recovery_observed_at(
        10_720.0,
        10_300.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        212,
        |stats| {
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 260;
            stats.latest_video_host_present_time_ms = Some(10_032.0);
            stats.latest_video_decode_ok_time_ms = Some(10_060.0);
            stats.latest_video_packet_arrival_time_ms = Some(10_718.0);
            stats.video_decoder_stalled = Some(true);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 18_000;
                track.video_packet_count_total += 140;
                track.observed_at_ms = 10_718.0;
            }
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 3,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 10_718.0,
                    },
                    observed_at_ms: 10_718.0,
                });
        },
    );
    assert!(replay.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestPli { reason, .. } | TransportCommand::RequestFir { reason, .. }
                if reason == "transportAwaitRecoveryAnchor"
        )
    }));
    assert!(replay
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportAwaitRecoveryAnchor:transportAwaitRecoveryAnchor"
        );
        assert_eq!(ledger.gate_result, "pass:localProbe");
        assert_eq!(ledger.action_selected, "requestPli");
    });
}

#[test]
fn recovery_integration_home_burst_input_rumble_display_pressure_then_stale_transport_await_replay_stays_absorbed(
) {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let baseline = harness.apply(
        11_000.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        240,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 81;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(10_992.0);
            stats.latest_video_decode_ok_time_ms = Some(10_996.0);
            stats.latest_video_packet_arrival_time_ms = Some(10_997.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.host_mailbox_enqueue_count_total = 956;
            stats.host_mailbox_overwrite_count_total = 27;
            stats.host_mailbox_drop_count_total = 2;
            stats.video_pacer_submit_count_total = 960;
            stats.video_pacer_drop_count_total = 1;
            stats.video_renderer_submit_count_total = 956;
            stats.video_renderer_drop_count_total = 0;
            stats.video_anchor_clean_epoch = Some(81);
            stats.video_anchor_clean_observed_at_ms = Some(10_998.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            set_input_rumble_burst(stats, 1, 10_998.0, 0);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 620_000,
                video_packet_count_total: 5_320,
                audio_bytes_total: 110_000,
                observed_at_ms: 10_998.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: 10_998.0,
                    },
                    observed_at_ms: 10_998.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let burst = harness.apply(
        11_120.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        241,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 97;
            stats.latest_video_host_present_time_ms = Some(10_306.0);
            stats.latest_video_decode_ok_time_ms = Some(11_116.0);
            stats.latest_video_packet_arrival_time_ms = Some(11_118.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.host_mailbox_enqueue_count_total = 964;
            stats.host_mailbox_overwrite_count_total = 27;
            stats.host_mailbox_drop_count_total = 3;
            stats.video_pacer_submit_count_total = 969;
            stats.video_pacer_drop_count_total = 1;
            stats.video_renderer_submit_count_total = 964;
            stats.video_renderer_drop_count_total = 0;
            stats.latest_video_escalation_observation =
                Some(crate::XbxEngineVideoEscalationObservation {
                    observation_id: 284,
                    reason: "transportAwaitRecoveryAnchor".to_string(),
                    action: "requestDecoderReset".to_string(),
                    recovery_stage: "rebuilding-supply".to_string(),
                    recovery_chain_value: "anchor".to_string(),
                    recovery_failure_cost: "high".to_string(),
                    recovery_window_source: "transport-await-window".to_string(),
                    observed_at_ms: 11_014.0,
                });
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 284,
                    request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                    request_kind: Some("pli".to_string()),
                    status: "packet-seen".to_string(),
                    status_detail: None,
                    requested_at_ms: 11_010.0,
                    sent_at_ms: Some(11_010.0),
                    deadline_at_ms: Some(11_090.0),
                    transport_detail: None,
                    first_video_packet_at_ms: None,
                    first_video_packet_rtp_timestamp: None,
                    first_video_packet_is_keyframe: None,
                    first_keyframe_packet_at_ms: None,
                    first_keyframe_decoded_at_ms: None,
                    response_rtp_timestamp: None,
                    response_frame_seq: None,
                    response_verdict: None,
                    lifecycle_phase: None,
                    retired_at_ms: None,
                });
            set_input_rumble_burst(stats, 2, 11_118.0, 32);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 22_000;
                track.video_packet_count_total += 160;
                track.observed_at_ms = 11_118.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-complete-candidate".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 11_118.0;
                timeline.observed_at_ms = 11_118.0;
            }
        },
    );
    assert!(
        burst.is_empty(),
        "stale transport-await overlap should absorb display pressure burst locally: {burst:?}"
    );
    assert!(burst
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("recovery decision ledger");
        assert_eq!(ledger.input_signal, "none");
        assert_eq!(ledger.gate_result, "no-signal");
        assert_eq!(ledger.action_selected, "none");
    });

    let recovered = harness.apply(
        11_190.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        247,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(11_182.0);
            stats.latest_video_decode_ok_time_ms = Some(11_186.0);
            stats.latest_video_packet_arrival_time_ms = Some(11_188.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_escalation_observation = None;
            stats.latest_keyframe_request_episode = None;
            stats.video_anchor_clean_epoch = Some(81);
            stats.video_anchor_clean_observed_at_ms = Some(11_189.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.host_mailbox_enqueue_count_total = 972;
            stats.host_mailbox_overwrite_count_total = 27;
            stats.host_mailbox_drop_count_total = 3;
            stats.video_pacer_submit_count_total = 977;
            stats.video_pacer_drop_count_total = 1;
            stats.video_renderer_submit_count_total = 972;
            stats.video_renderer_drop_count_total = 0;
            set_input_rumble_burst(stats, 3, 11_188.0, 16);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 24_000;
                track.video_packet_count_total += 170;
                track.observed_at_ms = 11_188.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 11_188.0;
                timeline.observed_at_ms = 11_188.0;
            }
        },
    );
    assert!(
        recovered.is_empty(),
        "local display pressure should settle once output catches up: {recovered:?}"
    );

    let replay = harness.apply_with_recovery_observed_at(
        11_214.0,
        10_990.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        248,
        |stats| {
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(11_206.0);
            stats.latest_video_decode_ok_time_ms = Some(11_210.0);
            stats.latest_video_packet_arrival_time_ms = Some(11_212.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(81);
            stats.video_anchor_clean_observed_at_ms = Some(11_212.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.host_mailbox_enqueue_count_total = 974;
            stats.host_mailbox_overwrite_count_total = 27;
            stats.host_mailbox_drop_count_total = 3;
            stats.video_pacer_submit_count_total = 979;
            stats.video_pacer_drop_count_total = 1;
            stats.video_renderer_submit_count_total = 974;
            stats.video_renderer_drop_count_total = 0;
            set_input_rumble_burst(stats, 4, 11_212.0, 8);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 12_000;
                track.video_packet_count_total += 88;
                track.observed_at_ms = 11_212.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id += 1;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = 11_212.0;
                timeline.observed_at_ms = 11_212.0;
            }
        },
    );
    assert!(
        replay.is_empty(),
        "stale transportAwait replay should stay absorbed after burst input pressure settles: {replay:?}"
    );
    harness.with_stats(|stats| {
        let latest_input = stats
            .latest_data_channel_message_catalog_observation
            .as_ref()
            .expect("latest input observation");
        assert_eq!(latest_input.channel, "input");
        assert_eq!(latest_input.direction, "inbound");
        assert_ne!(
            stats.video_owner_state.as_deref(),
            Some("rebuilding-supply")
        );
        assert_ne!(
            stats.video_owner_reason.as_deref(),
            Some("transportAwaitRecoveryAnchor")
        );
    });
}

#[test]
fn recovery_integration_home_burst_input_rumble_submit_gap_and_latest_slot_overwrite_stays_local() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let commands = harness.apply(
        12_600.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        540,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 91;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 96;
            stats.host_display_tick_epoch = 1_139;
            stats.host_frame_present_epoch = 456;
            stats.host_cadence_phase = Some("starved".to_string());
            stats.latest_video_host_present_time_ms = Some(7_991.0);
            stats.latest_video_decode_ok_time_ms = Some(12_598.0);
            stats.latest_video_packet_arrival_time_ms = Some(12_599.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.host_mailbox_enqueue_count_total = 458;
            stats.host_mailbox_overwrite_count_total = 1;
            stats.host_mailbox_drop_count_total = 1;
            stats.video_pacer_submit_count_total = 533;
            stats.video_pacer_drop_count_total = 2;
            stats.video_renderer_submit_count_total = 531;
            stats.video_renderer_drop_count_total = 0;
            stats.latest_render_mailbox_decision =
                Some(crate::XbxEnginePipelineCandidateDecisionObservation {
                    decision_id: 102,
                    state: "latest-overwrite".to_string(),
                    action: "replace".to_string(),
                    detail: "mailboxOverwrite".to_string(),
                    frame_seq: Some(532),
                    replacement_decision: None,
                    observed_at_ms: 12_599.0,
                });
            set_input_rumble_burst(stats, 9, 12_599.0, 32);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 17_361_017,
                video_packet_count_total: 15_170,
                audio_bytes_total: 204_946,
                observed_at_ms: 12_599.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1_196,
                    source_event: "frame-complete-candidate".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: 12_599.0,
                    },
                    observed_at_ms: 12_599.0,
                });
        },
    );
    assert!(
        commands
            .iter()
            .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })),
        "local burst-input stall should not escalate to reconnect: {commands:?}"
    );
    harness.with_stats(|stats| {
        let latest_input = stats
            .latest_data_channel_message_catalog_observation
            .as_ref()
            .expect("latest input observation");
        assert_eq!(latest_input.channel, "input");
        assert!(matches!(
            stats.video_owner_reason.as_deref(),
            Some("displaySupplyCritical" | "displaySupplyDegraded" | "hostPresentStalled")
        ));
    });
}

#[test]
fn recovery_integration_home_hard_disconnect_emits_connectivity_reconnect_without_local_absorption()
{
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Home));

    let baseline = harness.apply(
        11_900.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        240,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 51;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(now_ms - 10.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 6.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 4.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(51);
            stats.video_anchor_clean_observed_at_ms = Some(now_ms - 3.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1280),
                video_height: Some(720),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 320_000,
                video_packet_count_total: 2_400,
                audio_bytes_total: 64_000,
                observed_at_ms: now_ms - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 21,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: now_ms - 2.0,
                    },
                    observed_at_ms: now_ms - 2.0,
                });
        },
    );
    assert!(
        baseline.is_empty(),
        "unexpected baseline commands: {baseline:?}"
    );

    let disconnect = harness.apply(
        12_020.0,
        ConnectionLifecycleStateFact::Disconnected,
        "rtcControlChannelClosed",
        240,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 420;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_200.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 800.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 700.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.transport_state =
                    xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
                track.observed_at_ms = now_ms - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id = 22;
                timeline.source_event = "frame-await-recovery-anchor".to_string();
                timeline.chain.state = "recovering".to_string();
                timeline.chain.reason = Some("transportAwaitRecoveryAnchor".to_string());
                timeline.chain.observed_at_ms = now_ms - 2.0;
                timeline.observed_at_ms = now_ms - 2.0;
            }
        },
    );

    assert!(disconnect.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestReconnectCandidate {
                reason_domain,
                ..
            } if *reason_domain
                == crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("home hard disconnect decision ledger");
        assert_eq!(
            ledger.input_signal,
            "rtcConnectionRecovering:rtcConnectionDisconnected"
        );
        assert_eq!(
            ledger.gate_result,
            "pass:reconnectGranted:connectivityEvidence"
        );
        assert_eq!(ledger.action_selected, "requestReconnectCandidate");
    });
}

#[test]
fn recovery_integration_cloud_media_loss_prefers_transport_await_before_reconnect() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let local_recover = harness.apply(
        6_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        220,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 18;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 12;
            stats.latest_video_host_present_time_ms = Some(5_760.0);
            stats.latest_video_decode_ok_time_ms = Some(5_785.0);
            stats.latest_video_packet_arrival_time_ms = Some(5_998.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 410_000,
                video_packet_count_total: 3_100,
                audio_bytes_total: 64_000,
                observed_at_ms: 5_998.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: 5_998.0,
                    },
                    observed_at_ms: 5_998.0,
                });
        },
    );
    assert!(local_recover.iter().any(|command| {
        matches!(command, TransportCommand::RequestPli { reason, .. } | TransportCommand::RequestFir { reason, .. } if reason == "transportAwaitRecoveryAnchor")
    }));
    assert!(local_recover
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("local recover ledger");
        assert_eq!(
            ledger.input_signal,
            "transportAwaitRecoveryAnchor:transportAwaitRecoveryAnchor"
        );
        assert_eq!(ledger.gate_result, "pass:localProbe");
        assert_eq!(ledger.action_selected, "requestPli");
        assert_eq!(ledger.state_after, "active-recovery");
    });

    let reconnect = harness.apply(
        6_420.0,
        ConnectionLifecycleStateFact::Recovering,
        "rtcConnectionRecovering",
        220,
        |stats| {
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 220;
            stats.latest_video_host_present_time_ms = Some(5_760.0);
            stats.latest_video_decode_ok_time_ms = Some(5_785.0);
            stats.latest_video_packet_arrival_time_ms = Some(6_000.0);
            stats.video_renderer_stalled = Some(true);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.transport_state =
                    xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
                track.observed_at_ms = 6_418.0;
            }
        },
    );
    assert!(reconnect.iter().any(|command| {
        matches!(command, TransportCommand::RequestReconnectCandidate { reason, .. } if reason == "rtcConnectionRecovering")
    }));
}

#[test]
fn recovery_integration_trace_contract_continuation_heavy_first_hit_requests_pli_not_reconnect() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let first = harness.apply(
        10_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        220,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 24;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 28;
            stats.latest_video_host_present_time_ms = Some(9_520.0);
            stats.latest_video_decode_ok_time_ms = Some(9_610.0);
            stats.latest_video_packet_arrival_time_ms = Some(9_998.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 410_000,
                video_packet_count_total: 3_100,
                audio_bytes_total: 64_000,
                observed_at_ms: 9_998.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,
                        observed_at_ms: 9_998.0,
                    },
                    observed_at_ms: 9_998.0,
                });
        },
    );
    assert!(first.iter().any(|command| {
        matches!(command, TransportCommand::RequestPli { reason, .. } if reason == "transportAwaitRecoveryAnchor")
    }));
    assert!(first
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
}

#[test]
fn recovery_integration_trace_contract_continuation_heavy_refreshes_pli_without_reconnect() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let _ = harness.apply(
        10_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        220,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 24;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 28;
            stats.latest_video_host_present_time_ms = Some(9_520.0);
            stats.latest_video_decode_ok_time_ms = Some(9_610.0);
            stats.latest_video_packet_arrival_time_ms = Some(9_998.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 410_000,
                video_packet_count_total: 3_100,
                audio_bytes_total: 64_000,
                observed_at_ms: 9_998.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,
                        observed_at_ms: 9_998.0,
                    },
                    observed_at_ms: 9_998.0,
                });
        },
    );

    let second = harness.apply(
        10_140.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        220,
        |stats| {
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 64;
            stats.latest_video_host_present_time_ms = Some(9_520.0);
            stats.latest_video_decode_ok_time_ms = Some(9_610.0);
            stats.latest_video_packet_arrival_time_ms = Some(10_138.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 24,
                    request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                    request_kind: Some("pli".to_string()),
                    status: "sent".to_string(),
                    status_detail: None,
                    requested_at_ms: 10_000.0,
                    sent_at_ms: Some(10_010.0),
                    deadline_at_ms: Some(10_900.0),
                    transport_detail: None,
                    first_video_packet_at_ms: None,
                    first_video_packet_rtp_timestamp: None,
                    first_video_packet_is_keyframe: None,
                    first_keyframe_packet_at_ms: None,
                    first_keyframe_decoded_at_ms: None,
                    response_rtp_timestamp: None,
                    response_frame_seq: None,
                    response_verdict: Some("pending".to_string()),
                    lifecycle_phase: Some("sent".to_string()),
                    retired_at_ms: None,
                });
            stats.latest_h264_inspection_observation =
                Some(crate::XbxEngineH264InspectionObservation {
                    observation_id: 7,
                    frame_rtp_timestamp: Some(0x1020_3040),
                    nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
                    nal_count: 1,
                    vcl_nal_count: 1,
                    has_inband_sps: false,
                    has_inband_pps: false,
                    committed_sps_present: true,
                    committed_pps_present: true,
                    slice_headers_valid: true,
                    delta_continuation_ready: true,
                    parameter_sets_changed: false,
                    config_changed: false,
                    is_idr: false,
                    sample_width: None,
                    sample_height: None,
                    bootstrap_ready: false,
                    bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
                    admission_accepted: true,
                    continuation_verdict: Some("continuationAcceptedWhileAwaitingIdr".to_string()),
                    observed_at_ms: 10_138.0,
                    bound_episode_id: Some(24),
                    ..Default::default()
                });
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-await-recovery-anchor".to_string();
                timeline.chain.state = "recovering".to_string();
                timeline.chain.reason = Some("transportAwaitRecoveryAnchor".to_string());
                timeline.observed_at_ms = 10_138.0;
            }
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total = 410_000;
                track.video_packet_count_total = 3_100;
                track.observed_at_ms = 10_138.0;
            }
        },
    );

    assert!(
        second.iter().any(|command| {
            matches!(command, TransportCommand::RequestPli { reason, .. } if reason == "transportAwaitRecoveryAnchor")
                || matches!(command, TransportCommand::RequestFir { reason, .. } if reason == "transportAwaitRecoveryAnchor")
        }),
        "expected refresh pli/fir for continuation-heavy unresolved recovery, commands={second:?}"
    );
    assert!(second
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("continuation-heavy ledger");
        assert_eq!(
            ledger.input_signal,
            "transportAwaitRecoveryAnchor:transportAwaitRecoveryAnchor"
        );
        assert!(
            matches!(ledger.action_selected.as_str(), "requestPli" | "requestFir"),
            "unexpected action_selected: {}",
            ledger.action_selected
        );
        assert_eq!(ledger.state_after, "active-recovery");
    });
}

#[test]
fn recovery_integration_trace_contract_continuation_refresh_sets_episode_health() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let _ = harness.apply(
        10_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        220,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 24;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 28;
            stats.latest_video_host_present_time_ms = Some(9_520.0);
            stats.latest_video_decode_ok_time_ms = Some(9_610.0);
            stats.latest_video_packet_arrival_time_ms = Some(9_998.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 410_000,
                video_packet_count_total: 3_100,
                audio_bytes_total: 64_000,
                observed_at_ms: 9_998.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,
                        observed_at_ms: 9_998.0,
                    },
                    observed_at_ms: 9_998.0,
                });
        },
    );

    let second = harness.apply(
        10_140.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        220,
        |stats| {
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 64;
            stats.latest_video_host_present_time_ms = Some(9_520.0);
            stats.latest_video_decode_ok_time_ms = Some(9_610.0);
            stats.latest_video_packet_arrival_time_ms = Some(10_138.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 24,
                    request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                    request_kind: Some("pli".to_string()),
                    status: "sent".to_string(),
                    status_detail: None,
                    requested_at_ms: 10_000.0,
                    sent_at_ms: Some(10_010.0),
                    deadline_at_ms: Some(10_900.0),
                    transport_detail: None,
                    first_video_packet_at_ms: None,
                    first_video_packet_rtp_timestamp: None,
                    first_video_packet_is_keyframe: None,
                    first_keyframe_packet_at_ms: None,
                    first_keyframe_decoded_at_ms: None,
                    response_rtp_timestamp: None,
                    response_frame_seq: None,
                    response_verdict: Some("pending".to_string()),
                    lifecycle_phase: Some("sent".to_string()),
                    retired_at_ms: None,
                });
            stats.latest_h264_inspection_observation =
                Some(crate::XbxEngineH264InspectionObservation {
                    observation_id: 7,
                    frame_rtp_timestamp: Some(0x1020_3040),
                    nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
                    nal_count: 1,
                    vcl_nal_count: 1,
                    has_inband_sps: false,
                    has_inband_pps: false,
                    committed_sps_present: true,
                    committed_pps_present: true,
                    slice_headers_valid: true,
                    delta_continuation_ready: true,
                    parameter_sets_changed: false,
                    config_changed: false,
                    is_idr: false,
                    sample_width: None,
                    sample_height: None,
                    bootstrap_ready: false,
                    bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
                    admission_accepted: true,
                    continuation_verdict: Some("continuationAcceptedWhileAwaitingIdr".to_string()),
                    observed_at_ms: 10_138.0,
                    bound_episode_id: Some(24),
                    ..Default::default()
                });
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-await-recovery-anchor".to_string();
                timeline.chain.state = "recovering".to_string();
                timeline.chain.reason = Some("transportAwaitRecoveryAnchor".to_string());
                timeline.observed_at_ms = 10_138.0;
            }
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total = 410_000;
                track.video_packet_count_total = 3_100;
                track.observed_at_ms = 10_138.0;
            }
        },
    );

    assert!(
        second.iter().any(|command| {
            matches!(command, TransportCommand::RequestPli { reason, .. } if reason == "transportAwaitRecoveryAnchor")
                || matches!(command, TransportCommand::RequestFir { reason, .. } if reason == "transportAwaitRecoveryAnchor")
        }),
        "expected refresh pli/fir for continuation-heavy unresolved recovery, commands={second:?}"
    );
    harness.with_stats(|stats| {
        assert_eq!(
            stats.recovery_keyframe_episode_health.as_deref(),
            Some("continuation-only")
        );
    });
}

#[test]
fn recovery_integration_trace_contract_continuation_heavy_second_refresh_still_requests_pli() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let _ = harness.apply(
        10_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        220,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 25;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 24;
            stats.latest_video_host_present_time_ms = Some(9_520.0);
            stats.latest_video_decode_ok_time_ms = Some(9_610.0);
            stats.latest_video_packet_arrival_time_ms = Some(9_998.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 420_000,
                video_packet_count_total: 3_120,
                audio_bytes_total: 64_000,
                observed_at_ms: 9_998.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,
                        observed_at_ms: 9_998.0,
                    },
                    observed_at_ms: 9_998.0,
                });
        },
    );

    let _ = harness.apply(
        10_140.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        220,
        |stats| {
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 64;
            stats.latest_video_host_present_time_ms = Some(9_520.0);
            stats.latest_video_decode_ok_time_ms = Some(9_610.0);
            stats.latest_video_packet_arrival_time_ms = Some(10_138.0);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 25,
                    request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                    request_kind: Some("pli".to_string()),
                    status: "sent".to_string(),
                    status_detail: None,
                    requested_at_ms: 10_000.0,
                    sent_at_ms: Some(10_010.0),
                    deadline_at_ms: Some(10_900.0),
                    transport_detail: None,
                    first_video_packet_at_ms: None,
                    first_video_packet_rtp_timestamp: None,
                    first_video_packet_is_keyframe: None,
                    first_keyframe_packet_at_ms: None,
                    first_keyframe_decoded_at_ms: None,
                    response_rtp_timestamp: None,
                    response_frame_seq: None,
                    response_verdict: Some("pending".to_string()),
                    lifecycle_phase: Some("sent".to_string()),
                    retired_at_ms: None,
                });
            stats.latest_h264_inspection_observation =
                Some(crate::XbxEngineH264InspectionObservation {
                    observation_id: 9,
                    frame_rtp_timestamp: Some(0x2020_3040),
                    nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
                    nal_count: 1,
                    vcl_nal_count: 1,
                    has_inband_sps: false,
                    has_inband_pps: false,
                    committed_sps_present: true,
                    committed_pps_present: true,
                    slice_headers_valid: true,
                    delta_continuation_ready: true,
                    parameter_sets_changed: false,
                    config_changed: false,
                    is_idr: false,
                    sample_width: None,
                    sample_height: None,
                    bootstrap_ready: false,
                    bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
                    admission_accepted: true,
                    continuation_verdict: Some("continuationAcceptedWhileAwaitingIdr".to_string()),
                    observed_at_ms: 10_138.0,
                    bound_episode_id: Some(25),
                    ..Default::default()
                });
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observed_at_ms = 10_138.0;
            }
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.observed_at_ms = 10_138.0;
            }
        },
    );

    let third = harness.apply(
        10_300.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        220,
        |stats| {
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 92;
            stats.latest_video_host_present_time_ms = Some(9_520.0);
            stats.latest_video_decode_ok_time_ms = Some(9_610.0);
            stats.latest_video_packet_arrival_time_ms = Some(10_298.0);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 26,
                    request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                    request_kind: Some("pli".to_string()),
                    status: "sent".to_string(),
                    status_detail: None,
                    requested_at_ms: 10_140.0,
                    sent_at_ms: Some(10_150.0),
                    deadline_at_ms: Some(11_040.0),
                    transport_detail: None,
                    first_video_packet_at_ms: None,
                    first_video_packet_rtp_timestamp: None,
                    first_video_packet_is_keyframe: None,
                    first_keyframe_packet_at_ms: None,
                    first_keyframe_decoded_at_ms: None,
                    response_rtp_timestamp: None,
                    response_frame_seq: None,
                    response_verdict: Some("pending".to_string()),
                    lifecycle_phase: Some("sent".to_string()),
                    retired_at_ms: None,
                });
            stats.latest_h264_inspection_observation =
                Some(crate::XbxEngineH264InspectionObservation {
                    observation_id: 10,
                    frame_rtp_timestamp: Some(0x2020_3050),
                    nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
                    nal_count: 1,
                    vcl_nal_count: 1,
                    has_inband_sps: false,
                    has_inband_pps: false,
                    committed_sps_present: true,
                    committed_pps_present: true,
                    slice_headers_valid: true,
                    delta_continuation_ready: true,
                    parameter_sets_changed: false,
                    config_changed: false,
                    is_idr: false,
                    sample_width: None,
                    sample_height: None,
                    bootstrap_ready: false,
                    bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
                    admission_accepted: true,
                    continuation_verdict: Some("continuationAcceptedWhileAwaitingIdr".to_string()),
                    observed_at_ms: 10_298.0,
                    bound_episode_id: Some(26),
                    ..Default::default()
                });
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observed_at_ms = 10_298.0;
            }
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.observed_at_ms = 10_298.0;
            }
        },
    );

    assert!(third.iter().any(|command| {
        matches!(command, TransportCommand::RequestPli { reason, .. } if reason == "transportAwaitRecoveryAnchor")
            || matches!(command, TransportCommand::RequestFir { reason, .. } if reason == "transportAwaitRecoveryAnchor")
    }));
    assert!(third
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
}

#[test]
fn recovery_integration_local_supply_suspect_dwell_clears_on_no_signal_gap() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let first = harness.apply(
        1_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoverySuspect",
        180,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 31;
            stats.transport_recovery_episode_opened_at_ms = Some(900.0);
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(995.0);
            stats.latest_video_decode_ok_time_ms = Some(996.0);
            stats.latest_video_packet_arrival_time_ms = Some(998.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
            stats.video_decoder_recovery_state_changed_at_ms = Some(999.0);
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-complete-candidate".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,
                        observed_at_ms: 999.0,
                    },
                    observed_at_ms: 999.0,
                });
        },
    );
    assert!(
        first.is_empty(),
        "suspect dwell should not emit keyframe immediately: {first:?}"
    );
    harness.with_stats(|stats| {
        assert_eq!(
            stats.recovery_active_escalation_reason.as_deref(),
            Some("localSupplySuspect")
        );
    });

    let middle = harness.apply(
        1_120.0,
        ConnectionLifecycleStateFact::Connected,
        "displaySupplyCritical",
        180,
        |stats| {
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 72;
            stats.latest_video_host_present_time_ms = Some(1_115.0);
            stats.latest_video_decode_ok_time_ms = Some(1_116.0);
            stats.latest_video_packet_arrival_time_ms = Some(1_118.0);
            stats.video_decoder_recovery_state = Some("steady".to_string());
            stats.video_decoder_recovery_state_changed_at_ms = Some(1_118.0);
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-complete-candidate".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.observed_at_ms = 1_118.0;
            }
        },
    );
    assert!(
        middle.is_empty(),
        "display absorption should stay local: {middle:?}"
    );
    assert_eq!(harness.policy.local_supply_suspect_since_ms, None);

    let third = harness.apply(
        1_330.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoverySuspect",
        180,
        |stats| {
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(1_325.0);
            stats.latest_video_decode_ok_time_ms = Some(1_326.0);
            stats.latest_video_packet_arrival_time_ms = Some(1_328.0);
            stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
            stats.video_decoder_recovery_state_changed_at_ms = Some(1_329.0);
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-complete-candidate".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.observed_at_ms = 1_329.0;
            }
        },
    );
    assert!(
        third.is_empty(),
        "suspect re-entry should restart dwell without immediate keyframe request: {third:?}"
    );
    assert_eq!(harness.policy.local_supply_suspect_since_ms, Some(1_330.0));
    harness.with_stats(|stats| {
        assert_eq!(
            stats.recovery_active_escalation_reason.as_deref(),
            Some("localSupplySuspect")
        );
        assert_eq!(
            stats.recovery_owner_surface_state.as_deref(),
            Some("suspect")
        );
    });
}

#[test]
fn recovery_integration_trace_contract_continuation_heavy_stops_only_after_clean_anchor() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let _ = harness.apply(
        10_000.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        220,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 26;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 24;
            stats.latest_video_host_present_time_ms = Some(9_520.0);
            stats.latest_video_decode_ok_time_ms = Some(9_610.0);
            stats.latest_video_packet_arrival_time_ms = Some(9_998.0);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 430_000,
                video_packet_count_total: 3_150,
                audio_bytes_total: 64_000,
                observed_at_ms: 9_998.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,
                        observed_at_ms: 9_998.0,
                    },
                    observed_at_ms: 9_998.0,
                });
        },
    );

    let settled = harness.apply(
        10_180.0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        221,
        |stats| {
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(10_176.0);
            stats.latest_video_decode_ok_time_ms = Some(10_174.0);
            stats.latest_video_packet_arrival_time_ms = Some(10_178.0);
            stats.video_anchor_clean_epoch = Some(26);
            stats.video_anchor_clean_observed_at_ms = Some(10_172.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 27,
                    request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                    request_kind: Some("pli".to_string()),
                    status: "decoded".to_string(),
                    status_detail: None,
                    requested_at_ms: 10_000.0,
                    sent_at_ms: Some(10_010.0),
                    deadline_at_ms: Some(10_900.0),
                    transport_detail: None,
                    first_video_packet_at_ms: Some(10_160.0),
                    first_video_packet_rtp_timestamp: Some(0x3030_4000),
                    first_video_packet_is_keyframe: Some(true),
                    first_keyframe_packet_at_ms: Some(10_160.0),
                    first_keyframe_decoded_at_ms: Some(10_170.0),
                    response_rtp_timestamp: Some(0x3030_4000),
                    response_frame_seq: Some(221),
                    response_verdict: Some("cleanAnchorCommitted".to_string()),
                    lifecycle_phase: Some("decoded".to_string()),
                    retired_at_ms: None,
                });
            stats.latest_h264_inspection_observation =
                Some(crate::XbxEngineH264InspectionObservation {
                    observation_id: 11,
                    frame_rtp_timestamp: Some(0x3030_4000),
                    nal_types: vec!["SliceLayerWithoutPartitioningIdr".to_string()],
                    nal_count: 1,
                    vcl_nal_count: 1,
                    has_inband_sps: true,
                    has_inband_pps: true,
                    committed_sps_present: true,
                    committed_pps_present: true,
                    slice_headers_valid: true,
                    delta_continuation_ready: false,
                    parameter_sets_changed: true,
                    config_changed: true,
                    is_idr: true,
                    sample_width: Some(1920),
                    sample_height: Some(1080),
                    bootstrap_ready: true,
                    bootstrap_reject_reason: None,
                    admission_accepted: true,
                    continuation_verdict: None,
                    observed_at_ms: 10_170.0,
                    ..Default::default()
                });
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.observed_at_ms = 10_176.0;
            }
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total = 470_000;
                track.video_packet_count_total = 3_260;
                track.observed_at_ms = 10_178.0;
            }
        },
    );

    assert!(
        settled.is_empty(),
        "expected no command after clean anchor, commands={settled:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .recent_recovery_decision_ledgers
            .last()
            .expect("settled ledger");
        assert_ne!(ledger.state_after, "active-recovery");
        assert_ne!(
            stats.video_owner_state.as_deref(),
            Some("rebuilding-supply")
        );
    });
}

#[test]
fn recovery_integration_transport_await_continuation_only_sustained_upgrades_to_fir() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let commands = harness.apply(
        12_180.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        220,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 31;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 72;
            stats.latest_video_host_present_time_ms = Some(11_520.0);
            stats.latest_video_decode_ok_time_ms = Some(11_610.0);
            stats.latest_video_packet_arrival_time_ms = Some(12_178.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 480_000,
                video_packet_count_total: 3_400,
                audio_bytes_total: 64_000,
                observed_at_ms: 12_178.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 1,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,
                        observed_at_ms: 12_178.0,
                    },
                    observed_at_ms: 12_178.0,
                });
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 31,
                    request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                    request_kind: Some("pli".to_string()),
                    status: "sent".to_string(),
                    status_detail: None,
                    requested_at_ms: 12_000.0,
                    sent_at_ms: Some(12_010.0),
                    deadline_at_ms: Some(12_900.0),
                    transport_detail: None,
                    first_video_packet_at_ms: Some(12_080.0),
                    first_video_packet_rtp_timestamp: Some(0x1020_3040),
                    first_video_packet_is_keyframe: Some(false),
                    first_keyframe_packet_at_ms: None,
                    first_keyframe_decoded_at_ms: None,
                    response_rtp_timestamp: Some(0x1020_3040),
                    response_frame_seq: Some(41),
                    response_verdict: Some("pending".to_string()),
                    lifecycle_phase: Some("packetSeen".to_string()),
                    retired_at_ms: None,
                });
            stats.latest_h264_inspection_observation =
                Some(crate::XbxEngineH264InspectionObservation {
                    observation_id: 7,
                    frame_rtp_timestamp: Some(0x1020_3040),
                    nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
                    nal_count: 1,
                    vcl_nal_count: 1,
                    has_inband_sps: false,
                    has_inband_pps: false,
                    committed_sps_present: true,
                    committed_pps_present: true,
                    slice_headers_valid: true,
                    delta_continuation_ready: true,
                    parameter_sets_changed: false,
                    config_changed: false,
                    is_idr: false,
                    sample_width: None,
                    sample_height: None,
                    bootstrap_ready: false,
                    bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
                    admission_accepted: true,
                    continuation_verdict: Some("continuationAcceptedWhileAwaitingIdr".to_string()),
                    observed_at_ms: 12_170.0,
                    bound_episode_id: Some(31),
                    ..Default::default()
                });
        },
    );

    assert!(
        commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestFir { reason, .. } if reason == "transportAwaitRecoveryAnchor")
        }),
        "commands={commands:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("continuation-only fir ledger");
        assert_eq!(ledger.action_selected, "requestFir");
        assert_eq!(
            ledger.unlock_reason.as_deref(),
            Some("continuationOnlyAwaitingIdr")
        );
    });
}

#[test]
fn recovery_integration_transport_await_continuation_only_uses_progressed_transport_episode() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let commands = harness.apply(
        12_180.0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        220,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 31;
            stats.transport_recovery_episode_opened_at_ms = Some(11_900.0);
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 72;
            stats.latest_video_host_present_time_ms = Some(11_520.0);
            stats.latest_video_decode_ok_time_ms = Some(11_610.0);
            stats.latest_video_packet_arrival_time_ms = Some(12_178.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 640_000,
                video_packet_count_total: 6_100,
                audio_bytes_total: 120_000,
                observed_at_ms: 12_178.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 12,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,
                        observed_at_ms: 12_178.0,
                    },
                    observed_at_ms: 12_178.0,
                });
            stats.recent_keyframe_request_episodes.push(
                crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 31,
                    request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                    request_kind: Some("pli".to_string()),
                    status: "response-observed".to_string(),
                    status_detail: Some("continuationAcceptedWhileAwaitingIdr".to_string()),
                    requested_at_ms: 12_000.0,
                    sent_at_ms: Some(12_010.0),
                    deadline_at_ms: Some(12_900.0),
                    transport_detail: None,
                    first_video_packet_at_ms: Some(12_080.0),
                    first_video_packet_rtp_timestamp: Some(0x2233_4401),
                    first_video_packet_is_keyframe: Some(false),
                    first_keyframe_packet_at_ms: None,
                    first_keyframe_decoded_at_ms: None,
                    response_rtp_timestamp: Some(0x2233_4401),
                    response_frame_seq: Some(41),
                    response_verdict: Some("pending".to_string()),
                    lifecycle_phase: Some("packetSeen".to_string()),
                    retired_at_ms: None,
                },
            );
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 90,
                    request_reason: Some("displaySupplyCritical".to_string()),
                    request_kind: Some("pli".to_string()),
                    status: "sent".to_string(),
                    status_detail: None,
                    requested_at_ms: 12_150.0,
                    sent_at_ms: Some(12_151.0),
                    deadline_at_ms: Some(13_000.0),
                    transport_detail: None,
                    first_video_packet_at_ms: None,
                    first_video_packet_rtp_timestamp: None,
                    first_video_packet_is_keyframe: None,
                    first_keyframe_packet_at_ms: None,
                    first_keyframe_decoded_at_ms: None,
                    response_rtp_timestamp: None,
                    response_frame_seq: None,
                    response_verdict: Some("pending".to_string()),
                    lifecycle_phase: Some("sent".to_string()),
                    retired_at_ms: None,
                });
            stats.latest_h264_inspection_observation =
                Some(crate::XbxEngineH264InspectionObservation {
                    observation_id: 7,
                    frame_rtp_timestamp: Some(0x2233_4401),
                    nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".to_string()],
                    nal_count: 1,
                    vcl_nal_count: 1,
                    has_inband_sps: false,
                    has_inband_pps: false,
                    committed_sps_present: true,
                    committed_pps_present: true,
                    slice_headers_valid: true,
                    delta_continuation_ready: true,
                    parameter_sets_changed: false,
                    config_changed: false,
                    is_idr: false,
                    sample_width: None,
                    sample_height: None,
                    bootstrap_ready: false,
                    bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
                    admission_accepted: true,
                    continuation_verdict: Some("continuationAcceptedWhileAwaitingIdr".to_string()),
                    observed_at_ms: 12_170.0,
                    bound_episode_id: Some(31),
                    ..Default::default()
                });
        },
    );

    assert!(
        commands.iter().any(|command| {
            matches!(command, TransportCommand::RequestFir { reason, .. } if reason == "transportAwaitRecoveryAnchor")
        }),
        "commands={commands:?}"
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("progressed transport fir ledger");
        assert_eq!(ledger.action_selected, "requestFir");
        assert_eq!(
            ledger.unlock_reason.as_deref(),
            Some("continuationOnlyAwaitingIdr")
        );
    });
}

#[test]
fn recovery_integration_transport_await_decoder_no_output_after_continuation_upgrades_to_fir() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let now_ms = 12_260.0;
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_phase = Some("steady".to_string());
        stats.transport_recovery_epoch = 44;
        stats.transport_recovery_episode_opened_at_ms = Some(12_000.0);
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 72;
        stats.latest_video_host_present_time_ms = Some(11_520.0);
        stats.latest_video_decode_ok_time_ms = Some(11_610.0);
        stats.latest_video_packet_arrival_time_ms = Some(12_255.0);
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
        stats.video_anchor_clean_epoch = None;
        stats.video_anchor_clean_observed_at_ms = None;
        stats.video_anchor_clean_source_event = None;
        stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
        stats.video_decoder_recovery_event = Some("backend-failure-escalated".to_string());
        stats.video_decoder_recovery_detail =
            Some("nominalContinuationNoOutputSoftFallback".to_string());
        stats.video_decoder_recovery_state_changed_at_ms = Some(12_210.0);
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 44,
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                request_kind: Some("pli".to_string()),
                status: "response-observed".to_string(),
                status_detail: Some("continuationAcceptedWhileAwaitingIdr".to_string()),
                requested_at_ms: 12_000.0,
                sent_at_ms: Some(12_010.0),
                deadline_at_ms: Some(12_900.0),
                transport_detail: None,
                first_video_packet_at_ms: Some(12_070.0),
                first_video_packet_rtp_timestamp: Some(0x3344_5501),
                first_video_packet_is_keyframe: Some(false),
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: Some(0x3344_5501),
                response_frame_seq: Some(52),
                response_verdict: Some("pending".to_string()),
                lifecycle_phase: Some("packetSeen".to_string()),
                retired_at_ms: None,
            });
        stats.latest_h264_inspection_observation = None;
    }
    let policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
    let owner_signal = crate::transport::rtc::recovery::coordinator::RecoveryOwnerSignal {
        reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
        reason_label: "transportAwaitRecoveryAnchor".to_string(),
        observed_at_ms: now_ms,
        gap_severity: None,
        repairability: None,
    };
    let proposal = crate::transport::rtc::recovery::coordinator::CoordinatorProposal {
        decision: crate::transport::rtc::recovery::escalation::VideoEscalationDecision {
            observation_id: 3,
            action: RecoveryAction::RequestPli,
        },
        coalescing_mode: Some(crate::transport::rtc::recovery::contract::CoalescingMode::Refresh),
        unlock_reason: Some("continuationOnlyRefreshIntervalElapsed".to_string()),
        preempt_reason: None,
        budget_before: crate::transport::rtc::recovery::escalation::RecoveryActionBudgetState {
            recovery_epoch: 44,
            keyframe_budget_used: 1,
            keyframe_budget_limit: 3,
            decoder_reset_budget_used: 0,
            decoder_reset_budget_limit: 2,
            reconnect_budget_used: 0,
            reconnect_budget_limit: 2,
        },
        budget_after: crate::transport::rtc::recovery::escalation::RecoveryActionBudgetState {
            recovery_epoch: 44,
            keyframe_budget_used: 2,
            keyframe_budget_limit: 3,
            decoder_reset_budget_used: 0,
            decoder_reset_budget_limit: 2,
            reconnect_budget_used: 0,
            reconnect_budget_limit: 2,
        },
    };

    assert_eq!(
        policy.should_upgrade_transport_await_refresh_to_fir(
            crate::transport::rtc::policy::video_scheduling_owner::VideoSchedulingOwnerState::RebuildingSupply,
            &proposal,
            &owner_signal,
            now_ms,
        ),
        Some("decoderNoOutputAfterContinuation")
    );
}

#[test]
fn recovery_integration_transport_await_gap_repair_stalled_upgrades_to_fir() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let owner_signal = crate::transport::rtc::recovery::coordinator::RecoveryOwnerSignal {
        reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
        reason_label: "transportAwaitRecoveryAnchor".to_string(),
        observed_at_ms: 12_180.0,
        gap_severity: None,
        repairability: None,
    };
    let proposal = crate::transport::rtc::recovery::coordinator::CoordinatorProposal {
        decision: crate::transport::rtc::recovery::escalation::VideoEscalationDecision {
            observation_id: 402,
            action: RecoveryAction::CoalescedKeyframeInFlight,
        },
        coalescing_mode: Some(crate::transport::rtc::recovery::contract::CoalescingMode::Merge),
        unlock_reason: None,
        preempt_reason: None,
        budget_before: crate::transport::rtc::recovery::escalation::RecoveryActionBudgetState {
            recovery_epoch: 45,
            keyframe_budget_used: 1,
            keyframe_budget_limit: 2,
            decoder_reset_budget_used: 0,
            decoder_reset_budget_limit: 2,
            reconnect_budget_used: 0,
            reconnect_budget_limit: 1,
        },
        budget_after: crate::transport::rtc::recovery::escalation::RecoveryActionBudgetState {
            recovery_epoch: 45,
            keyframe_budget_used: 1,
            keyframe_budget_limit: 2,
            decoder_reset_budget_used: 0,
            decoder_reset_budget_limit: 2,
            reconnect_budget_used: 0,
            reconnect_budget_limit: 1,
        },
    };
    RuntimeStatsSink::update_shared(runtime_stats.as_ref(), |stats| {
        stats.session_phase = Some("steady".to_string());
        stats.transport_recovery_epoch = 45;
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.video_anchor_clean_epoch = None;
        stats.video_anchor_clean_observed_at_ms = None;
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("transportAwaitRecoveryAnchor".to_string()),
                chain_break_evidence: None,
                observed_at_ms: 12_178.0,
            },
            observed_at_ms: 12_178.0,
        });
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 45,
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                request_kind: Some("pli".to_string()),
                status: "packet-seen".to_string(),
                status_detail: None,
                requested_at_ms: 12_000.0,
                sent_at_ms: Some(12_010.0),
                deadline_at_ms: Some(12_900.0),
                transport_detail: None,
                first_video_packet_at_ms: Some(12_070.0),
                first_video_packet_rtp_timestamp: Some(0x5566_7788),
                first_video_packet_is_keyframe: Some(true),
                first_keyframe_packet_at_ms: Some(12_070.0),
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: Some(0x5566_7788),
                response_frame_seq: Some(45),
                response_verdict: Some("pending".to_string()),
                lifecycle_phase: Some("packetSeen".to_string()),
                retired_at_ms: None,
            });
        stats.latest_anchor_candidate_ledger = Some(crate::XbxEngineAnchorCandidateLedger {
            recovery_epoch: 45,
            frame_rtp_timestamp: None,
            state: crate::XbxEngineAnchorCandidateState::AwaitingRecovery,
            source_event: "gap-repair-in-flight".to_string(),
            failure_reason: Some(crate::XbxEngineAnchorCandidateFailureReason::LocalRepairPending),
            observed_at_ms: 12_150.0,
        });
    });

    assert_eq!(
        policy.should_upgrade_transport_await_refresh_to_fir(
            VideoSchedulingOwnerState::RebuildingSupply,
            &proposal,
            &owner_signal,
            12_180.0,
        ),
        Some("awaitingRecoveryAnchor")
    );
}

#[test]
fn recovery_integration_transport_await_recovering_no_output_after_continuation_upgrades_to_fir() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let now_ms = 12_260.0;
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_phase = Some("steady".to_string());
        stats.transport_recovery_epoch = 44;
        stats.transport_recovery_episode_opened_at_ms = Some(12_000.0);
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 72;
        stats.latest_video_host_present_time_ms = Some(11_520.0);
        stats.latest_video_decode_ok_time_ms = Some(11_610.0);
        stats.latest_video_packet_arrival_time_ms = Some(12_255.0);
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
        stats.video_anchor_clean_epoch = None;
        stats.video_anchor_clean_observed_at_ms = None;
        stats.video_anchor_clean_source_event = None;
        stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
        stats.video_decoder_recovery_event = Some("backend-failure-escalated".to_string());
        stats.video_decoder_recovery_detail =
            Some("recoveringContinuationNoOutputSoftFallback".to_string());
        stats.video_decoder_recovery_state_changed_at_ms = Some(12_210.0);
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 44,
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                request_kind: Some("pli".to_string()),
                status: "response-observed".to_string(),
                status_detail: Some("continuationAcceptedWhileAwaitingIdr".to_string()),
                requested_at_ms: 12_000.0,
                sent_at_ms: Some(12_010.0),
                deadline_at_ms: Some(12_900.0),
                transport_detail: None,
                first_video_packet_at_ms: Some(12_070.0),
                first_video_packet_rtp_timestamp: Some(0x3344_5501),
                first_video_packet_is_keyframe: Some(false),
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: Some(0x3344_5501),
                response_frame_seq: Some(52),
                response_verdict: Some("pending".to_string()),
                lifecycle_phase: Some("packetSeen".to_string()),
                retired_at_ms: None,
            });
        stats.latest_h264_inspection_observation = None;
    }
    let policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
    let owner_signal = crate::transport::rtc::recovery::coordinator::RecoveryOwnerSignal {
        reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
        reason_label: "transportAwaitRecoveryAnchor".to_string(),
        observed_at_ms: now_ms,
        gap_severity: None,
        repairability: None,
    };
    let proposal = crate::transport::rtc::recovery::coordinator::CoordinatorProposal {
        decision: crate::transport::rtc::recovery::escalation::VideoEscalationDecision {
            observation_id: 3,
            action: RecoveryAction::RequestPli,
        },
        coalescing_mode: Some(crate::transport::rtc::recovery::contract::CoalescingMode::Refresh),
        unlock_reason: Some("continuationOnlyRefreshIntervalElapsed".to_string()),
        preempt_reason: None,
        budget_before: crate::transport::rtc::recovery::escalation::RecoveryActionBudgetState {
            recovery_epoch: 44,
            keyframe_budget_used: 1,
            keyframe_budget_limit: 3,
            decoder_reset_budget_used: 0,
            decoder_reset_budget_limit: 2,
            reconnect_budget_used: 0,
            reconnect_budget_limit: 2,
        },
        budget_after: crate::transport::rtc::recovery::escalation::RecoveryActionBudgetState {
            recovery_epoch: 44,
            keyframe_budget_used: 2,
            keyframe_budget_limit: 3,
            decoder_reset_budget_used: 0,
            decoder_reset_budget_limit: 2,
            reconnect_budget_used: 0,
            reconnect_budget_limit: 2,
        },
    };

    assert_eq!(
        policy.should_upgrade_transport_await_refresh_to_fir(
            crate::transport::rtc::policy::video_scheduling_owner::VideoSchedulingOwnerState::RebuildingSupply,
            &proposal,
            &owner_signal,
            now_ms,
        ),
        Some("decoderNoOutputAfterContinuation")
    );
}

#[test]
fn cloud_high_rtt_repeated_transport_severe_deadline_stays_local_without_connectivity_evidence() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.session_phase = Some("steady".to_string());
        stats.transport_recovery_epoch = 41;
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 360;
        stats.latest_video_host_present_time_ms = Some(7_000.0);
        stats.latest_video_decode_ok_time_ms = Some(7_400.0);
        stats.latest_video_packet_arrival_time_ms = Some(7_520.0);
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(true);
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 512_000,
            video_packet_count_total: 4_200,
            audio_bytes_total: 96_000,
            observed_at_ms: 7_540.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("transportAwaitRecoveryAnchor".to_string()),
                chain_break_evidence: None,

                observed_at_ms: 7_540.0,
            },
            observed_at_ms: 7_540.0,
        });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(240.0);
    connection.latest_loss_ratio_1s = Some(0.06);
    connection.last_observed_at_ms = Some(8_000.0);

    let first = TransportSnapshot::new(
        1,
        8_000.0,
        connection.clone(),
        MediaProjection {
            frame_count: 220,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("transportSevereDeadline".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(8_000.0),
            ..Default::default()
        },
        BweProjection {
            latest_rtt_ms: Some(240.0),
            latest_loss_ratio_1s: Some(0.06),
            latest_actual_video_bitrate_kbps: Some(6_000.0),
            latest_observed_remb_kbps: Some(8_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(8_000.0),
            target_remb_kbps: Some(8_000),
            last_observed_at_ms: Some(8_000.0),
        },
        DiagnosticsProjection::default(),
    );
    let first_commands = transport_commands(policy.on_snapshot(&first));
    assert!(first_commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));

    let second = TransportSnapshot::new(
        2,
        8_040.0,
        connection,
        MediaProjection {
            frame_count: 220,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("transportSevereDeadline".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(8_040.0),
            ..Default::default()
        },
        BweProjection {
            latest_rtt_ms: Some(240.0),
            latest_loss_ratio_1s: Some(0.06),
            latest_actual_video_bitrate_kbps: Some(5_600.0),
            latest_observed_remb_kbps: Some(7_000),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(8_040.0),
            target_remb_kbps: Some(7_000),
            last_observed_at_ms: Some(8_040.0),
        },
        DiagnosticsProjection::default(),
    );
    let second_commands = transport_commands(policy.on_snapshot(&second));
    assert!(second_commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(
        ledger.gate_result,
        "suppressed:reconnectBlocked:transportGate:awaitingRecoveryChain"
    );
    assert_eq!(ledger.action_selected, "cooldownSuppressed");
}

#[test]
fn media_reconnect_candidate_is_blocked_while_transport_await_reset_progress_is_active() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let now_ms = 9_000.0;
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_recovery_epoch = 52;
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 260;
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("awaitingRecoveryAnchor".to_string()),
                chain_break_evidence: None,

                observed_at_ms: now_ms,
            },
            observed_at_ms: now_ms,
        });
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 2,
                reason: "transportAwaitRecoveryAnchor".to_string(),
                action: "requestDecoderReset".to_string(),
                recovery_stage: "rebuilding-supply".to_string(),
                recovery_chain_value: "anchor".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "transport-await-window".to_string(),
                observed_at_ms: now_ms - 80.0,
            });
        stats.latest_video_decoder_reset_time_ms = Some(now_ms - 60.0);
    }
    let policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    let owner_signal = crate::transport::rtc::recovery::coordinator::RecoveryOwnerSignal {
        reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
        reason_label: "transportAwaitRecoveryAnchor".to_string(),
        observed_at_ms: now_ms,
        gap_severity: None,
        repairability: None,
    };
    let proposal = crate::transport::rtc::recovery::coordinator::CoordinatorProposal {
        decision: crate::transport::rtc::recovery::escalation::VideoEscalationDecision {
            observation_id: 3,
            action: RecoveryAction::RequestReconnectCandidate,
        },
        coalescing_mode: None,
        unlock_reason: None,
        preempt_reason: None,
        budget_before: crate::transport::rtc::recovery::escalation::RecoveryActionBudgetState {
            recovery_epoch: 52,
            keyframe_budget_used: 1,
            keyframe_budget_limit: 2,
            decoder_reset_budget_used: 1,
            decoder_reset_budget_limit: 2,
            reconnect_budget_used: 0,
            reconnect_budget_limit: 2,
        },
        budget_after: crate::transport::rtc::recovery::escalation::RecoveryActionBudgetState {
            recovery_epoch: 52,
            keyframe_budget_used: 1,
            keyframe_budget_limit: 2,
            decoder_reset_budget_used: 1,
            decoder_reset_budget_limit: 2,
            reconnect_budget_used: 1,
            reconnect_budget_limit: 2,
        },
    };
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    let snapshot = TransportSnapshot::new(
        1,
        now_ms,
        connection,
        MediaProjection {
            frame_count: 200,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("transportAwaitRecoveryAnchor".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(now_ms),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    assert_eq!(
        policy.media_reconnect_block_reason(
        &snapshot,
        crate::transport::rtc::policy::video_scheduling_owner::VideoSchedulingOwnerState::RebuildingSupply,
        &proposal,
        &owner_signal,
        now_ms,
    ),
        Some("mediaGate:missingHardEvidence")
    );
}

#[test]
fn media_reconnect_candidate_is_blocked_while_control_replay_backlog_is_active() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let now_ms = 9_500.0;
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_recovery_epoch = 12;
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.control_pending_replay_action_count = 1;
        stats.control_pending_replay_since_ms = Some(now_ms - 200.0);
        stats.control_pending_replay_summary =
            Some("keyframe=true decoderReset=false ready=false".to_string());
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("awaitingRecoveryAnchor".to_string()),
                chain_break_evidence: None,

                observed_at_ms: now_ms,
            },
            observed_at_ms: now_ms,
        });
    }
    let policy = RtcSessionPolicy::new(runtime_config, runtime_stats);
    let owner_signal = crate::transport::rtc::recovery::coordinator::RecoveryOwnerSignal {
        reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
        reason_label: "transportAwaitRecoveryAnchor".to_string(),
        observed_at_ms: now_ms,
        gap_severity: None,
        repairability: None,
    };
    let proposal = crate::transport::rtc::recovery::coordinator::CoordinatorProposal {
        decision: crate::transport::rtc::recovery::escalation::VideoEscalationDecision {
            observation_id: 3,
            action: RecoveryAction::RequestReconnectCandidate,
        },
        coalescing_mode: None,
        unlock_reason: None,
        preempt_reason: None,
        budget_before: crate::transport::rtc::recovery::escalation::RecoveryActionBudgetState {
            recovery_epoch: 12,
            keyframe_budget_used: 1,
            keyframe_budget_limit: 2,
            decoder_reset_budget_used: 1,
            decoder_reset_budget_limit: 2,
            reconnect_budget_used: 0,
            reconnect_budget_limit: 2,
        },
        budget_after: crate::transport::rtc::recovery::escalation::RecoveryActionBudgetState {
            recovery_epoch: 12,
            keyframe_budget_used: 1,
            keyframe_budget_limit: 2,
            decoder_reset_budget_used: 1,
            decoder_reset_budget_limit: 2,
            reconnect_budget_used: 1,
            reconnect_budget_limit: 2,
        },
    };
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    let snapshot = TransportSnapshot::new(
        1,
        now_ms,
        connection,
        MediaProjection {
            frame_count: 120,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("transportAwaitRecoveryAnchor".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(now_ms),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    assert_eq!(
        policy.media_reconnect_block_reason(
            &snapshot,
            crate::transport::rtc::policy::video_scheduling_owner::VideoSchedulingOwnerState::RebuildingSupply,
            &proposal,
            &owner_signal,
            now_ms,
        ),
        Some("mediaGate:controlReplayBacklog")
    );
}

#[test]
fn media_reconnect_candidate_waits_for_success_edge_before_regrant() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let now_ms = 10_000.0;
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.transport_recovery_epoch = 18;
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 800.0);
        stats.latest_video_host_present_time_ms = Some(now_ms - 820.0);
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("awaitingRecoveryAnchor".to_string()),
                chain_break_evidence: None,

                observed_at_ms: now_ms,
            },
            observed_at_ms: now_ms,
        });
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 7,
                request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                request_kind: Some("control".to_string()),
                status: "missed".to_string(),
                status_detail: None,
                requested_at_ms: now_ms - 1_200.0,
                sent_at_ms: Some(now_ms - 1_180.0),
                deadline_at_ms: Some(now_ms - 300.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: None,
                response_frame_seq: None,
                response_verdict: Some("missed".to_string()),
                lifecycle_phase: None,
                retired_at_ms: None,
            });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());
    policy.last_successful_media_edge_at_ms = Some(now_ms - 800.0);
    policy.reconnect_success_edge_at_last_grant = Some(now_ms - 800.0);
    policy.reconnect_grants_without_success_edge = 1;

    let owner_signal = crate::transport::rtc::recovery::coordinator::RecoveryOwnerSignal {
        reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
        reason_label: "transportAwaitRecoveryAnchor".to_string(),
        observed_at_ms: now_ms,
        gap_severity: None,
        repairability: None,
    };

    let proposal = crate::transport::rtc::recovery::coordinator::CoordinatorProposal {
        decision: crate::transport::rtc::recovery::escalation::VideoEscalationDecision {
            observation_id: 3,
            action: RecoveryAction::RequestReconnectCandidate,
        },
        coalescing_mode: None,
        unlock_reason: None,
        preempt_reason: None,
        budget_before: crate::transport::rtc::recovery::escalation::RecoveryActionBudgetState {
            recovery_epoch: 18,
            keyframe_budget_used: 1,
            keyframe_budget_limit: 2,
            decoder_reset_budget_used: 1,
            decoder_reset_budget_limit: 2,
            reconnect_budget_used: 1,
            reconnect_budget_limit: 3,
        },
        budget_after: crate::transport::rtc::recovery::escalation::RecoveryActionBudgetState {
            recovery_epoch: 18,
            keyframe_budget_used: 1,
            keyframe_budget_limit: 2,
            decoder_reset_budget_used: 1,
            decoder_reset_budget_limit: 2,
            reconnect_budget_used: 2,
            reconnect_budget_limit: 3,
        },
    };
    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    let snapshot = TransportSnapshot::new(
        1,
        now_ms,
        connection,
        MediaProjection {
            frame_count: 120,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("transportAwaitRecoveryAnchor".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(now_ms),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );

    assert_eq!(
        policy.media_reconnect_block_reason(
            &snapshot,
            crate::transport::rtc::policy::video_scheduling_owner::VideoSchedulingOwnerState::RebuildingSupply,
            &proposal,
            &owner_signal,
            now_ms,
        ),
        Some("mediaGate:awaitSuccessEdge")
    );
}

#[test]
fn cloud_high_rtt_repeated_transport_expired_deadline_stays_local_without_connectivity_evidence() {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.session_phase = Some("steady".to_string());
        stats.transport_recovery_epoch = 42;
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 380;
        stats.latest_video_host_present_time_ms = Some(9_000.0);
        stats.latest_video_decode_ok_time_ms = Some(9_350.0);
        stats.latest_video_packet_arrival_time_ms = Some(9_480.0);
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(true);
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 520_000,
            video_packet_count_total: 4_400,
            audio_bytes_total: 96_000,
            observed_at_ms: 9_500.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("transportAwaitRecoveryAnchor".to_string()),
                chain_break_evidence: None,

                observed_at_ms: 9_500.0,
            },
            observed_at_ms: 9_500.0,
        });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

    let mut connection = ConnectionProjection::default();
    connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    connection.control_channel_open = true;
    connection.latest_transport_path = Some("Direct".to_string());
    connection.latest_rtt_ms = Some(260.0);
    connection.latest_loss_ratio_1s = Some(0.04);
    connection.last_observed_at_ms = Some(10_000.0);

    for (version, now_ms) in [(1, 10_000.0), (2, 10_460.0)] {
        let snapshot = TransportSnapshot::new(
            version,
            now_ms,
            connection.clone(),
            MediaProjection {
                frame_count: 220,
                ..MediaProjection::default()
            },
            RecoveryProjection {
                latest_diagnosis_label: Some("transportExpiredDeadline".to_string()),
                pending_action: false,
                successful_action_count: 0,
                failed_action_count: 0,
                last_observed_at_ms: Some(now_ms),
                ..Default::default()
            },
            BweProjection {
                latest_rtt_ms: Some(260.0),
                latest_loss_ratio_1s: Some(0.04),
                latest_actual_video_bitrate_kbps: Some(5_200.0),
                latest_observed_remb_kbps: Some(6_500),
                latest_transport_path: Some("Direct".to_string()),
                latest_sample_tick_ms: Some(now_ms),
                target_remb_kbps: Some(6_500),
                last_observed_at_ms: Some(now_ms),
            },
            DiagnosticsProjection::default(),
        );
        let commands = transport_commands(policy.on_snapshot(&snapshot));
        assert!(commands
            .iter()
            .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
        if version < 2 {
            sleep(Duration::from_millis(450));
        }
    }

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(
        ledger.gate_result,
        "suppressed:reconnectBlocked:transportGate:awaitingRecoveryChain"
    );
    assert_eq!(ledger.action_selected, "cooldownSuppressed");
}

#[test]
fn cloud_high_rtt_repeated_transport_expired_deadline_reconnects_after_recovery_chain_failure_and_connectivity_loss(
) {
    let runtime_config = Arc::new(Mutex::new(XbxEngineRuntimeConfig::default()));
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
        stats.session_phase = Some("steady".to_string());
        stats.transport_recovery_epoch = 44;
        stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
        stats.host_no_pending_pressure_level = Some("critical".to_string());
        stats.host_no_pending_streak = 420;
        stats.latest_video_host_present_time_ms = Some(9_000.0);
        stats.latest_video_decode_ok_time_ms = Some(9_350.0);
        stats.latest_video_packet_arrival_time_ms = Some(9_480.0);
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(true);
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
            video_bytes_total: 520_000,
            video_packet_count_total: 4_400,
            audio_bytes_total: 96_000,
            observed_at_ms: 9_500.0,
        });
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-await-recovery-anchor".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("transportAwaitRecoveryAnchor".to_string()),
                chain_break_evidence: None,

                observed_at_ms: 9_500.0,
            },
            observed_at_ms: 9_500.0,
        });
    }
    let mut policy = RtcSessionPolicy::new(runtime_config, runtime_stats.clone());

    let mut healthy_connection = ConnectionProjection::default();
    healthy_connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    healthy_connection.control_channel_open = true;
    healthy_connection.latest_transport_path = Some("Direct".to_string());
    healthy_connection.latest_rtt_ms = Some(260.0);
    healthy_connection.latest_loss_ratio_1s = Some(0.04);
    healthy_connection.last_observed_at_ms = Some(10_000.0);

    let first_snapshot = TransportSnapshot::new(
        1,
        10_000.0,
        healthy_connection,
        MediaProjection {
            frame_count: 220,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("transportExpiredDeadline".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(10_000.0),
            ..Default::default()
        },
        BweProjection {
            latest_rtt_ms: Some(260.0),
            latest_loss_ratio_1s: Some(0.04),
            latest_actual_video_bitrate_kbps: Some(5_200.0),
            latest_observed_remb_kbps: Some(6_500),
            latest_transport_path: Some("Direct".to_string()),
            latest_sample_tick_ms: Some(10_000.0),
            target_remb_kbps: Some(6_500),
            last_observed_at_ms: Some(10_000.0),
        },
        DiagnosticsProjection::default(),
    );
    let first_commands = transport_commands(policy.on_snapshot(&first_snapshot));
    assert!(first_commands
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));

    sleep(Duration::from_millis(450));
    if let Ok(mut stats) = runtime_stats.lock() {
        stats.latest_h264_inspection_observation =
            Some(crate::XbxEngineH264InspectionObservation {
                observation_id: 2,
                nal_types: vec!["SliceLayerWithoutPartitioningIdr".to_string()],
                nal_count: 1,
                vcl_nal_count: 1,
                has_inband_sps: false,
                has_inband_pps: false,
                committed_sps_present: false,
                committed_pps_present: false,
                slice_headers_valid: true,
                delta_continuation_ready: false,
                parameter_sets_changed: false,
                config_changed: false,
                is_idr: true,
                bootstrap_ready: false,
                bootstrap_reject_reason: Some("bootstrapMissingSps".to_string()),
                admission_accepted: true,
                observed_at_ms: 10_455.0,
                ..Default::default()
            });
    }

    let mut broken_connection = ConnectionProjection::default();
    broken_connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    broken_connection.last_observed_at_ms = Some(8_000.0);

    let second_snapshot = TransportSnapshot::new(
        2,
        10_460.0,
        broken_connection,
        MediaProjection {
            frame_count: 220,
            ..MediaProjection::default()
        },
        RecoveryProjection {
            latest_diagnosis_label: Some("transportExpiredDeadline".to_string()),
            pending_action: false,
            successful_action_count: 0,
            failed_action_count: 0,
            last_observed_at_ms: Some(10_460.0),
            ..Default::default()
        },
        BweProjection::default(),
        DiagnosticsProjection::default(),
    );
    let second_commands = transport_commands(policy.on_snapshot(&second_snapshot));
    assert!(second_commands.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestReconnectCandidate {
                reason,
                reason_domain,
                ..
            } if reason == "transportExpiredDeadline"
                && *reason_domain == crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport
        )
    }));

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let ledger = stats
        .latest_recovery_decision_ledger
        .as_ref()
        .expect("recovery decision ledger");
    assert_eq!(
        ledger.gate_result,
        "pass:reconnectGranted:connectivityEvidence"
    );
    assert_eq!(ledger.action_selected, "requestReconnectCandidate");
}

#[test]
fn cloud_high_rtt_sample_loss_then_recovered_late_stays_local_until_severe_deadline_reconnect() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));

    let sample_loss = harness.apply(
        10_800.0,
        ConnectionLifecycleStateFact::Connected,
        "transportSampleLoss",
        220,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 43;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Disconnected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 320;
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_180.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 740.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 120.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = None;
            stats.video_anchor_clean_observed_at_ms = None;
            stats.video_anchor_clean_source_event = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Disconnected,
                video_bytes_total: 540_000,
                video_packet_count_total: 4_500,
                audio_bytes_total: 96_000,
                observed_at_ms: now_ms - 4.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 14,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: now_ms - 4.0,
                    },
                    observed_at_ms: now_ms - 4.0,
                });
        },
    );
    assert!(sample_loss
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("sample loss decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportSampleLoss:transportSampleLoss"
        );
        assert_ne!(ledger.action_selected, "requestReconnectCandidate");
    });

    let recovered_late = harness.apply(
        10_920.0,
        ConnectionLifecycleStateFact::Connected,
        "transportRecoveredLate",
        220,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_220.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 780.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 160.0);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 12_000;
                track.video_packet_count_total += 80;
                track.observed_at_ms = now_ms - 4.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id = 15;
                timeline.chain.observed_at_ms = now_ms - 4.0;
                timeline.observed_at_ms = now_ms - 4.0;
            }
        },
    );
    assert!(recovered_late
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("transport recovered late decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportRecoveredLate:transportRecoveredLate"
        );
        assert_ne!(ledger.action_selected, "requestReconnectCandidate");
    });

    let severe_first = harness.apply(
        10_960.0,
        ConnectionLifecycleStateFact::Connected,
        "transportSevereDeadline",
        220,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_260.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 820.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 220.0);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.observed_at_ms = now_ms - 4.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id = 16;
                timeline.chain.observed_at_ms = now_ms - 4.0;
                timeline.observed_at_ms = now_ms - 4.0;
            }
        },
    );
    assert!(severe_first
        .iter()
        .all(|command| !matches!(command, TransportCommand::RequestReconnectCandidate { .. })));

    // `transport_deadline_reconnect_block_reason` 需要：(1) transport-await 硬证据；(2) 连接域失活证据。
    // 与同文件中 transport deadline + stale connection 的集成测例一致。
    let mut broken_connection = ConnectionProjection::default();
    broken_connection.lifecycle_state = ConnectionLifecycleStateFact::Connected;
    broken_connection.last_observed_at_ms = Some(8_000.0);

    let severe_second = harness.apply_with_connection_projection(
        11_000.0,
        broken_connection,
        "transportSevereDeadline",
        220,
        |stats| {
            let now_ms = crate::transport::rtc::stats::now_ms_f64();
            stats.latest_h264_inspection_observation =
                Some(crate::XbxEngineH264InspectionObservation {
                    observation_id: 901,
                    nal_types: vec!["SliceLayerWithoutPartitioningIdr".to_string()],
                    nal_count: 1,
                    vcl_nal_count: 1,
                    has_inband_sps: false,
                    has_inband_pps: false,
                    committed_sps_present: false,
                    committed_pps_present: false,
                    slice_headers_valid: true,
                    delta_continuation_ready: false,
                    parameter_sets_changed: false,
                    config_changed: false,
                    is_idr: true,
                    bootstrap_ready: false,
                    bootstrap_reject_reason: Some("bootstrapMissingSps".to_string()),
                    admission_accepted: true,
                    observed_at_ms: 10_990.0,
                    ..Default::default()
                });
            stats.latest_video_host_present_time_ms = Some(now_ms - 1_300.0);
            stats.latest_video_decode_ok_time_ms = Some(now_ms - 860.0);
            stats.latest_video_packet_arrival_time_ms = Some(now_ms - 260.0);
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.observed_at_ms = now_ms - 4.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id = 17;
                timeline.chain.observed_at_ms = now_ms - 4.0;
                timeline.observed_at_ms = now_ms - 4.0;
            }
        },
    );
    assert!(severe_second.iter().any(|command| {
        matches!(
            command,
            TransportCommand::RequestReconnectCandidate {
                reason,
                reason_domain,
                ..
            } if reason == "transportSevereDeadline"
                && *reason_domain == crate::XbxEngineRecoveryReasonDomain::ConnectivityTransport
        )
    }));
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("severe deadline decision ledger");
        assert_eq!(
            ledger.input_signal,
            "transportSevereDeadline:transportSevereDeadline"
        );
        assert_eq!(ledger.action_selected, "requestReconnectCandidate");
    });
}
