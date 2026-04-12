use super::super::{RecoveryCoordinator, RecoveryOwnerSignal};
use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationReason};
use crate::transport::rtc::recovery::runtime_state::unix_now_ms;
use crate::XbxEngineMediaRuntimeStats;
use crate::XbxEngineVideoTrackStatus;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use xbxengine_protocol::{XbxEngineTargetTypeDto, XbxEngineTransportStateDto};

use super::harness::{make_test_nack_observation, test_escalation_controller};

#[test]
fn transport_await_hard_fallback_does_not_inherit_timeout_window_after_decoder_reset_and_short_healthy_reentry(
) {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
    stats.transport_recovery_epoch = 33;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.video_renderer_stalled = Some(true);
    stats.latest_video_host_present_time_ms = Some(now_ms - 8_000.0);
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 320;
    stats.inbound_primary_video_bytes_total = 64_000;
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 96_000,
        video_packet_count_total: 400,
        audio_bytes_total: 4_000,
        observed_at_ms: now_ms,
    });
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_video_decoder_reset_time_ms = Some(now_ms + 180.0);
    });
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 5_000.0,
        },
        &shared_stats,
    );

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.video_anchor_clean_epoch = Some(33);
        stats.video_anchor_clean_observed_at_ms = Some(now_ms + 5_060.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-keyframe-submitted".to_string());
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 3301,
            source_event: "frame-observed".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                chain_break_evidence: None,

                observed_at_ms: now_ms + 5_060.0,
            },
            observed_at_ms: now_ms + 5_060.0,
        });
        stats.video_renderer_stalled = Some(false);
        stats.video_decoder_stalled = Some(false);
        stats.latest_video_host_present_time_ms = Some(now_ms + 5_060.0);
    });
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 5_080.0,
        },
        &shared_stats,
    );

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.video_anchor_clean_epoch = None;
        stats.video_anchor_clean_observed_at_ms = None;
        stats.video_anchor_clean_source_event = None;
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 3302,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                chain_break_evidence: None,

                observed_at_ms: now_ms + 5_240.0,
            },
            observed_at_ms: now_ms + 5_240.0,
        });
        stats.video_renderer_stalled = Some(true);
        stats.video_decoder_stalled = Some(false);
        stats.latest_video_host_present_time_ms = Some(now_ms - 10_000.0);
    });
    let reentry = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 5_240.0,
        },
        &shared_stats,
    );
    assert_ne!(
        reentry.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
    let fallback = RuntimeStatsSink::read_shared(&shared_stats, |stats| {
        (
            stats.recovery_hard_fallback_timer_ms,
            stats.recovery_hard_fallback_trigger_reason.clone(),
        )
    })
    .unwrap_or((None, None));
    assert!(
        fallback.0.is_some_and(|timer_ms| timer_ms < 800.0),
        "hard fallback timer should restart after short healthy reentry, got {:?}",
        fallback.0
    );
    assert_ne!(
        fallback.1.as_deref(),
        Some("transportAwaitRecoveryKeyframeTimeout")
    );
}

