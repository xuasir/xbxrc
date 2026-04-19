//! RFC PLY-INT-01 .. PLY-INT-06

use super::super::harness::RecoveryIntegrationHarness;
use super::common::{
    assert_cmds_have_no_reconnect, fill_twcc_stable_local_feedback, wall_observed_ms,
};
use crate::api::backend::XbxEngineVideoFrameDropObservation;
use crate::transport::rtc::facts::{ConnectionLifecycleStateFact, TransportCommand};

#[test]
fn playback_phase_int01_no_first_present_must_not_promote_stable_serving() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t = wall_observed_ms();
    let _ = harness.apply(
        t,
        ConnectionLifecycleStateFact::Connected,
        "none",
        10,
        |stats| {
            stats.session_phase = Some("steady".to_string());
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 6;
            stats.host_display_tick_epoch = 6;
            stats.video_present_epoch = 0;
            stats.video_present_submit_count_total = 0;
            stats.host_cadence_phase = Some("priming".to_string());
            stats.latest_video_host_present_time_ms = None;
            stats.latest_video_decode_ok_time_ms = None;
            stats.video_renderer_stalled = Some(false);
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 12_000,
                video_packet_count_total: 90,
                audio_bytes_total: 800,
                observed_at_ms: t,
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
            "PLY-INT-01"
        );
    });
}

#[test]
fn playback_phase_int02_decode_overflow_with_fresh_present_must_not_emit_reconnect_candidate() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t = wall_observed_ms();
    let cmds = harness.apply(
        t,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        120,
        |stats| {
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 50;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 120;
            stats.latest_video_host_present_time_ms = Some(t - 14.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 70.0);
            stats.latest_video_packet_arrival_time_ms = Some(t - 4.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.inbound_primary_video_bytes_total = 520_000;
            stats.latest_video_frame_drop = Some(XbxEngineVideoFrameDropObservation {
                observation_id: 901,
                reason: "dropped".to_string(),
                stage: Some("decode".to_string()),
                action: None,
                detail: Some("outputQueueOverflow".to_string()),
                frame_rtp_timestamp: None,
                frame_seq: Some(12),
                frame_recovery_disposition: None,
                frame_unrecoverable_reason: None,
                frame_budget: None,
                observed_at_ms: t - 6.0,
                width: 1920,
                height: 1080,
                is_keyframe: false,
                queue_depth: 12,
            });
            stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                state: "remoteTrackAttached".to_string(),
                video_width: Some(1920),
                video_height: Some(1080),
                mime_type: Some("video/H264".to_string()),
                transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                video_bytes_total: 520_000,
                video_packet_count_total: 4_200,
                audio_bytes_total: 70_000,
                observed_at_ms: t - 3.0,
            });
            stats.latest_video_timeline_observation =
                Some(crate::XbxEngineVideoTimelineObservation {
                    observation_id: 902,
                    source_event: "frame-await-recovery-anchor".to_string(),
                    gap: None,
                    frame: None,
                    chain: crate::XbxEngineVideoTimelineChainSnapshot {
                        state: "recovering".to_string(),
                        reason: Some("transportAwaitRecoveryAnchor".to_string()),
                        chain_break_evidence: None,

                        observed_at_ms: t - 3.0,
                    },
                    observed_at_ms: t - 3.0,
                });
        },
    );
    assert_cmds_have_no_reconnect(&cmds, "PLY-INT-02");
}

