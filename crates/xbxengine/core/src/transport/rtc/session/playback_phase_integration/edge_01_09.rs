//! RFC PLY-EDGE-01 .. PLY-EDGE-09

use super::super::harness::RecoveryIntegrationHarness;
use super::common::{
    assert_cmds_have_no_reconnect, fill_twcc_stable_local_feedback, wall_observed_ms,
};
use crate::transport::rtc::facts::{ConnectionLifecycleStateFact, TransportCommand};

#[test]
fn playback_phase_edge01_transport_deferred_flood_must_not_mark_recovery_success() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t = wall_observed_ms();
    let _ = harness.apply(
        t,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        1000,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 200;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 200;
            stats.latest_video_host_present_time_ms = Some(t - 1_500.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 600.0);
            stats.video_renderer_stalled = Some(true);
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 10_001,
                    request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                    request_kind: Some("control".to_string()),
                    status: "deferred".to_string(),
                    status_detail: Some("sameFamilyCoalesced:transportStageSuppressed".to_string()),
                    requested_at_ms: t - 100.0,
                    sent_at_ms: None,
                    deadline_at_ms: Some(t + 500.0),
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
                });
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 220_000,
                video_packet_count_total: 2_000,
                audio_bytes_total: 44_000,
                observed_at_ms: t - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 10_010,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
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
            "PLY-EDGE-01: deferred 洪峰不得误判恢复成功"
        );
    });
}

#[test]
fn playback_phase_edge02_non_idr_vcl_with_delta_ready_must_not_exit_to_stable_serving() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t = wall_observed_ms();
    let _ = harness.apply(
        t,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        1010,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 201;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 220;
            stats.latest_video_host_present_time_ms = Some(t - 1_400.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 550.0);
            stats.video_renderer_stalled = Some(true);
            stats.latest_h264_inspection_observation =
                Some(crate::XbxEngineH264InspectionObservation {
                    observation_id: 10_100,
                    frame_rtp_timestamp: Some(2),
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
                    observed_at_ms: t - 4.0,
                });
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 230_000,
                video_packet_count_total: 2_050,
                audio_bytes_total: 45_000,
                observed_at_ms: t - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 10_101,
                    source_event: "frame-inspection-rejected-await-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
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
            "PLY-EDGE-02"
        );
    });
}

#[test]
fn playback_phase_edge03_long_coalesce_keyframe_inflight_must_not_be_eternally_stuck() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let mut stuck = 0u32;
    for step in 0..20u32 {
        let t = wall_observed_ms() + f64::from(step) * 15.0;
        let _ = harness.apply(
            t,
            ConnectionLifecycleStateFact::Connected,
            "transportAwaitRecoveryKeyframe",
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
                    video_bytes_total: 420_000 + u64::from(step) * 500,
                    video_packet_count_total: 3_800 + u64::from(step) * 4,
                    audio_bytes_total: 64_000,
                    observed_at_ms: t - 2.0,
                });
                stats.latest_video_timeline_observation =
                    Some(crate::XbxEngineVideoTimelineObservation {
                        observation_id: 11_000 + u64::from(step),
                        source_event: "frame-await-recovery-keyframe".to_string(),
                        gap: None,
                        frame: None,
                        chain: crate::XbxEngineVideoTimelineChainSnapshot {
                            state: "recovering".to_string(),
                            reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                            observed_at_ms: t - 2.0,
                        },
                        observed_at_ms: t - 2.0,
                    });
            },
        );
        harness.with_stats(|stats| {
            if let Some(l) = stats.latest_recovery_decision_ledger.as_ref() {
                if l.gate_result == "coalesced:keyframeInFlight"
                    && l.action_selected == "coalesced:keyframeInFlight"
                {
                    stuck += 1;
                }
            }
        });
    }
    assert!(
        stuck < 20,
        "PLY-EDGE-03: 不应 20 拍全部为 coalesced:keyframeInFlight 同态，stuck={stuck}"
    );
}