#[test]
fn transport_await_hard_fallback_timeout_can_enter_decoder_reset_without_current_window_attempt() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 32;
    stats.video_renderer_stalled = Some(true);
    stats.latest_video_host_present_time_ms = Some(now_ms - 6_000.0);
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 1_600.0,
        },
        &shared_stats,
    );
    let timeout = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 2_700.0,
        },
        &shared_stats,
    );
    assert_ne!(
        timeout.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
    assert_ne!(
        timeout.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
    assert_eq!(timeout.decision.action, RecoveryAction::RequestDecoderReset);
}

#[test]
fn transport_await_hard_fallback_keeps_connected_ingress_local_when_decoder_reset_path_exhausted() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
    stats.transport_recovery_epoch = 41;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.video_renderer_stalled = Some(true);
    stats.latest_video_host_present_time_ms = Some(now_ms - 9_000.0);
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 320;
    stats.inbound_primary_video_bytes_total = 12_000;
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(2560),
        video_height: Some(1440),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 42_000,
        video_packet_count_total: 120,
        audio_bytes_total: 2_100,
        observed_at_ms: now_ms,
    });
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    // 先把 decoder reset 预算耗尽。
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 200.0,
        },
        &shared_stats,
    );

    // 用一次显式 healthy clean anchor 把 hard fallback 计时窗口重置，
    // 让后续 timeout 窗口内“不存在 decoder reset 尝试”。
    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.video_anchor_clean_epoch = Some(41);
        stats.video_anchor_clean_observed_at_ms = Some(now_ms + 260.0);
        stats.video_anchor_clean_source_event = Some("chain-clean-keyframe-submitted".to_string());
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1201,
            source_event: "frame-observed".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "healthy".to_string(),
                reason: None,
                chain_break_evidence: None,

                observed_at_ms: now_ms + 260.0,
            },
            observed_at_ms: now_ms + 260.0,
        });
        stats.video_renderer_stalled = Some(false);
        stats.latest_video_host_present_time_ms = Some(now_ms + 260.0);
    });
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 300.0,
        },
        &shared_stats,
    );

    // 回到 Connected + ingress 持续，且仍有新输出推进。
    // 该场景应继续留在本地恢复链（cooldown），不应立即升级到 reconnect。
    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.video_anchor_clean_epoch = None;
        stats.video_anchor_clean_observed_at_ms = None;
        stats.video_anchor_clean_source_event = None;
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1202,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("streamThinStall".to_string()),
                chain_break_evidence: None,

                observed_at_ms: now_ms + 6_900.0,
            },
            observed_at_ms: now_ms + 6_900.0,
        });
        stats.video_renderer_stalled = Some(true);
        stats.latest_video_host_present_time_ms = Some(now_ms + 6_950.0);
        stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(2560),
            video_height: Some(1440),
            mime_type: Some("video/H264".to_string()),
            transport_state: XbxEngineTransportStateDto::Connected,
            video_bytes_total: 88_000,
            video_packet_count_total: 300,
            audio_bytes_total: 4_200,
            observed_at_ms: now_ms + 6_900.0,
        });
        stats.inbound_primary_video_bytes_total = 48_000;
    });
    let timeout = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 7_000.0,
        },
        &shared_stats,
    );
    assert_ne!(
        timeout.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
    assert_eq!(
        timeout.decision.action,
        RecoveryAction::CoalescedKeyframeInFlight
    );
}

#[test]
fn transport_await_hard_fallback_does_not_treat_ingress_without_output_as_local_progress() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
    stats.transport_recovery_epoch = 51;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.video_renderer_stalled = Some(true);
    stats.latest_video_host_present_time_ms = Some(now_ms - 6_000.0);
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 200.0,
        },
        &shared_stats,
    );

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 9201,
                reason: "transportAwaitRecoveryKeyframe".to_string(),
                action: "cooldownSuppressed".to_string(),
                recovery_stage: "rebuilding-supply".to_string(),
                recovery_chain_value: "anchor".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "transport-await-window".to_string(),
                observed_at_ms: now_ms + 260.0,
            });
        stats.latest_video_decoder_reset_time_ms = Some(now_ms + 220.0);
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1203,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("awaitingRecoveryKeyframe".to_string()),
                chain_break_evidence: None,

                observed_at_ms: now_ms + 6_900.0,
            },
            observed_at_ms: now_ms + 6_900.0,
        });
        stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: XbxEngineTransportStateDto::Connected,
            video_bytes_total: 96_000,
            video_packet_count_total: 340,
            audio_bytes_total: 4_600,
            observed_at_ms: now_ms + 6_900.0,
        });
        stats.inbound_primary_video_bytes_total = 64_000;
        // 明确保持“connected but unrecoverable”：仍在收包，但没有任何新输出推进。
        stats.latest_video_host_present_time_ms = Some(now_ms - 6_000.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 6_000.0);
    });

    let timeout = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 7_000.0,
        },
        &shared_stats,
    );
    // hard fallback 超时后：存在 decoder reset 时间证据时可能优先 `RequestDecoderReset`，
    // 仍为本地恢复链（非 reconnect）；与「ingress 无输出不得冒充 local progress」不矛盾。
    assert!(
        matches!(
            timeout.decision.action,
            RecoveryAction::RequestKeyframe | RecoveryAction::RequestDecoderReset
        ),
        "unexpected action {:?}",
        timeout.decision.action
    );
}

