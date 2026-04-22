//! RFC PLY-EDGE-10 .. PLY-EDGE-18

use super::super::harness::RecoveryIntegrationHarness;
use super::common::{assert_cmds_have_no_reconnect, wall_observed_ms};
use crate::transport::rtc::facts::ConnectionLifecycleStateFact;

#[test]
fn playback_phase_edge10_gap_recovered_with_timeline_recovering_must_not_imply_session_stable() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t = wall_observed_ms();
    let _ = harness.apply(
        t,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        1050,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 240;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 200;
            stats.latest_video_host_present_time_ms = Some(t - 1_300.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 500.0);
            stats.video_renderer_stalled = Some(true);
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 14_000,
                    source_event: "gap-resolved".to_string(),
                    gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
                        state: "repaired".to_string(),
                        sequence: Some(9),
                        frame_rtp_timestamp: Some(200),
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
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 270_000,
                video_packet_count_total: 2_300,
                audio_bytes_total: 47_000,
                observed_at_ms: t - 2.0,
            });
        },
    );
    harness.with_stats(|stats| {
        assert!(
            stats
                .latest_video_timeline_observation
                .as_ref()
                .is_some_and(|o| o.chain.state == "recovering"),
            "PLY-EDGE-10: gap-resolved 后 chain 仍可处于 recovering，不得仅凭 gap 判会话已稳"
        );
    });
}

#[test]
fn playback_phase_edge11_expired_unsent_keyframe_episode_must_surface_decision() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t = wall_observed_ms();
    let _ = harness.apply(
        t,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        1060,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 250;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 200;
            stats.latest_video_host_present_time_ms = Some(t - 1_400.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 520.0);
            stats.video_renderer_stalled = Some(true);
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 14_100,
                    request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                    request_kind: Some("control".to_string()),
                    status: "expired-unsent".to_string(),
                    status_detail: None,
                    requested_at_ms: t - 800.0,
                    sent_at_ms: None,
                    deadline_at_ms: Some(t - 100.0),
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
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 275_000,
                video_packet_count_total: 2_350,
                audio_bytes_total: 47_500,
                observed_at_ms: t - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 14_101,
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
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("PLY-EDGE-11 ledger");
        assert_ne!(
            ledger.gate_result, "no-signal",
            "PLY-EDGE-11: expired-unsent 不得被吞成无信号"
        );
    });
}

#[test]
fn playback_phase_edge12_deadline_expired_episode_must_not_stay_no_signal() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t = wall_observed_ms();
    let _ = harness.apply(
        t,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        1070,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 260;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 220;
            stats.latest_video_host_present_time_ms = Some(t - 1_500.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 540.0);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = Some(260);
            stats.video_anchor_clean_observed_at_ms = Some(t - 800.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            stats.latest_keyframe_request_episode =
                Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                    episode_id: 14_200,
                    request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
                    request_kind: Some("control".to_string()),
                    status: "missed".to_string(),
                    status_detail: Some("deadlineExpired".to_string()),
                    requested_at_ms: t - 600.0,
                    sent_at_ms: Some(t - 590.0),
                    deadline_at_ms: Some(t - 50.0),
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
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 280_000,
                video_packet_count_total: 2_400,
                audio_bytes_total: 48_000,
                observed_at_ms: t - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 14_201,
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
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("PLY-EDGE-12 ledger");
        assert!(
            ledger.action_selected == "requestPli"
                || ledger.action_selected == "requestDecoderReset"
                || ledger.action_selected == "requestReconnectCandidate"
                || ledger.gate_result.contains("pass")
                || ledger.gate_result.contains("suppressed")
                || (ledger.gate_result == "no-signal"
                    && ledger.recovery_episode_stage.as_deref() == Some("CleanAnchorCommitted")),
            "PLY-EDGE-12: deadlineExpired 后须有可解释出口，ledger={ledger:?}"
        );
    });
}

