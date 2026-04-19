//! RFC PLY-INT-07 .. PLY-INT-12

use super::super::harness::RecoveryIntegrationHarness;
use super::common::{assert_cmds_have_no_reconnect, wall_observed_ms};
use crate::transport::rtc::facts::{ConnectionLifecycleStateFact, TransportCommand};

#[test]
fn playback_phase_int07_cloud_high_rtt_with_transport_await_must_not_emit_connectivity_reconnect() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t = wall_observed_ms();
    let cmds = harness.apply(
        t,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        200,
        |stats| {
            stats.video_rtt_ms = Some(120.0);
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 88;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 160;
            stats.latest_video_host_present_time_ms = Some(t - 1_100.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 400.0);
            stats.latest_video_packet_arrival_time_ms = Some(t - 3.0);
            stats.video_decoder_stalled = Some(false);
            // 画像先于 cloudHighRtt 检查 displayConstrained；stall=true 会永久盖住 RTT 子画像。
            stats.video_renderer_stalled = Some(false);
            stats.inbound_primary_video_bytes_total = 600_000;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 300_000,
                video_packet_count_total: 2_600,
                audio_bytes_total: 50_000,
                observed_at_ms: t - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 501,
                    source_event: "gap-repair-in-flight".to_string(),
                    gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
                        state: "keyframe".to_string(),
                        sequence: Some(1),
                        frame_rtp_timestamp: Some(900),
                        frame_importance: Some("keyframe".to_string()),
                        budget_importance: None,

                        evidence_importance: None,

                        gap_dependency_confidence: None,

                        observed_at_ms: t - 4.0,
                    }),
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: t - 2.0,
                    },
                    observed_at_ms: t - 2.0,
                });
        },
    );
    assert_cmds_have_no_reconnect(&cmds, "PLY-INT-07");
    harness.with_stats(|stats| {
        assert!(
            stats
                .effective_remote_profile_label
                .as_deref()
                .unwrap_or("")
                .contains("cloudHighRtt")
                || stats.dynamic_remote_subprofile.as_deref() == Some("cloudHighRtt"),
            "PLY-INT-07: fixture 应归类 cloudHighRtt"
        );
    });
}

#[test]
fn playback_phase_int08_hard_fallback_reconnect_requires_evidence_not_uncapped_spam() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t = wall_observed_ms();
    let cmds = harness.apply(
        t,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        100,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 99;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 300;
            stats.latest_video_host_present_time_ms = Some(t - 3_000.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 2_800.0);
            stats.video_renderer_stalled = Some(true);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 50_000,
                video_packet_count_total: 400,
                audio_bytes_total: 10_000,
                observed_at_ms: t - 2.0,
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

                        observed_at_ms: t - 2.0,
                    },
                    observed_at_ms: t - 2.0,
                });
        },
    );
    let reconnect_count = cmds
        .iter()
        .filter(|c| matches!(c, TransportCommand::RequestReconnectCandidate { .. }))
        .count();
    assert!(
        reconnect_count <= 1,
        "PLY-INT-08: 单拍不应无节制重复 reconnect candidate，count={reconnect_count} cmds={cmds:?}"
    );
}

#[test]
fn playback_phase_int09_coalesced_keyframe_inflight_sequence_must_eventually_change_gate_or_action()
{
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let mut saw_non_coalesce_stuck = false;
    for step in 0..12u32 {
        let t = wall_observed_ms() + f64::from(step) * 18.0;
        let _ = harness.apply(
            t,
            ConnectionLifecycleStateFact::Connected,
            "transportAwaitRecoveryAnchor",
            260 + u64::from(step),
            |stats| {
                stats.session_phase = Some("recovering".to_string());
                stats.transport_recovery_epoch = 75;
                stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
                stats.host_no_pending_pressure_level = Some("critical".to_string());
                stats.host_no_pending_streak = 200;
                stats.latest_video_host_present_time_ms = Some(t - 1_700.0);
                stats.latest_video_decode_ok_time_ms = Some(t - 450.0);
                stats.latest_video_packet_arrival_time_ms = Some(t - 2.0);
                stats.video_decoder_stalled = Some(false);
                stats.video_renderer_stalled = Some(true);
                stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                    state: "remoteTrackAttached".to_string(),
                    video_width: Some(1920),
                    video_height: Some(1080),
                    mime_type: Some("video/H264".to_string()),
                    transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                    video_bytes_total: 420_000 + u64::from(step) * 1_000,
                    video_packet_count_total: 3_800 + u64::from(step) * 8,
                    audio_bytes_total: 64_000,
                    observed_at_ms: t - 2.0,
                });
                stats.latest_video_timeline_observation =
                    Some(crate::XbxEngineVideoTimelineObservation {
                        observation_id: 600 + u64::from(step),
                        source_event: "frame-await-recovery-anchor".to_string(),
                        gap: None,
                        frame: None,
                        chain: crate::XbxEngineVideoTimelineChainSnapshot {
                            state: "recovering".to_string(),
                            reason: Some("transportAwaitRecoveryAnchor".to_string()),
                            chain_break_evidence: None,

                            observed_at_ms: t - 2.0,
                        },
                        observed_at_ms: t - 2.0,
                    });
            },
        );
        harness.with_stats(|stats| {
            if let Some(ledger) = stats.latest_recovery_decision_ledger.as_ref() {
                if ledger.gate_result != "coalesced:keyframeInFlight"
                    || ledger.action_selected != "coalesced:keyframeInFlight"
                {
                    saw_non_coalesce_stuck = true;
                }
            }
        });
    }
    assert!(
        saw_non_coalesce_stuck,
        "PLY-INT-09: 多拍后 gate/action 不得永远停在 coalesced:keyframeInFlight 同态"
    );
}