#[test]
fn transport_await_hard_fallback_decoder_reset_budget_exhaustion_upgrades_to_reconnect() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
    stats.transport_recovery_epoch = 54;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.video_renderer_stalled = Some(true);
    stats.video_decoder_stalled = Some(true);
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 360;
    stats.inbound_primary_video_bytes_total = 64_000;
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 196_000,
        video_packet_count_total: 920,
        audio_bytes_total: 6_800,
        observed_at_ms: now_ms,
    });
    stats.latest_video_host_present_time_ms = Some(now_ms - 8_000.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 8_000.0);
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.transport_recovery_epoch_at_last_escalation = 54;
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 5401,
                reason: "transportAwaitRecoveryKeyframe".to_string(),
                action: "requestDecoderReset".to_string(),
                recovery_stage: "rebuilding-supply".to_string(),
                recovery_chain_value: "anchor".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "transport-await-window".to_string(),
                observed_at_ms: now_ms + 180.0,
            });
        stats.latest_video_decoder_reset_time_ms = Some(now_ms + 220.0);
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 5402,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("awaitingRecoveryKeyframe".to_string()),
                chain_break_evidence: None,

                observed_at_ms: now_ms + 6_900.0,
            },
            observed_at_ms: now_ms + 6_900.0,
        });
        stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/H264".to_string()),
            transport_state: XbxEngineTransportStateDto::Connected,
            video_bytes_total: 248_000,
            video_packet_count_total: 1_040,
            audio_bytes_total: 7_200,
            observed_at_ms: now_ms + 6_900.0,
        });
        stats.inbound_primary_video_bytes_total = 96_000;
        stats.latest_video_host_present_time_ms = Some(now_ms - 8_000.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 8_000.0);
    });

    let timeout = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 7_000.0,
        },
        &shared_stats,
    );
    assert_eq!(
        timeout.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
}

#[test]
fn transport_await_hard_fallback_keeps_local_when_decode_progress_is_fresh_without_present() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
    stats.transport_recovery_epoch = 52;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.video_renderer_stalled = Some(true);
    stats.video_decoder_stalled = Some(false);
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 360;
    stats.inbound_primary_video_bytes_total = 36_000;
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 160_000,
        video_packet_count_total: 720,
        audio_bytes_total: 5_200,
        observed_at_ms: now_ms,
    });
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 200.0,
        },
        &shared_stats,
    );

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 9301,
                reason: "transportAwaitRecoveryKeyframe".to_string(),
                action: "cooldownSuppressed".to_string(),
                recovery_stage: "rebuilding-supply".to_string(),
                recovery_chain_value: "anchor".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "transport-await-window".to_string(),
                observed_at_ms: now_ms + 260.0,
            });
        stats.latest_video_decoder_reset_time_ms = Some(now_ms + 220.0);
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1204,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("awaitingRecoveryKeyframe".to_string()),
                chain_break_evidence: None,

                observed_at_ms: now_ms + 6_900.0,
            },
            observed_at_ms: now_ms + 6_900.0,
        });
        if let Some(track) = stats.latest_video_track_status.as_mut() {
            track.observed_at_ms = now_ms + 6_900.0;
            track.video_bytes_total += 28_000;
            track.video_packet_count_total += 140;
        }
        stats.latest_video_host_present_time_ms = Some(now_ms - 6_000.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms + 6_950.0);
    });

    let timeout = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 7_000.0,
        },
        &shared_stats,
    );
    assert_eq!(timeout.decision.action, RecoveryAction::CooldownSuppressed);
}