#[test]
fn playback_phase_int03_repeated_stale_present_pulse_must_not_reconnect_storm() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let mut reconnect_hits = 0u32;
    for i in 0..5u32 {
        let t = wall_observed_ms() + i as f64 * 25.0;
        let cmds = harness.apply(
            t,
            ConnectionLifecycleStateFact::Connected,
            "none",
            200 + u64::from(i),
            |stats| {
                stats.session_phase = Some("steady".to_string());
                stats.transport_recovery_epoch = 40;
                stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
                stats.host_no_pending_pressure_level = Some("critical".to_string());
                stats.host_no_pending_streak = 200 + i * 10;
                stats.latest_video_host_present_time_ms = Some(t - 900.0 - f64::from(i * 50));
                stats.latest_video_decode_ok_time_ms = Some(t - 500.0);
                stats.video_renderer_stalled = Some(false);
                stats.latest_video_packet_arrival_time_ms = Some(t - 2.0);
                stats.inbound_primary_video_bytes_total = 800_000;
                stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
                    state: "remoteTrackAttached".to_string(),
                    video_width: Some(1920),
                    video_height: Some(1080),
                    mime_type: Some("video/H264".to_string()),
                    transport_state: xbxengine_protocol::XbxEngineTransportStateDto::Connected,
                    video_bytes_total: 200_000 + u64::from(i) * 2_000,
                    video_packet_count_total: 2_000 + u64::from(i) * 20,
                    audio_bytes_total: 40_000,
                    observed_at_ms: t - 1.0,
                });
                stats.latest_video_timeline_observation =
                    Some(crate::XbxEngineVideoTimelineObservation {
                        observation_id: 300 + u64::from(i),
                        source_event: "frame-complete-candidate".to_string(),
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
        reconnect_hits += cmds
            .iter()
            .filter(|c| matches!(c, TransportCommand::RequestReconnectCandidate { .. }))
            .count() as u32;
    }
    assert_eq!(
        reconnect_hits, 0,
        "PLY-INT-03: 连续 present 迟滞脉冲 + ingress 存活不得形成 reconnect 风暴"
    );
}

#[test]
fn playback_phase_int04_transport_await_family_hold_must_not_stick_forever_on_same_coalesce() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t0 = wall_observed_ms();
    let first = harness.apply(
        t0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        260,
        |stats| {
            let t = t0;
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 75;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 180;
            stats.latest_video_host_present_time_ms = Some(t - 1_600.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 420.0);
            stats.latest_video_packet_arrival_time_ms = Some(t - 2.0);
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
                observed_at_ms: t - 2.0,
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

                        observed_at_ms: t - 2.0,
                    },
                    observed_at_ms: t - 2.0,
                });
        },
    );
    assert!(
        first
            .iter()
            .any(|c| matches!(c, TransportCommand::RequestKeyframe { .. })),
        "PLY-INT-04: 首轮应能发出本地 keyframe 探测"
    );

    let t1 = t0 + 35.0;
    let second = harness.apply(
        t1,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        260,
        |stats| {
            let t = t1;
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 75;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("critical".to_string());
            stats.host_no_pending_streak = 220;
            stats.latest_video_host_present_time_ms = Some(t - 1_900.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 460.0);
            stats.latest_video_packet_arrival_time_ms = Some(t - 2.0);
            stats.video_decoder_stalled = Some(false);
            stats.video_renderer_stalled = Some(true);
            stats.latest_video_escalation_observation =
                Some(crate::XbxEngineVideoEscalationObservation {
                    observation_id: 52,
                    reason: "transportAwaitRecoveryAnchor".to_string(),
                    action: "requestKeyframe".to_string(),
                    recovery_stage: "rebuilding-supply".to_string(),
                    recovery_chain_value: "anchor".to_string(),
                    recovery_failure_cost: "high".to_string(),
                    recovery_window_source: "transport-await-window".to_string(),
                    observed_at_ms: t - 10.0,
                });
            if let Some(track) = stats.latest_video_track_status.as_mut() {
                track.video_bytes_total += 4_000;
                track.video_packet_count_total += 32;
                track.observed_at_ms = t - 2.0;
            }
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id = 52;
                timeline.source_event = "frame-await-recovery-anchor".to_string();
                timeline.chain.state = "recovering".to_string();
                timeline.chain.reason = Some("transportAwaitRecoveryAnchor".to_string());
                timeline.chain.observed_at_ms = t - 2.0;
                timeline.observed_at_ms = t - 2.0;
            }
        },
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("PLY-INT-04 ledger");
        assert_ne!(
            ledger.gate_result, "no-signal",
            "PLY-INT-04: 抑制态须有可观测 gate"
        );
    });
    assert!(
        second.is_empty()
            || second
                .iter()
                .any(|c| matches!(c, TransportCommand::RequestKeyframe { .. })),
        "PLY-INT-04: 第二拍应吸收重复或继续可解释动作: {second:?}"
    );

    // 第三拍：clean anchor 后不得仍锁在同一 transport-await 合流态（与墙钟对齐的进展合同）
    let t2 = t0 + 95.0;
    let _third = harness.apply(
        t2,
        ConnectionLifecycleStateFact::Connected,
        "none",
        262,
        |stats| {
            let t = t2;
            stats.transport_recovery_epoch = 75;
            stats.session_phase = Some("recovering".to_string());
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 20;
            stats.latest_video_host_present_time_ms = Some(t - 80.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 60.0);
            stats.video_renderer_stalled = Some(false);
            stats.video_anchor_clean_epoch = Some(75);
            stats.video_anchor_clean_observed_at_ms = Some(t - 70.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.observation_id = 53;
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.chain.observed_at_ms = t - 1.0;
                timeline.observed_at_ms = t - 1.0;
            }
        },
    );
    harness.with_stats(|stats| {
        let ledger = stats
            .latest_recovery_decision_ledger
            .as_ref()
            .expect("PLY-INT-04 post-anchor ledger");
        assert!(
            ledger.gate_result != "coalesced:keyframeInFlight"
                || ledger.unlock_reason.is_some()
                || stats.video_owner_state.as_deref() == Some("degraded-serving")
                || stats.video_owner_state.as_deref() == Some("stable-serving"),
            "PLY-INT-04: clean anchor + healthy chain 后须解锁或进入可服务态，ledger={ledger:?} owner={:?}",
            stats.video_owner_state
        );
    });
}

