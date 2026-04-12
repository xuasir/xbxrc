//! RFC PLY-INT-13 .. PLY-INT-18

use super::super::harness::RecoveryIntegrationHarness;
use super::common::{
    assert_cmds_have_no_reconnect, fill_twcc_stable_local_feedback, wall_observed_ms,
};
use crate::api::backend::XbxEngineVideoFrameDropObservation;
use crate::transport::rtc::facts::ConnectionLifecycleStateFact;

#[test]
fn playback_phase_int13_decoder_reset_burst_must_be_bounded_per_window() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let mut decoder_reset_hits = 0u32;
    for i in 0..8u32 {
        let t = wall_observed_ms() + f64::from(i) * 12.0;
        let _cmds = harness.apply(
            t,
            ConnectionLifecycleStateFact::Connected,
            "transportAwaitRecoveryKeyframe",
            500 + u64::from(i),
            |stats| {
                stats.session_phase = Some("recovering".to_string());
                stats.transport_recovery_epoch = 100;
                stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
                stats.host_no_pending_pressure_level = Some("critical".to_string());
                stats.host_no_pending_streak = 250;
                stats.latest_video_host_present_time_ms = Some(t - 1_800.0);
                stats.latest_video_decode_ok_time_ms = Some(t - 500.0);
                stats.video_renderer_stalled = Some(true);
                stats.latest_video_escalation_observation =
                    Some(crate::XbxEngineVideoEscalationObservation {
                        observation_id: 7000 + u64::from(i),
                        reason: "transportAwaitRecoveryKeyframe".to_string(),
                        action: "requestDecoderReset".to_string(),
                        recovery_stage: "rebuilding-supply".to_string(),
                        recovery_chain_value: "anchor".to_string(),
                        recovery_failure_cost: "high".to_string(),
                        recovery_window_source: "transport-await-window".to_string(),
                        observed_at_ms: t - 5.0,
                    });
                stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                    state: "remoteTrackAttached".to_string(),
                    video_width: Some(1920),
                    video_height: Some(1080),
                    mime_type: Some("video/H264".to_string()),
                    transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                    video_bytes_total: 300_000,
                    video_packet_count_total: 2_500,
                    audio_bytes_total: 50_000,
                    observed_at_ms: t - 2.0,
                });
                stats.latest_video_timeline_observation =
                    Some(crate::XbxEngineVideoTimelineObservation {
                        observation_id: 7100 + u64::from(i),
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
            if stats
                .latest_recovery_decision_ledger
                .as_ref()
                .is_some_and(|l| l.action_selected == "requestDecoderReset")
            {
                decoder_reset_hits += 1;
            }
        });
    }
    assert!(
        decoder_reset_hits <= 4,
        "PLY-INT-13: decoder reset 决策在短窗内应有上界，ledger_hits={decoder_reset_hits}"
    );
}

#[test]
fn playback_phase_int14_remote_track_ingress_growth_alone_must_not_force_stable_serving() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t = wall_observed_ms();
    let _ = harness.apply(
        t,
        ConnectionLifecycleStateFact::Connected,
        "none",
        600,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 40;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 200;
            stats.latest_video_host_present_time_ms = Some(t - 2_000.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 1_800.0);
            stats.video_renderer_stalled = Some(true);
            stats.inbound_primary_video_bytes_total = 2_000_000;
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 900_000,
                video_packet_count_total: 12_000,
                audio_bytes_total: 120_000,
                observed_at_ms: t - 1.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 8000,
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
            "PLY-INT-14: ingress 增长但无新鲜 present/decode 不得直接 healthy+stable-serving"
        );
    });
}

#[test]
fn playback_phase_int15_twcc_stable_display_starved_not_connectivity_reconnect() {
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
    assert_cmds_have_no_reconnect(&cmds, "PLY-INT-15");
}

#[test]
fn playback_phase_int16_post_first_present_noise_must_resolve_or_enter_controlled_recovery() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t0 = wall_observed_ms();
    let _ = harness.apply(
        t0,
        ConnectionLifecycleStateFact::Connected,
        "none",
        700,
        |stats| {
            let t = t0;
            stats.session_phase = Some("steady".to_string());
            stats.transport_recovery_epoch = 55;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(t - 20.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 18.0);
            stats.video_renderer_stalled = Some(false);
            stats.video_present_submit_count_total = 3;
            stats.latest_video_frame_drop = Some(XbxEngineVideoFrameDropObservation {
                observation_id: 1,
                reason: "dropped".to_string(),
                stage: Some("present".to_string()),
                action: None,
                detail: Some("dropLate".to_string()),
                frame_rtp_timestamp: None,
                frame_seq: Some(3),
                frame_recovery_disposition: None,
                frame_unrecoverable_reason: None,
                frame_budget: None,
                observed_at_ms: t - 8.0,
                width: 1920,
                height: 1080,
                is_keyframe: false,
                queue_depth: 2,
            });
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 400_000,
                video_packet_count_total: 3_200,
                audio_bytes_total: 50_000,
                observed_at_ms: t - 1.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 8100,
                    source_event: "frame-observed".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "healthy".to_string(),
                        reason: None,
                        observed_at_ms: t,
                    },
                    observed_at_ms: t,
                });
        },
    );
    let t1 = t0 + 120.0;
    let _ = harness.apply(
        t1,
        ConnectionLifecycleStateFact::Connected,
        "none",
        710,
        |stats| {
            let t = t1;
            stats.latest_video_host_present_time_ms = Some(t - 15.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 12.0);
            stats.latest_video_frame_drop = None;
        },
    );
    harness.with_stats(|stats| {
        let ledger = stats.latest_recovery_decision_ledger.as_ref();
        assert!(
            ledger.is_some(),
            "PLY-INT-16: 短窗波动后应产生可观测决策（非无判决）"
        );
    });
}