#[test]
fn transport_await_hard_fallback_does_not_treat_nonidr_packet_seen_as_local_decode_progress() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
    stats.transport_recovery_epoch = 54;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.video_renderer_stalled = Some(true);
    stats.video_decoder_stalled = Some(true);
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 360;
    stats.inbound_primary_video_bytes_total = 52_000;
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 188_000,
        video_packet_count_total: 860,
        audio_bytes_total: 6_100,
        observed_at_ms: now_ms,
    });
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 200.0,
        },
        &shared_stats,
    );

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 9_401,
                reason: "transportAwaitRecoveryKeyframe".to_string(),
                action: "cooldownSuppressed".to_string(),
                recovery_stage: "rebuilding-supply".to_string(),
                recovery_chain_value: "anchor".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "transport-await-window".to_string(),
                observed_at_ms: now_ms + 260.0,
            });
        stats.latest_video_decoder_reset_time_ms = Some(now_ms + 220.0);
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1_205,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("awaitingRecoveryKeyframe".to_string()),
                chain_break_evidence: None,

                observed_at_ms: now_ms + 6_900.0,
            },
            observed_at_ms: now_ms + 6_900.0,
        });
        if let Some(track) = stats.latest_video_track_status.as_mut() {
            track.observed_at_ms = now_ms + 6_900.0;
            track.video_bytes_total += 32_000;
            track.video_packet_count_total += 150;
        }
        stats.latest_video_host_present_time_ms = Some(now_ms - 6_000.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms + 6_950.0);
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 9_402,
                request_reason: Some("transportAwaitRecoveryKeyframe".to_string()),
                request_kind: Some("pli".to_string()),
                status: "packet-seen".to_string(),
                status_detail: None,
                requested_at_ms: now_ms + 6_700.0,
                sent_at_ms: Some(now_ms + 6_710.0),
                deadline_at_ms: Some(now_ms + 7_200.0),
                transport_detail: None,
                first_video_packet_at_ms: None,
                first_video_packet_rtp_timestamp: None,
                first_video_packet_is_keyframe: None,
                first_keyframe_packet_at_ms: Some(now_ms + 6_940.0),
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: Some(77_777),
                response_frame_seq: None,
                response_verdict: Some("late".to_string()),
                lifecycle_phase: None,
            });
        stats.latest_h264_inspection_observation =
            Some(crate::XbxEngineH264InspectionObservation {
                observation_id: 9_403,
                frame_rtp_timestamp: Some(77_777),
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
                sample_width: None,
                sample_height: None,
                bootstrap_ready: false,
                bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
                admission_accepted: true,
                observed_at_ms: now_ms + 6_960.0,
            });
    });

    let timeout = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 7_000.0,
        },
        &shared_stats,
    );
    assert_eq!(timeout.decision.action, RecoveryAction::RequestKeyframe);
}