#[test]
fn playback_phase_edge04_wait_for_burst_repeated_must_eventually_emit_action_or_pass() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let mut saw_non_wait_burst_only = false;
    for step in 0..18u32 {
        let t = wall_observed_ms() + f64::from(step) * 20.0;
        let _ = harness.apply(
            t,
            ConnectionLifecycleStateFact::Connected,
            "transportAwaitRecoveryKeyframe",
            400 + u64::from(step),
            |stats| {
                stats.session_phase = Some("recovering".to_string());
                stats.transport_recovery_epoch = 130;
                stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
                stats.host_no_pending_pressure_level = Some("critical".to_string());
                stats.host_no_pending_streak = 260;
                stats.latest_video_host_present_time_ms = Some(t - 1_900.0);
                stats.latest_video_decode_ok_time_ms = Some(t - 480.0);
                stats.latest_video_packet_arrival_time_ms = Some(t - 1.0);
                stats.video_decoder_stalled = Some(false);
                stats.video_renderer_stalled = Some(true);
                stats.inbound_primary_video_bytes_total = 1_200_000;
                stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                    state: "remoteTrackAttached".to_string(),
                    video_width: Some(1920),
                    video_height: Some(1080),
                    mime_type: Some("video/H264".to_string()),
                    transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                    video_bytes_total: 310_000 + u64::from(step) * 800,
                    video_packet_count_total: 2_700 + u64::from(step) * 6,
                    audio_bytes_total: 50_000,
                    observed_at_ms: t - 1.0,
                });
                stats.latest_video_timeline_observation =
                    Some(crate::XbxEngineVideoTimelineObservation {
                        observation_id: 12_000 + u64::from(step),
                        source_event: "frame-await-recovery-keyframe".to_string(),
                        gap: None,
                        frame: None,
                        chain: crate::XbxEngineVideoTimelineChainSnapshot {
                            state: "recovering".to_string(),
                            reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                            observed_at_ms: t - 2.0,
                        },
                        observed_at_ms: t - 2.0,
                    });
            },
        );
        harness.with_stats(|stats| {
            if let Some(l) = stats.latest_recovery_decision_ledger.as_ref() {
                if l.gate_result != "suppressed:waitForBurst" {
                    saw_non_wait_burst_only = true;
                }
            }
        });
    }
    assert!(
        saw_non_wait_burst_only,
        "PLY-EDGE-04: waitForBurst 不应成为唯一永久抑制"
    );
}

#[test]
fn playback_phase_edge05_cooldown_suppressed_must_not_erase_connectivity_escalation_path() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t = wall_observed_ms();
    let cmds = harness.apply(
        t,
        ConnectionLifecycleStateFact::Connected,
        "transportSevereDeadline",
        50,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 5;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 300;
            stats.latest_video_host_present_time_ms = Some(t - 5_000.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 4_800.0);
            stats.latest_video_packet_arrival_time_ms = Some(t - 3_000.0);
            stats.video_renderer_stalled = Some(true);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 10_000,
                video_packet_count_total: 80,
                audio_bytes_total: 5_000,
                observed_at_ms: t - 2.0,
            });
        },
    );
    let has_reconnect = cmds
        .iter()
        .any(|c| matches!(c, TransportCommand::RequestReconnectCandidate { .. }));
    let ledger_escalates = harness.with_stats(|stats| {
        stats
            .latest_recovery_decision_ledger
            .as_ref()
            .is_some_and(|l| {
                l.gate_result.contains("suppressed") || l.action_selected.contains("reconnect")
            })
    });
    let has_reconnect_or_suppressed = has_reconnect || ledger_escalates;
    assert!(
        has_reconnect_or_suppressed,
        "PLY-EDGE-05: severe deadline 仍应可达 reconnect 或显式抑制链，cmds={cmds:?}"
    );
}