#[test]
fn playback_phase_int05_twcc_stable_with_display_starved_must_not_emit_connectivity_reconnect() {
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
    assert_cmds_have_no_reconnect(&cmds, "PLY-INT-05");
    harness.with_stats(|stats| {
        if let Some(ledger) = stats.latest_recovery_decision_ledger.as_ref() {
            assert_ne!(
                ledger.action_selected, "requestReconnectCandidate",
                "PLY-INT-05 ledger"
            );
        }
    });
}

#[test]
fn playback_phase_int06_clean_anchor_progress_must_exit_toward_serving_not_starved_only() {
    let mut harness =
        RecoveryIntegrationHarness::new(Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud));
    let t0 = wall_observed_ms();
    let _ = harness.apply(
        t0,
        ConnectionLifecycleStateFact::Connected,
        "transportAwaitRecoveryAnchor",
        280,
        |stats| {
            let t = t0;
            stats.session_phase = Some("recovering".to_string());
            stats.transport_recovery_epoch = 72;
            stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
            stats.host_no_pending_pressure_level = Some("normal".to_string());
            stats.host_no_pending_streak = 30;
            stats.latest_video_host_present_time_ms = Some(t - 240.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 170.0);
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
                observed_at_ms: t,
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

                        observed_at_ms: t,
                    },
                    observed_at_ms: t,
                });
        },
    );
    let t1 = t0 + 45.0;
    let _ = harness.apply(
        t1,
        ConnectionLifecycleStateFact::Connected,
        "none",
        280,
        |stats| {
            let t = t1;
            stats.transport_recovery_epoch = 72;
            stats.session_phase = Some("steady".to_string());
            stats.host_no_pending_streak = 0;
            stats.latest_video_host_present_time_ms = Some(t - 18.0);
            stats.latest_video_decode_ok_time_ms = Some(t - 12.0);
            stats.video_anchor_clean_epoch = Some(stats.transport_recovery_epoch);
            stats.video_anchor_clean_observed_at_ms = Some(t - 15.0);
            stats.video_anchor_clean_source_event =
                Some("chain-clean-anchor-submitted".to_string());
            if let Some(timeline) = stats.latest_video_timeline_observation.as_mut() {
                timeline.source_event = "frame-observed".to_string();
                timeline.chain.state = "healthy".to_string();
                timeline.chain.reason = None;
                timeline.observed_at_ms = t;
                timeline.chain.observed_at_ms = t;
            }
        },
    );
    harness.with_stats(|stats| {
        let s = stats.video_owner_state.as_deref();
        assert!(
            matches!(s, Some("stable-serving" | "degraded-serving")),
            "PLY-INT-06: clean anchor + 进展后应离开纯 supply-starved/rebuilding 卡死，got={s:?}"
        );
    });
}