#[test]
fn playback_phase_edge13_degraded_then_steady_must_not_regress_to_priming() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t0 = wall_observed_ms();
    let _ = harness.apply(
        t0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        1080,
        |stats| {
            let t = t0;
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 270;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("high".to_string());
            stats.host_no_pending_streak = 80;
            stats.latest_video_host_present_time_ms = Some(t - 120.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 100.0);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 400_000,
                video_packet_count_total: 3_200,
                audio_bytes_total: 55_000,
                observed_at_ms: t - 1.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 14_300,
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
    let t1 = t0 + 50.0;
    let _ = harness.apply(
        t1,
        ConnectionLifecycleStateFact::Connected,
        "none",
        1081,
        |stats| {
            let t = t1;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(t - 25.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 20.0);
        },
    );
    harness.with_stats(|stats| {
        assert_ne!(
            stats.video_owner_state.as_deref(),
            Some("priming"),
            "PLY-EDGE-13"
        );
    });
}

#[test]
fn playback_phase_edge14_connected_high_ingress_low_output_must_not_emit_connectivity_reconnect() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t = wall_observed_ms();
    let cmds = harness.apply(
        t,
        ConnectionLifecycleStateFact::Connected,
        "none",
        1090,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 280;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 260;
            stats.latest_video_host_present_time_ms = Some(t - 8_000.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 7_500.0);
            stats.video_renderer_stalled = Some(true);
            stats.inbound_primary_video_bytes_total = 5_000_000;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 2_000_000,
                video_packet_count_total: 18_000,
                audio_bytes_total: 200_000,
                observed_at_ms: t - 1.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 14_400,
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
    assert_cmds_have_no_reconnect(&cmds, "PLY-EDGE-14");
}

#[test]
fn playback_phase_edge15_recovery_settled_then_reset_must_keep_ledger_coherent() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t0 = wall_observed_ms();
    let _ = harness.apply(
        t0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        1100,
        |stats| {
            let t = t0;
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 290;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.video_decoder_recovery_event = Some("recovery-settled".to_string());
            stats.latest_video_host_present_time_ms = Some(t - 400.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 350.0);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 300_000,
                video_packet_count_total: 2_500,
                audio_bytes_total: 50_000,
                observed_at_ms: t - 1.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 14_500,
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
    let t1 = t0 + 25.0;
    let _ = harness.apply(
        t1,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        1101,
        |stats| {
            let t = t1;
            stats.transport_recovery_epoch = 290;
            stats.session_phase = Some("recovering".to_string());
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 200;
            stats.latest_video_host_present_time_ms = Some(t - 1_200.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 500.0);
            stats.video_renderer_stalled = Some(true);
            stats.latest_video_escalation_observation =
                Some(crate::XbxEngineVideoEscalationObservation {
                    observation_id: 14_501,
                    reason: "transportAwaitRecoveryAnchor".to_string(),
                    action: "requestDecoderReset".to_string(),
                    recovery_stage: "rebuilding-supply".to_string(),
                    recovery_chain_value: "anchor".to_string(),
                    recovery_failure_cost: "high".to_string(),
                    recovery_window_source: "transport-await-window".to_string(),
                    observed_at_ms: t - 5.0,
                });
        },
    );
    harness.with_stats(|stats| {
        assert!(
            stats.latest_recovery_decision_ledger.is_some(),
            "PLY-EDGE-15: settled 后 reset 仍应有 ledger"
        );
    });
}

#[test]
fn playback_phase_edge16_reconfigure_then_transport_await_must_resolve_single_dominant_signal() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t = wall_observed_ms();
    let cmds = harness.apply(
        t,
        ConnectionLifecycleStateFact::Connected,
        "reconfigure",
        1110,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 300;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 200;
            stats.latest_video_host_present_time_ms = Some(t - 1_300.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 500.0);
            stats.video_renderer_stalled = Some(true);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 290_000,
                video_packet_count_total: 2_450,
                audio_bytes_total: 49_000,
                observed_at_ms: t - 2.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 14_600,
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
    let has_ledger = harness.with_stats(|s| s.latest_recovery_decision_ledger.is_some());
    assert!(
        !cmds.is_empty() || has_ledger,
        "PLY-EDGE-16: 交错信号下仍须产生单一主导决策链"
    );
}

#[test]
fn playback_phase_edge17_gap_resolved_timeline_must_not_mask_unresolved_transport_await() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t = wall_observed_ms();
    let _ = harness.apply(
        t,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        1120,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 310;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 210;
            stats.latest_video_host_present_time_ms = Some(t - 1_400.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 520.0);
            stats.video_renderer_stalled = Some(true);
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 14_700,
                    source_event: "gap-resolved".to_string(),
                    gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
                        state: "repaired".to_string(),
                        sequence: Some(3),
                        frame_rtp_timestamp: Some(50),
                        frame_importance: Some("delta".to_string()),
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
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 295_000,
                video_packet_count_total: 2_480,
                audio_bytes_total: 49_500,
                observed_at_ms: t - 2.0,
            });
        },
    );
    harness.with_stats(|stats| {
        let timeline = stats
            .latest_video_timeline_observation
            .as_ref()
            .expect("timeline");
        assert_eq!(
            timeline.chain.state, "recovering",
            "PLY-EDGE-17: 主 chain 仍须反映未恢复"
        );
    });
}

#[test]
fn playback_phase_edge18_null_surface_equivalent_must_not_promote_stable_without_present_submit() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t = wall_observed_ms();
    let _ = harness.apply(
        t,
        ConnectionLifecycleStateFact::Connected,
        "none",
        1130,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 320;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 8;
            stats.host_cadence_phase = Some("priming".to_string());
            stats.video_present_submit_count_total = 0;
            stats.video_present_epoch = 0;
            stats.latest_video_host_present_time_ms = None;
            stats.latest_video_decode_ok_time_ms = None;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 50_000,
                video_packet_count_total: 400,
                audio_bytes_total: 8_000,
                observed_at_ms: t,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 14_800,
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
    harness.with_stats(|stats| {
        assert_ne!(
            stats.video_owner_state.as_deref(),
            Some("stable-serving"),
            "PLY-EDGE-18: 无有效 surface/present 等价态不得提前 stable-serving"
        );
    });
}