#[test]
fn transport_await_hard_fallback_upgrades_to_decoder_reset_after_decode_progress_turns_stale() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
    stats.transport_recovery_epoch = 53;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.video_renderer_stalled = Some(true);
    stats.video_decoder_stalled = Some(false);
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 360;
    stats.inbound_primary_video_bytes_total = 44_000;
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 188_000,
        video_packet_count_total: 860,
        audio_bytes_total: 6_100,
        observed_at_ms: now_ms,
    });
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 200.0,
        },
        &shared_stats,
    );

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 9401,
                reason: "transportAwaitRecoveryKeyframe".to_string(),
                action: "cooldownSuppressed".to_string(),
                recovery_stage: "rebuilding-supply".to_string(),
                recovery_chain_value: "anchor".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "transport-await-window".to_string(),
                observed_at_ms: now_ms + 260.0,
            });
        stats.latest_video_decoder_reset_time_ms = Some(now_ms + 220.0);
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1205,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("awaitingRecoveryKeyframe".to_string()),
                chain_break_evidence: None,

                observed_at_ms: now_ms + 6_900.0,
            },
            observed_at_ms: now_ms + 6_900.0,
        });
        if let Some(track) = stats.latest_video_track_status.as_mut() {
            track.observed_at_ms = now_ms + 6_900.0;
            track.video_bytes_total += 24_000;
            track.video_packet_count_total += 120;
        }
        stats.latest_video_host_present_time_ms = Some(now_ms - 6_000.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms + 6_950.0);
    });

    let local = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 7_000.0,
        },
        &shared_stats,
    );
    assert_eq!(local.decision.action, RecoveryAction::CooldownSuppressed);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        if let Some(track) = stats.latest_video_track_status.as_mut() {
            track.observed_at_ms = now_ms + 7_180.0;
            track.video_bytes_total += 26_000;
            track.video_packet_count_total += 130;
        }
        stats.latest_video_host_present_time_ms = Some(now_ms - 6_000.0);
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 6_000.0);
        stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
            observation_id: 1206,
            source_event: "frame-await-recovery-keyframe".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".to_string(),
                reason: Some("awaitingRecoveryKeyframe".to_string()),
                chain_break_evidence: None,

                observed_at_ms: now_ms + 7_180.0,
            },
            observed_at_ms: now_ms + 7_180.0,
        });
    });

    let escalated = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 7_200.0,
        },
        &shared_stats,
    );
    assert_eq!(
        escalated.decision.action,
        RecoveryAction::RequestDecoderReset
    );
}

#[test]
fn transport_await_hard_fallback_resets_on_non_await_reason() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 31;
    stats.video_renderer_stalled = Some(true);
    stats.latest_video_host_present_time_ms = Some(now_ms - 6_000.0);
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::AdapterThinStream,
            reason_label: "adapterThinStream".to_string(),
            observed_at_ms: now_ms + 1_200.0,
        },
        &shared_stats,
    );
    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 2_800.0,
        },
        &shared_stats,
    );
    let fallback = RuntimeStatsSink::read_shared(&shared_stats, |stats| {
        (
            stats.recovery_hard_fallback_timer_ms,
            stats.recovery_hard_fallback_timer_reset_reason.clone(),
        )
    })
    .unwrap_or((None, None));
    assert!(fallback.0.is_some_and(|timer_ms| timer_ms <= 1_600.0));
    assert!(fallback.1.is_none());
}

#[test]
fn cooldown_suppressed_cannot_linger_when_connected_track_attached_but_no_present_decode_progress()
{
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_target_type = Some(XbxEngineTargetTypeDto::Cloud);
    stats.transport_recovery_epoch = 51;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.video_renderer_stalled = Some(true);
    stats.video_decoder_stalled = Some(true);
    stats.host_no_pending_pressure_level = Some("critical".to_string());
    stats.host_no_pending_streak = 400;
    stats.inbound_primary_video_bytes_total = 24_000;
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 128_000,
        video_packet_count_total: 640,
        audio_bytes_total: 4_800,
        observed_at_ms: now_ms,
    });
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::RequestKeyframe);

    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 200.0,
        },
        &shared_stats,
    );
    assert_eq!(
        second.decision.action,
        RecoveryAction::CoalescedKeyframeInFlight
    );

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: 9101,
                reason: "transportAwaitRecoveryKeyframe".to_string(),
                action: "cooldownSuppressed".to_string(),
                recovery_stage: "rebuilding-supply".to_string(),
                recovery_chain_value: "anchor".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "transport-await-window".to_string(),
                observed_at_ms: now_ms + 260.0,
            });
        stats.latest_video_decoder_reset_time_ms = Some(now_ms + 220.0);
    });

    let timeout = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
            reason_label: "transportAwaitRecoveryKeyframe".to_string(),
            observed_at_ms: now_ms + 7_000.0,
        },
        &shared_stats,
    );
    assert_eq!(timeout.decision.action, RecoveryAction::RequestDecoderReset);
}