#[test]
fn playback_phase_int10_transport_deferred_episode_must_not_promote_stable_serving() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t = wall_observed_ms();
    let _ = harness.apply(
        t,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        300,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 81;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 200;
            stats.latest_video_host_present_time_ms = Some(t - 1_400.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 500.0);
            stats.video_renderer_stalled = Some(true);
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 9001,
                    request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                    request_kind: Some("control".to_string()),
                    status: "deferred".to_string(),
                    status_detail: Some("sameFamilyCoalesced:transportStageSuppressed".to_string()),
                    requested_at_ms: t - 200.0,
                    sent_at_ms: None,
                    deadline_at_ms: Some(t + 400.0),
                    transport_detail: None,
                    first_video_packet_at_ms: None,
                    first_video_packet_rtp_timestamp: None,
                    first_video_packet_is_keyframe: None,
                    first_keyframe_packet_at_ms: None,
                    first_keyframe_decoded_at_ms: None,
                    response_rtp_timestamp: None,
                    response_frame_seq: None,
                    response_verdict: Some("transportDeferred".to_string()),
                    lifecycle_phase: None,
                    retired_at_ms: None,
                });
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 200_000,
                video_packet_count_total: 1_800,
                audio_bytes_total: 40_000,
                observed_at_ms: t - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 910,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: t - 2.0,
                    },
                    observed_at_ms: t - 2.0,
                });
        },
    );
    harness.with_stats(|stats| {
        assert_ne!(
            stats.video_owner_state.as_deref(),
            Some("stable-serving"),
            "PLY-INT-10: deferred episode 不得伪装成 stable-serving"
        );
    });
}

#[test]
fn playback_phase_int11_terminal_invalid_bootstrap_must_request_keyframe_or_stronger_not_silent() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t = wall_observed_ms();
    let cmds = harness.apply(
        t,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        310,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 82;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 210;
            stats.latest_video_host_present_time_ms = Some(t - 1_500.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 520.0);
            stats.video_renderer_stalled = Some(true);
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 9101,
                    request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                    request_kind: Some("control".to_string()),
                    status: "terminalDeferredInvalidBootstrap".to_string(),
                    status_detail: Some("terminalDeferredInvalidBootstrap".to_string()),
                    requested_at_ms: t - 300.0,
                    sent_at_ms: Some(t - 290.0),
                    deadline_at_ms: Some(t + 200.0),
                    transport_detail: None,
                    first_video_packet_at_ms: None,
                    first_video_packet_rtp_timestamp: None,
                    first_video_packet_is_keyframe: None,
                    first_keyframe_packet_at_ms: None,
                    first_keyframe_decoded_at_ms: None,
                    response_rtp_timestamp: None,
                    response_frame_seq: None,
                    response_verdict: Some("transportDeferred".to_string()),
                    lifecycle_phase: None,
                    retired_at_ms: None,
                });
            stats.latest_h264_inspection_observation =
                Some(crate::XbxEngineH264InspectionObservation {
                    observation_id: 920,
                    frame_rtp_timestamp: Some(1),
                    nal_types: vec!["VCL".to_string()],
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
                    sample_width: Some(1920),
                    sample_height: Some(1080),
                    bootstrap_ready: false,
                    bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
                    admission_accepted: false,
                    observed_at_ms: t - 5.0,

                    ..Default::default()
                });
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 210_000,
                video_packet_count_total: 1_900,
                audio_bytes_total: 42_000,
                observed_at_ms: t - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 921,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: t - 2.0,
                    },
                    observed_at_ms: t - 2.0,
                });
        },
    );
    let has_keyframe_or_reset = cmds.iter().any(|c| {
        matches!(
            c,
            TransportCommand::RequestKeyframe { .. } | TransportCommand::RequestDecoderReset { .. }
        )
    });
    harness.with_stats(|stats| {
        let ledger = stats.latest_recovery_decision_ledger.as_ref();
        let action_ok = ledger.is_some_and(|l| {
            l.action_selected == "requestKeyframe"
                || l.action_selected == "requestDecoderReset"
                || l.action_selected == "requestReconnectCandidate"
        });
        assert!(
            has_keyframe_or_reset || action_ok,
            "PLY-INT-11: 须有显式恢复动作，cmds={cmds:?} ledger={ledger:?}"
        );
    });
}

#[test]
fn playback_phase_int12_recovery_settled_plus_progress_must_not_stay_blocked() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t0 = wall_observed_ms();
    let _ = harness.apply(
        t0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        400,
        |stats| {
            let t = t0;
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 90;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.video_decoder_recovery_event = Some("recovery-settled".to_string());
            stats.video_decoder_recovery_state = Some("idle".to_string());
            stats.latest_video_host_present_time_ms = Some(t - 800.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 600.0);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 250_000,
                video_packet_count_total: 2_200,
                audio_bytes_total: 44_000,
                observed_at_ms: t - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 930,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        chain_break_evidence: None,

                        observed_at_ms: t,
                    },
                    observed_at_ms: t,
                });
        },
    );
    let t1 = t0 + 30.0;
    let _ = harness.apply(
        t1,
        ConnectionLifecycleStateFact::Connected,
        "none",
        400,
        |stats| {
            let t = t1;
            stats.transport_recovery_epoch = 90;
            stats.session_phase = Some("steady".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(t - 20.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 15.0);
            stats.video_anchor_clean_epoch = Some(90);
            stats.video_anchor_clean_observed_at_ms = Some(t - 18.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
        },
    );
    harness.with_stats(|stats| {
        assert_ne!(
            stats.recovery_exit_gate.as_deref(),
            Some("recovery-blocked"),
            "PLY-INT-12: recovery-settled 后不应长期卡在 recovery-blocked"
        );
    });
}