#[test]
fn playback_phase_int17_stable_serving_does_not_regress_to_priming_on_milestone_monotonicity() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t1 = wall_observed_ms();
    let _ = harness.apply(
        t1,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        100,
        |stats| {
            stats.transport_recovery_epoch = 30;
            stats.session_phase = Some("recovering".to_string());
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 30;
            stats.latest_video_host_present_time_ms = Some(t1 - 240.0);
            stats.latest_video_decode_ok_time_ms = Some(t1 - 170.0);
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: None,
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 140_000,
                video_packet_count_total: 1000,
                audio_bytes_total: 36_000,
                observed_at_ms: t1,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 10,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                        observed_at_ms: t1,
                    },
                    observed_at_ms: t1,
                });
        },
    );
    let t2 = t1 + 40.0;
    let _ = harness.apply(
        t2,
        ConnectionLifecycleStateFact::Connected,
        "none",
        100,
        |stats| {
            stats.transport_recovery_epoch = 30;
            stats.session_phase = Some("steady".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(t2 - 18.0);
            stats.latest_video_decode_ok_time_ms = Some(t2 - 12.0);
            stats.video_anchor_clean_epoch = Some(stats.transport_recovery_epoch);
            stats.video_anchor_clean_observed_at_ms = Some(t2 - 15.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.observed_at_ms = t2;
                timeline.chain.observed_at_ms = t2;
            }
        },
    );
    harness.with_stats(|stats| {
        let s = stats.video_owner_state.as_deref();
        assert!(
            matches!(s, Some("stable-serving" | "degraded-serving")),
            "PLY-INT-17 phase2 got={s:?}"
        );
    });
    let t3 = t2 + 80.0;
    let _ = harness.apply(
        t3,
        ConnectionLifecycleStateFact::Connected,
        "none",
        110,
        |stats| {
            stats.transport_recovery_epoch = 30;
            stats.session_phase = Some("steady".to_string());
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(t3 - 12.0);
            stats.latest_video_decode_ok_time_ms = Some(t3 - 10.0);
            stats.latest_video_packet_arrival_time_ms = Some(t3 - 2.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(stats.transport_recovery_epoch);
            stats.video_anchor_clean_observed_at_ms = Some(t3 - 14.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: None,
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 145_000,
                video_packet_count_total: 1_050,
                audio_bytes_total: 36_500,
                observed_at_ms: t3 - 1.5,
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
                        observed_at_ms: t3,
                    },
                    observed_at_ms: t3,
                });
        },
    );
    harness.with_stats(|stats| {
        let s = stats.video_owner_state.as_deref();
        assert!(
            matches!(s, Some("stable-serving" | "degraded-serving")),
            "PLY-INT-17 phase3 got={s:?}"
        );
        assert_ne!(s, Some("priming"), "PLY-INT-17");
        assert_ne!(s, Some("seeking-anchor"), "PLY-INT-17");
    });
}

#[test]
fn playback_phase_int18_owner_supply_state_flip_requires_timeline_anchor_evidence() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t0 = wall_observed_ms();
    let _ = harness.apply(
        t0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryKeyframe",
        900,
        |stats| {
            let t = t0;
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 120;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 220;
            stats.latest_video_host_present_time_ms = Some(t - 1_600.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 400.0);
            stats.video_renderer_stalled = Some(true);
            stats.video_anchor_clean_epoch = Some(120);
            stats.video_anchor_clean_observed_at_ms = Some(t - 300.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-keyframe-submitted".to_string());
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
                    observation_id: 8200,
                    source_event: "frame-await-recovery-keyframe".to_string(),
                    gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
                        state: "open".to_string(),
                        sequence: Some(2),
                        frame_rtp_timestamp: Some(100),
                        frame_importance: Some("delta".to_string()),
                        observed_at_ms: t - 5.0,
                    }),
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
        let r = stats.video_owner_reason.as_deref().unwrap_or("");
        assert!(
            r.contains("supply")
                || r.contains("transport")
                || r.contains("anchor")
                || r.contains("display")
                || r.contains("steady"),
            "PLY-INT-18: owner reason 应可复盘（含 supply/transport/anchor 等证据域），got={r:?}"
        );
    });
}