#[test]
fn playback_phase_edge06_decoder_reset_inflight_coalesce_must_respect_budget_signal() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t = wall_observed_ms();
    let _ = harness.apply(
        t,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        1020,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 210;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 240;
            stats.latest_video_host_present_time_ms = Some(t - 1_600.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 500.0);
            stats.video_renderer_stalled = Some(true);
            stats.latest_video_escalation_observation =
                Some(crate::XbxEngineVideoEscalationObservation {
                    observation_id: 13_000,
                    reason: "transportAwaitRecoveryKeyframe".to_string(),
                    action: "requestDecoderReset".to_string(),
                    recovery_stage: "rebuilding-supply".to_string(),
                    recovery_chain_value: "anchor".to_string(),
                    recovery_failure_cost: "high".to_string(),
                    recovery_window_source: "transport-await-window".to_string(),
                    observed_at_ms: t - 8.0,
                });
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 240_000,
                video_packet_count_total: 2_100,
                audio_bytes_total: 46_000,
                observed_at_ms: t - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 13_001,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: t - 2.0,
                    },
                    observed_at_ms: t - 2.0,
                });
        },
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("PLY-EDGE-06 ledger");
        assert!(
            ledger.gate_result != "coalesced:decoderResetInFlight"
                || ledger
                    .budget_after
                    .as_ref()
                    .is_some_and(|b| b.decoder_reset_budget_limit >= b.decoder_reset_budget_used),
            "PLY-EDGE-06: coalesce reset 时应携带 budget 语义"
        );
    });
}

#[test]
fn playback_phase_edge07_audio_twcc_ignored_must_not_block_video_recovery_gate() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t = wall_observed_ms();
    let cmds = harness.apply(
        t,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        1030,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 220;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 200;
            stats.latest_video_host_present_time_ms = Some(t - 1_400.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 480.0);
            stats.video_renderer_stalled = Some(true);
            stats.latest_twcc_remote_stream_observation =
                Some(crate::XbxEngineTwccRemoteStreamObservation {
                    observation_id: 50,
                    ssrc: 11_112_233,
                    mime_type: "audio/opus".to_string(),
                    twcc_ext_id: Some(2),
                    header_extensions: vec![],
                    rtcp_feedback: vec!["transport-cc".to_string()],
                    observed_at_ms: t - 1.0,
                });
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 260_000,
                video_packet_count_total: 2_200,
                audio_bytes_total: 48_000,
                observed_at_ms: t - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 13_100,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: t - 2.0,
                    },
                    observed_at_ms: t - 2.0,
                });
        },
    );
    let has_keyframe_cmd = cmds
        .iter()
        .any(|c| matches!(c, TransportCommand::RequestKeyframe { .. }));
    let ledger_has_signal = harness.with_stats(|stats| {
        stats
            .latest_recovery_decision_ledger
            .as_ref()
            .is_some_and(|l| l.gate_result != "no-signal")
    });
    assert!(
        has_keyframe_cmd || ledger_has_signal,
        "PLY-EDGE-07: 视频恢复门闩不应被纯音频 TWCC 观测饿死"
    );
}

#[test]
fn playback_phase_edge08_twcc_stable_display_starved_no_connectivity_reconnect() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t = wall_observed_ms();
    let cmds = harness.apply(
        t,
        ConnectionLifecycleStateFact::Connected,
        "none",
        200,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 61;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 240;
            stats.latest_video_host_present_time_ms = Some(t - 1_240.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 720.0);
            stats.video_renderer_stalled = Some(true);
            stats.inbound_primary_video_bytes_total = 900_000;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 180_000,
                video_packet_count_total: 1_600,
                audio_bytes_total: 32_000,
                observed_at_ms: t - 2.0,
            });
            fill_twcc_stable_local_feedback(stats, t);
        },
    );
    assert_cmds_have_no_reconnect(&cmds, "PLY-EDGE-08");
}

#[test]
fn playback_phase_edge09_high_present_overwrite_must_not_force_stable_serving_while_chain_recovering(
) {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t = wall_observed_ms();
    let _ = harness.apply(
        t,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        1040,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 230;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(t - 30.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 28.0);
            stats.video_present_overwrite_count_total = 400;
            stats.video_present_submit_count_total = 500;
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 500_000,
                video_packet_count_total: 4_000,
                audio_bytes_total: 60_000,
                observed_at_ms: t - 1.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 13_200,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
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
            "PLY-EDGE-09"
        );
    });
}