#[test]
fn new_transport_recovery_epoch_clears_severe_deadline_idle_timeout_shortcut() {
    let shared_stats = Mutex::new(XbxEngineMediaRuntimeStats {
        transport_recovery_epoch: 61,
        ..Default::default()
    });
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: unix_now_ms(),
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::CooldownSuppressed);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.transport_recovery_epoch = 62;
    });
    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::AdapterIdleTimeout,
            reason_label: "adapterIdleTimeout".to_string(),
            observed_at_ms: unix_now_ms() + 20.0,
        },
        &shared_stats,
    );
    assert_ne!(
        second.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
}

#[test]
fn transport_severe_deadline_requires_fresh_second_hit_before_reconnect() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 61;
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::CooldownSuppressed);

    std::thread::sleep(Duration::from_millis(420));
    let delayed = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: now_ms + 420.0,
        },
        &shared_stats,
    );
    assert_eq!(delayed.decision.action, RecoveryAction::CooldownSuppressed);

    let fresh_second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: now_ms + 460.0,
        },
        &shared_stats,
    );
    assert_eq!(
        fresh_second.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
}

#[test]
fn transport_severe_deadline_with_fresh_media_output_is_absorbed_before_reconnect() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 77;
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.video_present_fps = 60.0;
    stats.latest_video_host_present_time_ms = Some(now_ms - 40.0);
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 45.0);
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/H264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 1_200_000,
        video_packet_count_total: 6_400,
        audio_bytes_total: 64_000,
        observed_at_ms: now_ms - 20.0,
    });
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(first.decision.action, RecoveryAction::CooldownSuppressed);

    let second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: now_ms + 80.0,
        },
        &shared_stats,
    );
    assert_eq!(second.decision.action, RecoveryAction::CooldownSuppressed);
    assert_ne!(
        second.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
}

#[test]
fn transport_expired_deadline_window_resets_after_large_gap() {
    let observed_at_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.session_target_type = Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud);
    stats.latest_video_nack_observation = Some(make_test_nack_observation(
        "expiredDeadline",
        "delta",
        2,
        observed_at_ms,
    ));
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let first = coordinator.on_reason_with_runtime_stats(
        VideoEscalationReason::TransportExpiredDeadline,
        &shared_stats,
    );
    assert_eq!(first.action, RecoveryAction::CooldownSuppressed);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        if let Some(observation) = stats.latest_video_nack_observation.as_mut() {
            observation.observation_id = 2;
            observation.observed_at_ms += 500.0;
        }
    });
    let second = coordinator.on_reason_with_runtime_stats(
        VideoEscalationReason::TransportExpiredDeadline,
        &shared_stats,
    );
    assert_eq!(second.action, RecoveryAction::CooldownSuppressed);

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        if let Some(observation) = stats.latest_video_nack_observation.as_mut() {
            observation.observation_id = 3;
            observation.observed_at_ms += 40.0;
        }
    });
    let third = coordinator.on_reason_with_runtime_stats(
        VideoEscalationReason::TransportExpiredDeadline,
        &shared_stats,
    );
    assert_eq!(third.action, RecoveryAction::CooldownSuppressed);
}

#[test]
fn reconnect_budget_is_released_after_new_epoch_for_transport_severe_deadline() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 71;
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let _ = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    let first_reconnect = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: now_ms + 20.0,
        },
        &shared_stats,
    );
    assert_eq!(
        first_reconnect.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.transport_recovery_epoch = 72;
        stats.transport_recovery_epoch_at_last_escalation = 72;
        stats.latest_video_escalation_observation =
            Some(crate::XbxEngineVideoEscalationObservation {
                observation_id: first_reconnect.decision.observation_id,
                reason: "transportSevereDeadline".to_string(),
                action: "requestReconnectCandidate".to_string(),
                recovery_stage: "active-recovery".to_string(),
                recovery_chain_value: "transport".to_string(),
                recovery_failure_cost: "high".to_string(),
                recovery_window_source: "lifecycle-window".to_string(),
                observed_at_ms: now_ms + 20.0,
            });
    });

    let exhausted = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: now_ms + 40.0,
        },
        &shared_stats,
    );
    assert_eq!(
        exhausted.decision.action,
        RecoveryAction::CooldownSuppressed
    );

    let epoch_reset_first = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: now_ms + 80.0,
        },
        &shared_stats,
    );
    assert_eq!(
        epoch_reset_first.decision.action,
        RecoveryAction::CooldownSuppressed
    );

    RuntimeStatsSink::update_shared(&shared_stats, |stats| {
        stats.transport_recovery_epoch = 73;
        stats.transport_recovery_epoch_at_last_escalation = 72;
    });

    let epoch_reset_second = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: now_ms + 100.0,
        },
        &shared_stats,
    );
    assert_eq!(
        epoch_reset_second.decision.action,
        RecoveryAction::CooldownSuppressed
    );

    let epoch_reset_third = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: now_ms + 120.0,
        },
        &shared_stats,
    );
    assert_eq!(
        epoch_reset_third.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
}

#[test]
fn transport_recovered_late_does_not_inherit_severe_deadline_reconnect_counter() {
    let now_ms = unix_now_ms();
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 81;
    let shared_stats = Mutex::new(stats);
    let mut coordinator = RecoveryCoordinator::new(
        test_escalation_controller(120, 1, 1),
        Instant::now() - Duration::from_secs(3),
        Duration::from_millis(800),
    );

    let severe = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportSevereDeadline,
            reason_label: "transportSevereDeadline".to_string(),
            observed_at_ms: now_ms,
        },
        &shared_stats,
    );
    assert_eq!(severe.decision.action, RecoveryAction::CooldownSuppressed);

    let recovered_late = coordinator.propose_from_owner_signal(
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::TransportRecoveredLate,
            reason_label: "transportRecoveredLate".to_string(),
            observed_at_ms: now_ms + 20.0,
        },
        &shared_stats,
    );
    assert_ne!(
        recovered_late.decision.action,
        RecoveryAction::RequestReconnectCandidate
    );
    assert!(matches!(
        recovered_late.decision.action,
        RecoveryAction::RequestKeyframe | RecoveryAction::CooldownSuppressed
    ));
}

#[test]
fn coordinator_burst_rollback_warranted_covers_transport_await_suppress_pairs() {
    assert!(RecoveryCoordinator::coordinator_burst_rollback_warranted(
        RecoveryAction::RequestKeyframe,
        RecoveryAction::WaitForBurst,
    ));
    assert!(!RecoveryCoordinator::coordinator_burst_rollback_warranted(
        RecoveryAction::RequestKeyframe,
        RecoveryAction::CooldownSuppressed,
    ));
    assert!(RecoveryCoordinator::coordinator_burst_rollback_warranted(
        RecoveryAction::RequestKeyframe,
        RecoveryAction::CoalescedDecoderResetInFlight,
    ));
    assert!(RecoveryCoordinator::coordinator_burst_rollback_warranted(
        RecoveryAction::RequestDecoderReset,
        RecoveryAction::CoalescedDecoderResetInFlight,
    ));
    assert!(!RecoveryCoordinator::coordinator_burst_rollback_warranted(
        RecoveryAction::WaitForBurst,
        RecoveryAction::WaitForBurst,
    ));
}
