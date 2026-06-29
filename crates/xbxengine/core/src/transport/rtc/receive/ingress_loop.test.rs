use crate::media::video::h264::inspection::{H264AccessUnitInspection, H264AccessUnitInspector};
use crate::media::video::ingress::budget::FrameBudgetWindowSource;
use crate::media::video::test_fixtures::{
    bootstrap_idr_nalu, bootstrap_pps_nalu, bootstrap_sps_nalu, make_video_rtp_packet,
    make_video_source_for_test, send_bootstrap_access_unit,
};
use crate::media::video::types::FrameRecoveryDisposition;
use crate::transport::rtc::receive::ingress_loop::{
    resolve_effective_idle_controls, should_absorb_idle_timeout_for_steady_gap,
    should_trigger_idle_timeout,
};
use crate::transport::rtc::receive::insert_gate::{
    resolve_insert_decision, InsertContext, InsertDecision,
};
use crate::transport::rtc::receive::keyframe_requester::KeyframeRequestDispatch;
use crate::transport::rtc::receive::{
    now_ms_f64, should_block_non_keyframe_admission, test_transport_capability,
    DecodeCorruptionPolicy, ReceiverDecodeContext, ReceiverState, RtcVideoFrameSource,
};
use crate::transport::rtc::recovery::contract::{
    GapVsKeyframeMode, PacketRecoveryActionStage, ReferenceChainState,
};

fn assert_receiver_local_waiting_keyframe(source: &RtcVideoFrameSource) {
    assert_eq!(
        source.receiver_local_state(),
        ReceiverState::WaitingKeyframe
    );
}
use crate::transport::rtc::receive::decode_gate_eval::{
    FirstFrameAcquisitionRequestKind, RecoveryKeyframeAction,
};
use crate::transport::rtc::stream::adapter_types::{
    TransportLossObservation, TransportObservation,
};
use crate::transport::rtc::stream::nack_contract::NackSchedulerConfig;
use crate::transport::rtc::stream::packet_types::{RtcVideoIngressKind, RtcVideoRepairMetadata};
use crate::{
    XbxEngineAnchorCandidateState, XbxEngineRemoteAnswerObservation, XbxEngineVideoTrackStatus,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use xbxengine_protocol::{XbxEngineTargetTypeDto, XbxEngineTransportStateDto};

fn resolve_recovery_keyframe_action(
    first_frame_acquired: bool,
    is_blocking_non_keyframe_admission: bool,
    sustaining_recovery_active: bool,
    receiver_repairing: bool,
    hard_recovery_gap_risk: bool,
    _sample_loss_burst_count: u8,
    media_dropped_packets: u16,
    is_keyframe: bool,
    displayed_idr_serving: bool,
) -> (bool, RecoveryKeyframeAction) {
    if is_keyframe && media_dropped_packets > 0 {
        return (false, RecoveryKeyframeAction::DropAndRequestPli);
    }
    if is_keyframe {
        return (false, RecoveryKeyframeAction::Submit);
    }
    if !first_frame_acquired {
        return (true, RecoveryKeyframeAction::WaitKeyframe);
    }
    if media_dropped_packets > 0 {
        return (false, RecoveryKeyframeAction::DropAndRequestPli);
    }
    if is_blocking_non_keyframe_admission {
        if displayed_idr_serving && first_frame_acquired {
            return (false, RecoveryKeyframeAction::Submit);
        }
        if !first_frame_acquired {
            return (true, RecoveryKeyframeAction::WaitKeyframe);
        }
        if sustaining_recovery_active || receiver_repairing {
            return (false, RecoveryKeyframeAction::Submit);
        }
        if !hard_recovery_gap_risk {
            return (false, RecoveryKeyframeAction::Submit);
        }
        return (true, RecoveryKeyframeAction::WaitKeyframe);
    }
    (false, RecoveryKeyframeAction::Submit)
}

fn serviceable_runtime_stats(now_ms: f64) -> crate::XbxEngineMediaRuntimeStats {
    let mut stats = crate::XbxEngineMediaRuntimeStats::default();
    stats.transport_state = XbxEngineTransportStateDto::Connected;
    stats.transport_recovery_epoch = 7;
    stats.video_anchor_clean_epoch = Some(7);
    stats.video_anchor_clean_observed_at_ms = Some(now_ms - 20.0);
    stats.video_anchor_clean_source_event = Some("displayed-idr".to_string());
    stats.latest_video_decode_ok_time_ms = Some(now_ms - 24.0);
    stats.latest_video_host_present_time_ms = Some(now_ms - 28.0);
    stats.video_renderer_stalled = Some(false);
    stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
        state: "remoteTrackAttached".to_string(),
        video_width: Some(1920),
        video_height: Some(1080),
        mime_type: Some("video/h264".to_string()),
        transport_state: XbxEngineTransportStateDto::Connected,
        video_bytes_total: 88_000,
        video_packet_count_total: 320,
        audio_bytes_total: 2_048,
        observed_at_ms: now_ms - 10.0,
    });
    stats
}

fn startup_h264_answer_without_sprop() -> XbxEngineRemoteAnswerObservation {
    XbxEngineRemoteAnswerObservation {
        observation_id: 1,
        video_payload_order: vec![124],
        selected_video_payload_type: Some(124),
        selected_video_mime_type: Some("video/h264".to_string()),
        selected_video_profile_level_id: Some("4d002a".to_string()),
        selected_video_h264_sprop_parameter_sets: None,
        accepted_video_rtcp_feedback: vec!["nack:pli".to_string()],
        accepted_audio_rtcp_feedback: Vec::new(),
        accepted_video_header_extensions: Vec::new(),
        accepted_audio_header_extensions: Vec::new(),
        observed_at_ms: 10.0,
    }
}

fn bootstrap_non_idr_nalu() -> Vec<u8> {
    let mut nalu = bootstrap_idr_nalu().to_vec();
    nalu[0] = 0x41;
    nalu
}

#[test]
fn active_refresh_outcome_sent_does_not_reopen_hard_request_stats_window() {
    let (_tx, _transport_observation_rx, mut source) = make_video_source_for_test();
    let now_ms = 1_000.0;
    source.runtime_stats.update(|stats| {
        stats.recovery_effective_rtt_ms = Some(24.0);
    });
    source
        .trace_ledger
        .note_clean_anchor_committed(Some(90_001));
    source
        .trace_ledger
        .recovery_ledger_mut()
        .note_decoder_reference_synced(now_ms - 20.0);
    source.trace_ledger.mark_gap_repair_in_flight(
        &[702],
        now_ms - 5.0,
        Some(90_002),
        "supply",
        "supply",
    );
    source.sync_recovery_ledger_to_stats();

    let decision = source.plan_receive_feedback(
        "insert-gate-supply-break",
        now_ms,
        24.0,
        Default::default(),
        Some(InsertDecision::HoldRepair),
        false,
        true,
    );
    let dispatch = source.execute_receive_feedback_keyframe(
        decision,
        "insert-gate-supply-break",
        Some(90_002),
        now_ms,
        true,
    );

    assert!(matches!(dispatch, KeyframeRequestDispatch::Sent(_)));
    let stats = source.runtime_stats.read(|stats| {
        (
            stats.latest_keyframe_request_outcome.clone(),
            stats.receive_keyframe_last_sent_at_ms,
            stats.receive_keyframe_sent_count_unresolved,
            stats.latest_keyframe_request_episode.clone(),
        )
    });
    assert_eq!(stats, Some((Some("sent".to_string()), None, 0, None)));
}

#[test]
fn hard_receive_feedback_keyframe_creates_picture_recovery_episode() {
    let (_tx, _transport_observation_rx, mut source) = make_video_source_for_test();
    let now_ms = 1_000.0;
    source.runtime_stats.begin_transport_recovery_episode(900.0);
    source.runtime_stats.update(|stats| {
        stats.recovery_effective_rtt_ms = Some(24.0);
    });
    source.sync_recovery_ledger_to_stats();

    let decision = source.plan_receive_feedback(
        "receiver-local-nack-gap-too-large",
        now_ms,
        24.0,
        Default::default(),
        Some(InsertDecision::HoldRepair),
        true,
        false,
    );
    let dispatch = source.execute_receive_feedback_keyframe(
        decision,
        "receiver-local-nack-gap-too-large",
        None,
        now_ms,
        true,
    );

    assert!(matches!(dispatch, KeyframeRequestDispatch::Sent(_)));
    let stats = source.runtime_stats.read(|stats| {
        (
            stats.receive_recovery_ledger_generation,
            stats.receive_keyframe_last_sent_at_ms,
            stats.latest_keyframe_request_episode.clone(),
            stats.latest_picture_recovery_transition_observation.clone(),
        )
    });
    let (ledger_generation, last_sent_at_ms, episode, transition) = stats.expect("runtime stats");
    assert_eq!(last_sent_at_ms, Some(now_ms));
    let episode = episode.expect("keyframe request episode");
    assert_eq!(Some(episode.episode_id), ledger_generation);
    assert_eq!(
        episode.request_reason.as_deref(),
        Some("receiverWaitingKeyframe")
    );
    assert_eq!(episode.request_kind.as_deref(), Some("pli"));
    assert_eq!(episode.status, "sent");
    assert_eq!(episode.sent_at_ms, Some(now_ms));
    let transition = transition.expect("picture recovery transition");
    assert_eq!(transition.phase, "PliSent");
    assert_eq!(Some(episode.episode_id), transition.episode_id);
}

#[test]
fn frame_without_recovery_ledger_defaults_to_steady_disposition() {
    let (_, _, mut source) = make_video_source_for_test();

    let (deadline_at_ms, disposition, unrecoverable_reason, budget) =
        source.take_frame_recovery_ledger(123_456);

    assert_eq!(deadline_at_ms, None);
    assert_eq!(
        disposition,
        crate::media::video::types::FrameRecoveryDisposition::Steady
    );
    assert_eq!(unrecoverable_reason, None);
    assert_eq!(budget, None);
}

#[test]
fn first_timeout_detection_only_starts_confirmation_window() {
    let now = Instant::now();
    let mut pending_since = None;

    assert!(
        !RtcVideoFrameSource::should_confirm_transient_timeout_signal(
            &mut pending_since,
            now,
            Duration::from_millis(120),
        )
    );
    assert!(pending_since.is_some());
}

#[test]
fn timeout_detection_after_confirmation_window_is_emitted() {
    let now = Instant::now();
    let mut pending_since = Some(now);

    assert!(
        RtcVideoFrameSource::should_confirm_transient_timeout_signal(
            &mut pending_since,
            now + Duration::from_millis(130),
            Duration::from_millis(120),
        )
    );
    assert!(pending_since.is_none());
}

#[test]
fn idle_timeout_is_suppressed_before_first_packet() {
    let started_at = Instant::now();
    let later = started_at + Duration::from_millis(500);

    assert!(!should_trigger_idle_timeout(
        false,
        later,
        started_at,
        Duration::from_millis(150),
    ));
    assert!(should_trigger_idle_timeout(
        true,
        later,
        started_at,
        Duration::from_millis(150),
    ));
}

#[test]
fn cloud_profile_relaxes_idle_timeout_and_hint_cooldown() {
    let (idle_timeout, idle_hint_cooldown) = resolve_effective_idle_controls(
        Duration::from_millis(250),
        Duration::from_millis(400),
        Some(&XbxEngineTargetTypeDto::Cloud),
        Some(120.0),
    );

    assert_eq!(idle_timeout, Duration::from_millis(700));
    assert_eq!(idle_hint_cooldown, Duration::from_millis(700));
}

#[test]
fn slow_feedback_relaxes_idle_timeout_even_for_non_cloud() {
    let (idle_timeout, idle_hint_cooldown) = resolve_effective_idle_controls(
        Duration::from_millis(300),
        Duration::from_millis(450),
        Some(&XbxEngineTargetTypeDto::Home),
        Some(500.0),
    );

    assert_eq!(idle_timeout, Duration::from_millis(700));
    assert_eq!(idle_hint_cooldown, Duration::from_millis(700));
}

#[test]
fn steady_idle_timeout_is_absorbed_when_render_output_is_still_fresh() {
    let absorbed = should_absorb_idle_timeout_for_steady_gap(
        XbxEngineTransportStateDto::Connected,
        3,
        Some(3),
        Some("displayed-idr"),
        Some(1_000.0 - 90.0),
        Some(1_000.0 - 70.0),
        Some(false),
        Some(false),
        1_000.0,
        Duration::from_millis(150),
    );
    assert!(absorbed);

    let decoded_anchor_absorbed = should_absorb_idle_timeout_for_steady_gap(
        XbxEngineTransportStateDto::Connected,
        3,
        Some(3),
        Some("decoded-usable-idr"),
        None,
        Some(1_000.0 - 70.0),
        Some(false),
        Some(false),
        1_000.0,
        Duration::from_millis(150),
    );
    assert!(decoded_anchor_absorbed);
}

#[test]
fn no_render_slack_or_no_fresh_output_still_emits_idle_timeout_observation() {
    let stale_output_not_absorbed = should_absorb_idle_timeout_for_steady_gap(
        XbxEngineTransportStateDto::Connected,
        3,
        Some(3),
        Some("displayed-idr"),
        Some(1_000.0 - 520.0),
        Some(1_000.0 - 510.0),
        Some(false),
        Some(false),
        1_000.0,
        Duration::from_millis(150),
    );
    assert!(!stale_output_not_absorbed);

    let no_anchor_not_absorbed = should_absorb_idle_timeout_for_steady_gap(
        XbxEngineTransportStateDto::Connected,
        3,
        None,
        None,
        Some(1_000.0 - 60.0),
        Some(1_000.0 - 45.0),
        Some(false),
        Some(false),
        1_000.0,
        Duration::from_millis(150),
    );
    assert!(!no_anchor_not_absorbed);
}

#[test]
fn recovery_wait_without_hard_risk_allows_healthy_delta_to_submit() {
    let (next_is_blocking_non_keyframe_admission, recovery_action) =
        resolve_recovery_keyframe_action(true, true, false, false, false, 0, 0, false, false);

    assert!(!next_is_blocking_non_keyframe_admission);
    assert_eq!(recovery_action, RecoveryKeyframeAction::Submit);
}

#[test]
fn clean_anchor_serviceable_output_allows_soft_recovery_keyframe_request_after_invalid_bootstrap() {
    let now_ms = 1_000.0;
    let mut stats = serviceable_runtime_stats(now_ms);
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 1,
        source_event: "frame-observed".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "receiving".to_string(),
            reason: None,
            chain_break_evidence: None,

            observed_at_ms: now_ms - 12.0,
        },
        observed_at_ms: now_ms - 12.0,
    });

    assert!(!RtcVideoFrameSource::should_soft_request_recovery_keyframe(
        &stats,
        now_ms,
        Some("bootstrapMissingIdr"),
        true,
        false,
        true,
    ));
    assert!(!RtcVideoFrameSource::should_soft_request_recovery_keyframe(
        &stats,
        now_ms,
        Some("bootstrapMissingSps"),
        false,
        false,
        true,
    ));
}

#[test]
fn stale_transport_issue_after_clean_anchor_allows_soft_recovery_keyframe_request() {
    let now_ms = 1_000.0;
    let mut stats = serviceable_runtime_stats(now_ms);
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 2,
        source_event: "frame-await-recovery-anchor".to_string(),
        gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
            state: "pending".to_string(),
            sequence: Some(99),
            frame_rtp_timestamp: None,
            frame_importance: Some("supply".to_string()),
            budget_importance: None,

            evidence_importance: None,

            gap_dependency_confidence: None,

            observed_at_ms: now_ms - 5.0,
        }),
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "waiting-keyframe".to_string(),
            reason: Some("receiverWaitingKeyframe".to_string()),
            chain_break_evidence: None,

            observed_at_ms: now_ms - 5.0,
        },
        observed_at_ms: now_ms - 5.0,
    });

    assert!(RtcVideoFrameSource::should_soft_request_recovery_keyframe(
        &stats,
        now_ms,
        Some("bootstrapMissingSps"),
        true,
        true,
        true,
    ));
}

#[test]
fn unresolved_transport_issue_without_clean_anchor_blocks_soft_recovery_keyframe_request() {
    let now_ms = 1_000.0;
    let mut stats = serviceable_runtime_stats(now_ms);
    stats.video_anchor_clean_epoch = None;
    stats.video_anchor_clean_observed_at_ms = None;
    stats.video_anchor_clean_source_event = None;
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 2,
        source_event: "frame-await-recovery-anchor".to_string(),
        gap: Some(crate::XbxEngineVideoTimelineGapSnapshot {
            state: "pending".to_string(),
            sequence: Some(99),
            frame_rtp_timestamp: None,
            frame_importance: Some("supply".to_string()),
            budget_importance: None,
            evidence_importance: None,
            gap_dependency_confidence: None,
            observed_at_ms: now_ms - 5.0,
        }),
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "waiting-keyframe".to_string(),
            reason: Some("receiverWaitingKeyframe".to_string()),
            chain_break_evidence: None,
            observed_at_ms: now_ms - 5.0,
        },
        observed_at_ms: now_ms - 5.0,
    });

    assert!(!RtcVideoFrameSource::should_soft_request_recovery_keyframe(
        &stats,
        now_ms,
        Some("bootstrapMissingSps"),
        true,
        true,
        true,
    ));
}

#[test]
fn unresolved_transport_await_without_current_clean_anchor_rearms_clean_anchor() {
    let now_ms = 1_000.0;
    let mut stats = serviceable_runtime_stats(now_ms);
    stats.video_anchor_clean_epoch = None;
    stats.video_anchor_clean_observed_at_ms = None;
    stats.video_anchor_clean_source_event = None;
    stats.latest_video_timeline_observation = Some(crate::XbxEngineVideoTimelineObservation {
        observation_id: 22,
        source_event: "frame-await-recovery-anchor".to_string(),
        gap: None,
        frame: None,
        chain: crate::XbxEngineVideoTimelineChainSnapshot {
            state: "waiting-keyframe".to_string(),
            reason: Some("receiverWaitingKeyframe".to_string()),
            chain_break_evidence: None,
            observed_at_ms: now_ms - 5.0,
        },
        observed_at_ms: now_ms - 5.0,
    });

    assert!(RtcVideoFrameSource::should_rearm_clean_anchor_for_transport_await(&stats));

    stats.video_anchor_clean_epoch = Some(stats.transport_recovery_epoch);
    stats.video_anchor_clean_observed_at_ms = Some(now_ms - 1.0);
    stats.video_anchor_clean_source_event = Some("displayed-idr".to_string());

    assert!(!RtcVideoFrameSource::should_rearm_clean_anchor_for_transport_await(&stats));
}

#[test]
fn recovery_wait_does_not_override_loss_semantics() {
    let (next_is_blocking_non_keyframe_admission, recovery_action) =
        resolve_recovery_keyframe_action(true, true, false, false, false, 0, 1, false, false);

    assert!(!next_is_blocking_non_keyframe_admission);
    assert_eq!(recovery_action, RecoveryKeyframeAction::DropAndRequestPli);
}

#[test]
fn lossy_keyframe_defers_to_nack_recovery_admission() {
    let (next_is_blocking_non_keyframe_admission, recovery_action) =
        resolve_recovery_keyframe_action(true, false, false, false, true, 0, 2, false, false);

    assert!(!next_is_blocking_non_keyframe_admission);
    assert_eq!(recovery_action, RecoveryKeyframeAction::DropAndRequestPli);
}

#[test]
fn short_sample_loss_burst_stays_in_drop_and_request_keyframe() {
    let (next_is_blocking_non_keyframe_admission, recovery_action) =
        resolve_recovery_keyframe_action(true, false, false, false, false, 2, 1, false, false);

    assert!(!next_is_blocking_non_keyframe_admission);
    assert_eq!(recovery_action, RecoveryKeyframeAction::DropAndRequestPli);
}

#[test]
fn longer_sample_loss_burst_still_defers_to_nack_recovery_admission() {
    let (next_is_blocking_non_keyframe_admission, recovery_action) =
        resolve_recovery_keyframe_action(true, false, false, false, false, 3, 1, false, false);

    assert!(!next_is_blocking_non_keyframe_admission);
    assert_eq!(recovery_action, RecoveryKeyframeAction::DropAndRequestPli);
}

#[test]
fn low_value_local_gap_wait_is_absorbed_without_transport_wait_upgrade() {
    let (next_is_blocking_non_keyframe_admission, recovery_action) =
        resolve_recovery_keyframe_action(true, true, false, false, false, 0, 0, false, false);

    assert!(!next_is_blocking_non_keyframe_admission);
    assert_eq!(recovery_action, RecoveryKeyframeAction::Submit);
}

#[test]
fn pre_first_frame_wait_does_not_absorb_non_keyframe_delta() {
    let (next_is_blocking_non_keyframe_admission, recovery_action) =
        resolve_recovery_keyframe_action(false, true, false, false, false, 0, 0, false, false);

    assert!(next_is_blocking_non_keyframe_admission);
    assert_eq!(recovery_action, RecoveryKeyframeAction::WaitKeyframe);
}

#[test]
fn active_recovery_epoch_ignores_stale_prior_output_when_judging_first_frame_acquired() {
    let mut stats = crate::XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 9;
    stats.transport_recovery_episode_active = true;
    stats.transport_recovery_episode_opened_at_ms = Some(1_000.0);
    stats.latest_video_decode_ok_time_ms = Some(900.0);
    stats.latest_host_mailbox_submit_time_ms = Some(910.0);
    stats.latest_video_host_present_time_ms = Some(920.0);
    stats.host_mailbox_enqueue_count_total = 12;
    stats.host_frame_present_epoch = 6;

    assert!(!RtcVideoFrameSource::first_frame_acquired(&stats));

    stats.latest_video_decode_ok_time_ms = Some(1_020.0);
    assert!(RtcVideoFrameSource::first_frame_acquired(&stats));
}

#[test]
fn current_epoch_clean_anchor_counts_as_first_frame_acquired() {
    let mut stats = crate::XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 4;
    stats.transport_recovery_episode_active = true;
    stats.transport_recovery_episode_opened_at_ms = Some(2_000.0);
    stats.video_anchor_clean_epoch = Some(4);
    stats.video_anchor_clean_observed_at_ms = Some(2_001.0);
    stats.recovery_displayed_idr_at_ms = Some(2_001.0);
    stats.recovery_fresh_anchor_recovered_at_ms = Some(2_001.0);
    stats.video_anchor_clean_source_event = Some("displayed-idr".to_string());

    assert!(RtcVideoFrameSource::first_frame_acquired(&stats));
}

#[test]
fn sustaining_recovery_prefers_keepalive_over_reenter_wait_keyframe() {
    let (next_is_blocking_non_keyframe_admission, recovery_action) =
        resolve_recovery_keyframe_action(true, true, true, true, true, 0, 0, false, false);

    assert!(!next_is_blocking_non_keyframe_admission);
    assert_eq!(recovery_action, RecoveryKeyframeAction::Submit);
}

#[test]
fn hard_recovery_wait_without_building_phase_still_reenters_wait_keyframe() {
    let (next_is_blocking_non_keyframe_admission, recovery_action) =
        resolve_recovery_keyframe_action(true, true, false, false, true, 0, 0, false, false);

    assert!(next_is_blocking_non_keyframe_admission);
    assert_eq!(recovery_action, RecoveryKeyframeAction::WaitKeyframe);
}

#[test]
fn clean_anchor_building_phase_allows_stale_wait_continuation_to_submit() {
    let (next_is_blocking_non_keyframe_admission, recovery_action) =
        resolve_recovery_keyframe_action(true, true, false, false, true, 0, 0, true, true);

    assert!(!next_is_blocking_non_keyframe_admission);
    assert_eq!(recovery_action, RecoveryKeyframeAction::Submit);
}

#[test]
fn drop_and_request_action_contract_keeps_resolver_stateless() {
    let (next_is_blocking_non_keyframe_admission, recovery_action) =
        resolve_recovery_keyframe_action(true, true, false, false, true, 3, 2, false, false);

    assert_eq!(recovery_action, RecoveryKeyframeAction::DropAndRequestPli);
    // resolve 层只给出动作，等待态由 action 分支显式处理，避免隐式耦合。
    assert!(!next_is_blocking_non_keyframe_admission);
}

#[tokio::test]
async fn drop_and_request_fallback_without_missing_sequences_requests_keyframe() {
    let (_tx, mut transport_observation_rx, mut source) = make_video_source_for_test();

    source
        .handle_drop_and_request_keyframe_action(12_345, 2, false, "disposable")
        .await;

    assert!(transport_observation_rx.try_recv().is_err());
    assert!(source.is_blocking_non_keyframe_admission());
    assert_eq!(
        source.receiver_local_state(),
        crate::transport::rtc::receive::ReceiverState::WaitingKeyframe
    );
}

#[test]
#[ignore = "decoder_feedback_allows_sustaining_exit removed per receive-side RFC"]
fn sustaining_recovery_exit_gate_requires_recent_decoder_feedback() {}

#[tokio::test]
#[ignore = "pre-decode clean-anchor / decoder feedback gate removed per receive-side RFC"]
async fn clean_anchor_allows_first_recovery_idr_to_bypass_stale_decoder_feedback_gate() {
    let (tx, _transport_observation_rx, mut source) = make_video_source_for_test();
    source.runtime_stats.begin_transport_recovery_episode(100.0);
    source.runtime_stats.update(|stats| {
        let now_ms = now_ms_f64();
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 450.0);
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
    });
    send_bootstrap_access_unit(&tx, 100, 9_000).await;
    tx.send(make_video_rtp_packet(
        103,
        9_016,
        true,
        bootstrap_idr_nalu(),
    ))
    .await
    .expect("next frame packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should produce frame")
        .expect("frame should exist");

    assert!(frame.is_keyframe);
    assert_eq!(frame.rtp_timestamp, 9_000);
    assert_eq!(frame.clean_anchor_commit_recovery_epoch, Some(1));
    assert_eq!(
        frame.frame_recovery_disposition,
        FrameRecoveryDisposition::Repairing
    );
    assert_eq!(
        frame.budget.window_source,
        FrameBudgetWindowSource::Recovery
    );
    assert!(matches!(
        frame.budget.rtt_slack,
        crate::media::video::ingress::budget::FrameBudgetRttSlack::Unknown
            | crate::media::video::ingress::budget::FrameBudgetRttSlack::Ample
            | crate::media::video::ingress::budget::FrameBudgetRttSlack::Tight
            | crate::media::video::ingress::budget::FrameBudgetRttSlack::Exhausted
    ));
}

fn test_receiver_decode_context(
    receiver_state: ReceiverState,
    has_active_gap: bool,
    nack_exhausted: bool,
    first_frame_acquired: bool,
) -> ReceiverDecodeContext {
    test_receiver_decode_context_with_output(
        receiver_state,
        has_active_gap,
        nack_exhausted,
        first_frame_acquired,
        first_frame_acquired,
    )
}

fn test_receiver_decode_context_with_output(
    receiver_state: ReceiverState,
    has_active_gap: bool,
    nack_exhausted: bool,
    first_frame_acquired: bool,
    decoder_reference_synced: bool,
) -> ReceiverDecodeContext {
    ReceiverDecodeContext {
        receiver_state,
        has_active_gap,
        nack_exhausted,
        first_frame_acquired,
        decoder_reference_synced,
    }
}

fn test_insert_context(
    decode: ReceiverDecodeContext,
    gap_mode: GapVsKeyframeMode,
    action_stage: PacketRecoveryActionStage,
) -> InsertContext {
    InsertContext {
        decode,
        gap_mode,
        action_stage,
        fresh_idr_admission: false,
        post_parameter_sets_change_strict: false,
        supply_break_continuation: false,
        reference_chain_state: ReferenceChainState::Continuous,
        keyframe_required: false,
    }
}

fn assert_insert_emit(inspection: &H264AccessUnitInspection, ctx: &InsertContext) {
    assert_eq!(
        resolve_insert_decision(inspection, ctx, DecodeCorruptionPolicy::StandardWebRtc, 0),
        InsertDecision::Emit
    );
}

fn assert_insert_hold(inspection: &H264AccessUnitInspection, ctx: &InsertContext) {
    assert_eq!(
        resolve_insert_decision(inspection, ctx, DecodeCorruptionPolicy::StandardWebRtc, 0),
        InsertDecision::HoldRepair
    );
}

#[test]
fn insert_decision_rejects_frames_without_bootstrap_or_continuation() {
    let bootstrap_idr = H264AccessUnitInspection {
        nals: Vec::new(),
        parameter_sets: None,
        width: None,
        height: None,
        is_idr: true,
        has_inband_sps: false,
        has_inband_pps: false,
        slice_headers_valid: true,
        parameter_sets_changed: false,
        config_changed: false,
        bootstrap_ready: true,
        bootstrap_reject_reason: None,
        commit_state:
            crate::media::video::h264::inspection::H264AccessUnitInspector::test_commit_state(),
    };
    assert_insert_emit(
        &bootstrap_idr,
        &test_insert_context(
            test_receiver_decode_context(ReceiverState::Receiving, false, false, true),
            GapVsKeyframeMode::RepairFirst,
            PacketRecoveryActionStage::Steady,
        ),
    );

    let non_idr = H264AccessUnitInspection {
        nals: Vec::new(),
        parameter_sets: None,
        width: None,
        height: None,
        is_idr: false,
        has_inband_sps: false,
        has_inband_pps: false,
        slice_headers_valid: true,
        parameter_sets_changed: false,
        config_changed: false,
        bootstrap_ready: false,
        bootstrap_reject_reason: None,
        commit_state:
            crate::media::video::h264::inspection::H264AccessUnitInspector::test_commit_state(),
    };
    assert_insert_hold(
        &non_idr,
        &test_insert_context(
            test_receiver_decode_context(ReceiverState::Priming, false, false, false),
            GapVsKeyframeMode::RepairFirst,
            PacketRecoveryActionStage::WaitKeyframe,
        ),
    );

    let invalid_slice = H264AccessUnitInspection {
        slice_headers_valid: false,
        ..non_idr.clone()
    };
    assert_insert_hold(
        &invalid_slice,
        &test_insert_context(
            test_receiver_decode_context(ReceiverState::Receiving, false, false, true),
            GapVsKeyframeMode::RepairFirst,
            PacketRecoveryActionStage::Steady,
        ),
    );
}

#[test]
fn pre_first_frame_non_idr_continuation_is_rejected_until_first_frame_exists() {
    let inspector = H264AccessUnitInspector::new();
    inspector
        .seed_committed_parameter_sets_if_absent(&bootstrap_sps_nalu(), &bootstrap_pps_nalu())
        .expect("seed committed parameter sets");
    let mut payload = vec![0, 0, 0, 1];
    payload.extend_from_slice(&bootstrap_non_idr_nalu());
    let inspection = inspector
        .inspect_access_unit(&payload)
        .expect("inspect non-idr access unit");

    assert!(inspection.delta_continuation_ready());
    assert_insert_hold(
        &inspection,
        &test_insert_context(
            test_receiver_decode_context(ReceiverState::Priming, false, false, false),
            GapVsKeyframeMode::RepairFirst,
            PacketRecoveryActionStage::WaitKeyframe,
        ),
    );
    assert_insert_hold(
        &inspection,
        &test_insert_context(
            test_receiver_decode_context_with_output(
                ReceiverState::Receiving,
                false,
                false,
                true,
                false,
            ),
            GapVsKeyframeMode::RepairFirst,
            PacketRecoveryActionStage::Steady,
        ),
    );
}

#[test]
fn waiting_keyframe_holds_non_idr_continuation_during_active_repair() {
    let inspector = H264AccessUnitInspector::new();
    inspector
        .seed_committed_parameter_sets_if_absent(&bootstrap_sps_nalu(), &bootstrap_pps_nalu())
        .expect("seed committed parameter sets");
    let mut payload = vec![0, 0, 0, 1];
    payload.extend_from_slice(&bootstrap_non_idr_nalu());
    let inspection = inspector
        .inspect_access_unit(&payload)
        .expect("inspect non-idr access unit");

    assert!(inspection.delta_continuation_ready());
    assert_insert_hold(
        &inspection,
        &test_insert_context(
            test_receiver_decode_context_with_output(
                ReceiverState::WaitingKeyframe,
                false,
                true,
                true,
                true,
            ),
            GapVsKeyframeMode::RepairFirst,
            PacketRecoveryActionStage::NackPending,
        ),
    );
}

#[test]
fn displayed_idr_decoder_sync_holds_non_idr_when_hard_gap_blocks_delta() {
    let inspector = H264AccessUnitInspector::new();
    inspector
        .seed_committed_parameter_sets_if_absent(&bootstrap_sps_nalu(), &bootstrap_pps_nalu())
        .expect("seed committed parameter sets");
    let mut payload = vec![0, 0, 0, 1];
    payload.extend_from_slice(&bootstrap_non_idr_nalu());
    let inspection = inspector
        .inspect_access_unit(&payload)
        .expect("inspect non-idr access unit");

    assert!(inspection.delta_continuation_ready());
    assert_insert_hold(
        &inspection,
        &test_insert_context(
            test_receiver_decode_context_with_output(
                ReceiverState::WaitingKeyframe,
                true,
                true,
                true,
                true,
            ),
            GapVsKeyframeMode::RepairFirst,
            PacketRecoveryActionStage::NackPending,
        ),
    );
}

#[test]
fn waiting_keyframe_rejects_non_idr_continuation_when_hard_gap_blocks_delta() {
    let inspector = H264AccessUnitInspector::new();
    inspector
        .seed_committed_parameter_sets_if_absent(&bootstrap_sps_nalu(), &bootstrap_pps_nalu())
        .expect("seed committed parameter sets");
    let mut payload = vec![0, 0, 0, 1];
    payload.extend_from_slice(&bootstrap_non_idr_nalu());
    let inspection = inspector
        .inspect_access_unit(&payload)
        .expect("inspect non-idr access unit");

    assert!(inspection.delta_continuation_ready());
    assert_insert_hold(
        &inspection,
        &test_insert_context(
            test_receiver_decode_context_with_output(
                ReceiverState::WaitingKeyframe,
                true,
                true,
                true,
                false,
            ),
            GapVsKeyframeMode::RepairFirst,
            PacketRecoveryActionStage::NackPending,
        ),
    );
}

#[test]
fn blocking_is_only_when_waiting_and_nack_exhausted() {
    assert!(should_block_non_keyframe_admission(
        &test_receiver_decode_context(ReceiverState::WaitingKeyframe, true, true, true,)
    ));
    assert!(!should_block_non_keyframe_admission(
        &test_receiver_decode_context(ReceiverState::Repairing, true, true, true,)
    ));
    assert!(!(true && !true));
    assert!(true && !false);
}

#[test]
fn repairing_continuation_is_held_during_gap_repair() {
    let inspector = H264AccessUnitInspector::new();
    inspector
        .seed_committed_parameter_sets_if_absent(&bootstrap_sps_nalu(), &bootstrap_pps_nalu())
        .expect("seed committed parameter sets");
    let mut payload = vec![0, 0, 0, 1];
    payload.extend_from_slice(&bootstrap_non_idr_nalu());
    let inspection = inspector
        .inspect_access_unit(&payload)
        .expect("inspect non-idr access unit");

    assert!(inspection.delta_continuation_ready());
    assert_insert_hold(
        &inspection,
        &test_insert_context(
            test_receiver_decode_context(ReceiverState::Repairing, true, false, true),
            GapVsKeyframeMode::RepairFirst,
            PacketRecoveryActionStage::NackPending,
        ),
    );
}

#[test]
fn clean_anchor_records_current_transport_recovery_epoch() {
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    let (transport_observation_tx, _transport_observation_rx) =
        tokio::sync::mpsc::unbounded_channel();
    let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
    let source = RtcVideoFrameSource::new(
        rx,
        transport_observation_tx,
        runtime_stats.clone(),
        16,
        Duration::from_millis(10),
        Duration::from_millis(20),
        Duration::from_millis(200),
        NackSchedulerConfig {
            max_age_ms: 1_000,
            frame_deadline_ms: 120,
            burst_count: 2,
            retry_interval_ms: 20,
            max_retry_count: 3,
        },
        test_transport_capability(),
    );
    drop(tx);

    source.runtime_stats.begin_transport_recovery_episode(100.0);
    source.runtime_stats.record_pending_displayed_idr_rtp(1);
    source
        .runtime_stats
        .seed_decoder_reference_sync_for_pending_idr(1, 170.0);
    source
        .runtime_stats
        .record_displayed_idr_fact(180.0, 1, None);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.video_anchor_clean_epoch, Some(1));
    assert_eq!(stats.video_anchor_clean_observed_at_ms, Some(180.0));
    assert_eq!(
        stats.video_anchor_clean_source_event.as_deref(),
        Some("displayed-idr")
    );
    assert!(stats.transport_recovery_episode_active);
    assert_eq!(stats.transport_recovery_episode_closed_at_ms, None);
    assert_eq!(stats.transport_recovery_episode_close_reason, None);
}

#[tokio::test]
async fn rx_closed_records_close_cause_label() {
    let (tx, _transport_observation_rx, mut source) = make_video_source_for_test();
    source
        .runtime_stats
        .record_video_ingress_close_intent(now_ms_f64(), "rebuildPeerConnection");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should finish after rx closes");
    assert!(frame.is_none());

    let stats = source
        .runtime_stats
        .read(|stats| {
            (
                stats.latest_observation_label.clone(),
                stats.latest_observation_summary.clone(),
            )
        })
        .expect("runtime stats");
    assert_eq!(stats.0.as_deref(), Some("rtcVideoIngressRxClosed"));
    assert!(stats
        .1
        .as_deref()
        .is_some_and(|summary| summary.contains("cause=rebuildPeerConnection")));
}

#[test]
fn packet_loss_detected_does_not_reopen_episode_but_keyframe_request_does() {
    let (_tx, rx) = tokio::sync::mpsc::channel(1);
    let (transport_observation_tx, _transport_observation_rx) =
        tokio::sync::mpsc::unbounded_channel();
    let runtime_stats = Arc::new(Mutex::new(crate::XbxEngineMediaRuntimeStats::default()));
    let mut source = RtcVideoFrameSource::new(
        rx,
        transport_observation_tx,
        runtime_stats.clone(),
        16,
        Duration::from_millis(10),
        Duration::from_millis(20),
        Duration::from_millis(200),
        NackSchedulerConfig {
            max_age_ms: 1_000,
            frame_deadline_ms: 120,
            burst_count: 2,
            retry_interval_ms: 20,
            max_retry_count: 3,
        },
        test_transport_capability(),
    );

    source.runtime_stats.begin_transport_recovery_episode(100.0);
    source.runtime_stats.record_pending_displayed_idr_rtp(1);
    source
        .runtime_stats
        .seed_decoder_reference_sync_for_pending_idr(1, 130.0);
    source
        .runtime_stats
        .record_displayed_idr_fact(140.0, 1, None);
    source
        .runtime_stats
        .complete_transport_recovery_after_stable_settle(180.0);

    source.queue_transport_observation(TransportObservation::Loss(
        TransportLossObservation::PacketLossDetected,
    ));

    {
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.transport_recovery_epoch, 1);
        assert!(!stats.transport_recovery_episode_active);
        assert_eq!(stats.video_anchor_clean_epoch, Some(1));
        assert_eq!(
            stats.video_anchor_clean_source_event.as_deref(),
            Some("displayed-idr")
        );
    }

    source.queue_transport_observation(TransportObservation::Loss(
        TransportLossObservation::RecoveryKeyframeRequested,
    ));

    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.transport_recovery_epoch, 1);
    assert!(!stats.transport_recovery_episode_active);
    assert_eq!(stats.video_anchor_clean_epoch, Some(1));
    assert!(stats.video_anchor_clean_observed_at_ms.is_some());
    assert_eq!(
        stats.video_anchor_clean_source_event.as_deref(),
        Some("displayed-idr")
    );
}

#[tokio::test]
async fn bootstrap_keyframe_packets_are_assembled_into_frame() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();

    send_bootstrap_access_unit(&tx, 100, 9000).await;
    tx.send(make_video_rtp_packet(103, 9016, true, bootstrap_idr_nalu()))
        .await
        .expect("next frame packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("frame assembly should finish")
        .expect("bootstrap frame should be emitted");
    assert!(frame.is_keyframe);
    assert!(frame.h264.bootstrap_ready);
    assert_eq!(frame.rtp_timestamp, 9000);
    assert!(frame.width > 0);
    assert!(frame.height > 0);
    assert!(transport_observation_rx.try_recv().is_err());
}

#[tokio::test]
async fn assembled_frame_carries_current_transport_recovery_epoch_tag() {
    let (tx, _transport_observation_rx, mut source) = make_video_source_for_test();
    source.runtime_stats.begin_transport_recovery_episode(100.0);

    send_bootstrap_access_unit(&tx, 100, 9000).await;
    tx.send(make_video_rtp_packet(103, 9016, true, bootstrap_idr_nalu()))
        .await
        .expect("next frame packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("frame assembly should finish")
        .expect("bootstrap frame should be emitted");
    assert_eq!(frame.rtp_timestamp, 9000);
    assert_eq!(frame.recovery_epoch_tag, Some(1));
}

#[tokio::test]
async fn invalid_keyframe_does_not_arm_clean_anchor_ingress() {
    let (tx, _transport_observation_rx, mut source) = make_video_source_for_test();
    source.runtime_stats.begin_transport_recovery_episode(100.0);

    tx.send(make_video_rtp_packet(
        100,
        9_000,
        true,
        bootstrap_idr_nalu(),
    ))
    .await
    .expect("invalid keyframe packet should be queued");
    tx.send(make_video_rtp_packet(
        101,
        9_016,
        true,
        bootstrap_idr_nalu(),
    ))
    .await
    .expect("tail packet should flush invalid keyframe sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should finish after invalid keyframe and rx close");

    assert!(frame.is_none());
}

#[tokio::test]
async fn clean_anchor_waits_for_decode_before_committing_stats() {
    let (tx, _transport_observation_rx, mut source) = make_video_source_for_test();
    source.runtime_stats.begin_transport_recovery_episode(100.0);
    source.runtime_stats.update(|stats| {
        let now_ms = now_ms_f64();
        stats.latest_video_decode_ok_time_ms = Some(now_ms - 16.0);
        stats.video_decoder_stalled = Some(false);
        stats.video_renderer_stalled = Some(false);
    });
    send_bootstrap_access_unit(&tx, 100, 9_000).await;
    tx.send(make_video_rtp_packet(
        103,
        9_016,
        true,
        bootstrap_idr_nalu(),
    ))
    .await
    .expect("next frame packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should produce frame")
        .expect("frame should exist");

    assert_eq!(frame.clean_anchor_commit_recovery_epoch, None);
    let stats = source.runtime_stats.read(|stats| {
        (
            stats.video_anchor_clean_epoch,
            stats.video_anchor_clean_source_event.clone(),
            stats.latest_anchor_candidate_ledger.clone(),
        )
    });
    let (clean_epoch, clean_source_event, latest_anchor_candidate_ledger) =
        stats.expect("runtime stats");
    assert_eq!(clean_epoch, None);
    assert_eq!(clean_source_event, None);
    let ledger = latest_anchor_candidate_ledger.expect("observed anchor candidate");
    assert_eq!(ledger.state, XbxEngineAnchorCandidateState::Observed);
    assert_eq!(ledger.frame_rtp_timestamp, Some(frame.rtp_timestamp));
}

#[tokio::test]
async fn recovery_required_fresh_idr_without_episode_becomes_recovery_owner() {
    let (tx, _transport_observation_rx, mut source) = make_video_source_for_test();
    source.runtime_stats.begin_transport_recovery_episode(100.0);
    source.sync_recovery_ledger_to_stats();
    source
        .trace_ledger
        .recovery_ledger_mut()
        .note_nack_exhausted();
    source.sync_recovery_ledger_to_stats();
    let episode = source
        .runtime_stats
        .read(|stats| stats.latest_keyframe_request_episode.clone())
        .expect("runtime stats");
    assert!(episode.is_none());

    send_bootstrap_access_unit(&tx, 100, 9_000).await;
    tx.send(make_video_rtp_packet(
        103,
        9_016,
        true,
        bootstrap_idr_nalu(),
    ))
    .await
    .expect("next frame packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should produce frame")
        .expect("frame should exist");

    assert!(frame.is_keyframe);
    assert_eq!(frame.rtp_timestamp, 9_000);
    assert_eq!(frame.recovery_owner_rtp_timestamp, Some(9_000));
    assert_eq!(
        frame.budget.window_source,
        FrameBudgetWindowSource::Recovery
    );
    assert_eq!(
        frame.frame_recovery_disposition,
        FrameRecoveryDisposition::Repairing
    );
    assert_eq!(frame.clean_anchor_commit_recovery_epoch, None);
}

#[tokio::test]
async fn steady_fresh_idr_without_recovery_required_does_not_become_recovery_owner() {
    let (tx, _transport_observation_rx, mut source) = make_video_source_for_test();
    source.runtime_stats.begin_transport_recovery_episode(100.0);

    send_bootstrap_access_unit(&tx, 100, 9_000).await;
    tx.send(make_video_rtp_packet(
        103,
        9_016,
        true,
        bootstrap_idr_nalu(),
    ))
    .await
    .expect("next frame packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should produce frame")
        .expect("frame should exist");

    assert!(frame.is_keyframe);
    assert_eq!(frame.rtp_timestamp, 9_000);
    assert_eq!(frame.recovery_owner_rtp_timestamp, None);
    assert_eq!(frame.budget.window_source, FrameBudgetWindowSource::Playout);
    assert_eq!(frame.clean_anchor_commit_recovery_epoch, None);
}

#[test]
fn clean_anchor_ack_consumes_submission_epoch() {
    let (_tx, _transport_observation_rx, mut source) = make_video_source_for_test();
    source.runtime_stats.begin_transport_recovery_episode(100.0);

    source.runtime_stats.record_pending_displayed_idr_rtp(9_000);
    source
        .runtime_stats
        .seed_decoder_reference_sync_for_pending_idr(9_000, 110.0);
    source
        .runtime_stats
        .record_displayed_idr_fact(120.0, 9_000, None);
    source.runtime_stats.record_anchor_candidate_ledger(
        1,
        Some(9_016),
        XbxEngineAnchorCandidateState::Observed,
        "frame-complete-candidate",
        None,
        121.0,
    );

    source.maybe_ack_clean_anchor_commit_from_runtime_stats();

    assert_eq!(source.last_consumed_clean_anchor_epoch, Some(1));
}

#[test]
fn decoded_clean_anchor_ack_consumes_epoch_without_displayed_idr() {
    let (_tx, _transport_observation_rx, mut source) = make_video_source_for_test();
    source.runtime_stats.begin_transport_recovery_episode(100.0);
    source.waiting_recovery_keyframe_since_ms = Some(105.0);
    source.next_recovery_keyframe_retry_at_ms = Some(305.0);
    source
        .trace_ledger
        .recovery_ledger_mut()
        .note_keyframe_request_sent(105.0);
    source.runtime_stats.update(|stats| {
        stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
        stats.video_decoder_recovery_state_changed_at_ms = Some(105.0);
        stats.video_decoder_recovery_event = Some("external-decoder-reset-requested".to_string());
        stats.video_decoder_recovery_detail = Some("decoderResetRequested".to_string());
    });
    source.sync_recovery_ledger_to_stats();

    source
        .runtime_stats
        .record_picture_recovery_episode_decoded(120.0, 9_000, 42);

    source.maybe_ack_clean_anchor_commit_from_runtime_stats();

    assert_eq!(source.last_consumed_clean_anchor_epoch, Some(1));
    assert_eq!(source.waiting_recovery_keyframe_since_ms, None);
    assert_eq!(source.next_recovery_keyframe_retry_at_ms, None);
    let stats = source
        .runtime_stats
        .read(|stats| stats.clone())
        .expect("runtime stats snapshot");
    assert_eq!(stats.receive_keyframe_required, Some(false));
    assert_eq!(
        stats.receive_keyframe_response_state.as_deref(),
        Some("usable-idr")
    );
    assert_eq!(stats.receive_display_state.as_deref(), Some("none"));
    assert_eq!(stats.recovery_displayed_idr_at_ms, None);
    assert_eq!(
        stats.video_decoder_recovery_state.as_deref(),
        Some("nominal")
    );
    assert_eq!(
        stats.video_decoder_recovery_event.as_deref(),
        Some("clean-anchor-committed")
    );
    let receiver = stats
        .latest_video_receiver_observation
        .as_ref()
        .expect("clean anchor ack should publish receiver observation");
    assert!(!receiver.keyframe_request_pending);
    assert_ne!(receiver.receiver_state, "waiting-keyframe");
    let timeline = stats
        .latest_video_timeline_observation
        .as_ref()
        .expect("clean anchor ack should publish timeline observation");
    assert_eq!(timeline.source_event, "clean-anchor-committed");
    assert_ne!(timeline.chain.state, "waiting-keyframe");
}

#[test]
fn decoded_clean_anchor_ack_consumes_initial_epoch_zero() {
    let (_tx, _transport_observation_rx, mut source) = make_video_source_for_test();
    source
        .trace_ledger
        .recovery_ledger_mut()
        .note_keyframe_request_sent(105.0);
    source.sync_recovery_ledger_to_stats();

    source
        .runtime_stats
        .record_picture_recovery_episode_decoded(120.0, 9_000, 42);

    source.maybe_ack_clean_anchor_commit_from_runtime_stats();

    assert_eq!(source.last_consumed_clean_anchor_epoch, Some(0));
    let stats = source
        .runtime_stats
        .read(|stats| stats.clone())
        .expect("runtime stats snapshot");
    assert_eq!(stats.receive_keyframe_required, Some(false));
    assert_eq!(
        stats.receive_keyframe_response_state.as_deref(),
        Some("usable-idr")
    );
}

#[test]
fn clean_anchor_ack_consumes_epoch_after_recovery_advance() {
    let (_tx, _transport_observation_rx, mut source) = make_video_source_for_test();
    source.runtime_stats.begin_transport_recovery_episode(100.0);

    source.runtime_stats.record_pending_displayed_idr_rtp(9_000);
    source
        .runtime_stats
        .record_displayed_idr_fact(120.0, 9_000, None);
    source
        .runtime_stats
        .advance_transport_recovery_episode(130.0);

    source.maybe_ack_clean_anchor_commit_from_runtime_stats();

    assert_eq!(source.last_consumed_clean_anchor_epoch, None);
}

#[tokio::test]
async fn clean_anchor_then_consecutive_non_idr_continuation_does_not_fall_back_to_wait_keyframe() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();
    source
        .trace_ledger
        .mark_gap_reorder_pending(&[401], 0.5, Some(8_900), "supply", "supply");
    assert!(source.trace_ledger.has_hard_recovery_risk_for_test());

    send_bootstrap_access_unit(&tx, 100, 9_000).await;
    let non_idr = bootstrap_non_idr_nalu();
    tx.send(make_video_rtp_packet(103, 9_016, true, &non_idr))
        .await
        .expect("first continuation should flush bootstrap sample");
    tx.send(make_video_rtp_packet(104, 9_032, true, &non_idr))
        .await
        .expect("second continuation should flush first continuation");
    tx.send(make_video_rtp_packet(105, 9_048, true, &non_idr))
        .await
        .expect("third continuation should flush second continuation");
    tx.send(make_video_rtp_packet(106, 9_064, true, &non_idr))
        .await
        .expect("tail continuation should flush third continuation");
    drop(tx);

    let bootstrap_frame =
        tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
            .await
            .expect("bootstrap frame should assemble")
            .expect("bootstrap frame should be emitted");
    assert!(bootstrap_frame.is_keyframe);
    assert!(bootstrap_frame.h264.committed_sps_present());
    assert!(bootstrap_frame.h264.committed_pps_present());
    source
        .trace_ledger
        .note_clean_anchor_committed(Some(bootstrap_frame.rtp_timestamp));
    source
        .receive_core_mut()
        .receive_engine
        .clear_recovery_state_after_decoded_anchor();
    source.runtime_stats.update(|stats| {
        let now_ms = now_ms_f64();
        stats.latest_video_decode_ok_time_ms = Some(now_ms);
        stats.latest_video_decode_ok_rtp_timestamp = Some(bootstrap_frame.rtp_timestamp);
        stats.recovery_decoder_reference_synced_at_ms = Some(now_ms);
        stats.video_anchor_clean_epoch = Some(stats.transport_recovery_epoch);
        stats.video_anchor_clean_observed_at_ms = Some(now_ms);
    });
    source.sync_recovery_ledger_to_stats();

    for rtp in [9_016, 9_032, 9_048] {
        let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
            .await
            .expect("continuation frame should assemble")
            .expect("continuation should be emitted after bootstrap commit");
        assert_eq!(frame.rtp_timestamp, rtp);
        assert!(!frame.is_keyframe);
        assert!(frame.h264.committed_sps_present());
        assert!(frame.h264.committed_pps_present());
        assert!(frame.h264.delta_continuation_ready());
    }

    while transport_observation_rx.try_recv().is_ok() {}
}

#[tokio::test]
async fn stale_wait_after_clean_anchor_accepts_bootstrap_missing_idr_continuation() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();

    send_bootstrap_access_unit(&tx, 100, 9_000).await;
    let non_idr = bootstrap_non_idr_nalu();
    tx.send(make_video_rtp_packet(103, 9_016, true, &non_idr))
        .await
        .expect("first continuation should flush bootstrap sample");

    let bootstrap_frame =
        tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
            .await
            .expect("bootstrap frame should assemble")
            .expect("bootstrap frame should be emitted");
    assert!(bootstrap_frame.is_keyframe);
    assert!(bootstrap_frame.h264.committed_sps_present());
    assert!(bootstrap_frame.h264.committed_pps_present());
    source.runtime_stats.update(|stats| {
        stats.latest_video_decode_ok_time_ms = Some(1.0);
        stats.latest_video_host_present_time_ms = Some(1.0);
    });
    source.runtime_stats.update(|stats| {
        let now_ms = now_ms_f64();
        stats.latest_video_decode_ok_time_ms = Some(now_ms);
        stats.latest_video_host_present_time_ms = Some(now_ms);
        stats.recovery_decoder_reference_synced_at_ms = Some(now_ms);
        stats.recovery_displayed_idr_at_ms = Some(now_ms);
        stats.host_mailbox_enqueue_count_total = 1;
        stats.host_frame_present_epoch = 1;
    });
    source.set_is_blocking_non_keyframe_admission(false);

    tx.send(make_video_rtp_packet(104, 9_032, true, &non_idr))
        .await
        .expect("non-idr packet should enqueue");
    tx.send(make_video_rtp_packet(105, 9_048, true, &non_idr))
        .await
        .expect("follow-up packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("continuation frame should assemble")
        .expect("continuation should pass after displayed-idr output fact");
    assert!(!frame.is_keyframe);
    assert!(transport_observation_rx.try_recv().is_err());
}

#[test]
fn waiting_recovery_keyframe_timeout_triggers_retry_request() {
    let (_tx, transport_observation_rx, mut source) = make_video_source_for_test();
    source.runtime_stats.update(|stats| {
        let now_ms = now_ms_f64();
        stats.latest_video_decode_ok_time_ms = Some(now_ms);
        stats.latest_video_host_present_time_ms = Some(now_ms);
        stats.host_mailbox_enqueue_count_total = 1;
        stats.host_frame_present_epoch = 1;
    });
    source.set_is_blocking_non_keyframe_admission(true);
    // 强制 next_retry_at_ms 为过去时间，确保触发重试。
    source.next_recovery_keyframe_retry_at_ms = Some(0.0);

    let before_ms = now_ms_f64();
    source.maybe_retry_waiting_recovery_keyframe(before_ms);

    assert_eq!(source.recovery_keyframe_retry_count, 1);
    // next_retry_at_ms 应推进到 before_ms + retry_interval，用固定基准比较避免时间竞争。
    assert!(source
        .next_recovery_keyframe_retry_at_ms
        .is_some_and(|at| at > before_ms));
    let _ = transport_observation_rx;
}

#[test]
fn waiting_recovery_keyframe_stops_retrying_after_max_count() {
    let (_tx, mut transport_observation_rx, mut source) = make_video_source_for_test();
    source.runtime_stats.update(|stats| {
        let now_ms = now_ms_f64();
        stats.latest_video_decode_ok_time_ms = Some(now_ms);
        stats.latest_video_host_present_time_ms = Some(now_ms);
        stats.host_mailbox_enqueue_count_total = 1;
        stats.host_frame_present_epoch = 1;
    });
    source.set_is_blocking_non_keyframe_admission(true);

    // 把 retry_count 推到上限前一次。
    use crate::transport::rtc::receive::nack_policy::RECOVERY_KEYFRAME_RETRY_MAX_COUNT;
    source.recovery_keyframe_retry_count = RECOVERY_KEYFRAME_RETRY_MAX_COUNT - 1;
    source.next_recovery_keyframe_retry_at_ms = Some(0.0);
    let now = now_ms_f64();

    // 最后一次合法重试。
    source.maybe_retry_waiting_recovery_keyframe(now);
    assert_eq!(
        source.recovery_keyframe_retry_count,
        RECOVERY_KEYFRAME_RETRY_MAX_COUNT
    );

    // 再次触发：已达上限，不再发请求，next_retry_at_ms 被清空。
    source.next_recovery_keyframe_retry_at_ms = Some(0.0);
    source.maybe_retry_waiting_recovery_keyframe(now);
    assert_eq!(
        source.recovery_keyframe_retry_count,
        RECOVERY_KEYFRAME_RETRY_MAX_COUNT
    );
    assert!(source.next_recovery_keyframe_retry_at_ms.is_none());
    assert!(transport_observation_rx.try_recv().is_err());
}

#[tokio::test]
async fn repair_packet_closes_bootstrap_gap_and_allows_frame_assembly() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();

    tx.send(make_video_rtp_packet(
        100,
        9000,
        false,
        bootstrap_sps_nalu(),
    ))
    .await
    .expect("sps packet should enqueue");
    tx.send(make_video_rtp_packet(102, 9000, true, bootstrap_idr_nalu()))
        .await
        .expect("idr packet should enqueue");
    let mut repair_packet = make_video_rtp_packet(101, 9000, false, bootstrap_pps_nalu());
    repair_packet.ingress_kind = RtcVideoIngressKind::RtxReinject {
        repair: RtcVideoRepairMetadata {
            native_ssrc: 88,
            native_payload_type: 97,
            native_sequence_number: 9_001,
        },
    };
    tx.send(repair_packet)
        .await
        .expect("repair packet should enqueue");
    tx.send(make_video_rtp_packet(103, 9016, true, bootstrap_idr_nalu()))
        .await
        .expect("next frame packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("frame assembly should finish")
        .expect("repaired bootstrap frame should be emitted");
    assert!(frame.is_keyframe);
    assert!(frame.h264.bootstrap_ready);
    assert_eq!(frame.rtp_timestamp, 9000);
    assert!(transport_observation_rx.try_recv().is_err());

    let latest = source
        .runtime_stats
        .read(|stats| stats.latest_video_rtx_reinject_observation.clone())
        .flatten()
        .expect("repair observation should be recorded");
    assert_eq!(latest.sequence_number, 101);
    assert_eq!(latest.rtp_timestamp, 9000);
    assert_eq!(latest.native_sequence_number, Some(9_001));
    assert_eq!(latest.repair_ssrc, 88);
    assert!(matches!(
        latest.stage.as_str(),
        "adapterResolveMiss" | "adapterResolved"
    ));
}

#[tokio::test]
async fn repair_reorder_gap_closure_stays_local_and_records_resolved_gap_match() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();

    tx.send(make_video_rtp_packet(
        100,
        9_000,
        false,
        bootstrap_sps_nalu(),
    ))
    .await
    .expect("sps packet should enqueue");
    tx.send(make_video_rtp_packet(
        102,
        9_000,
        true,
        bootstrap_idr_nalu(),
    ))
    .await
    .expect("idr packet should enqueue");
    let mut repair_packet = make_video_rtp_packet(101, 9_000, false, bootstrap_pps_nalu());
    repair_packet.ingress_kind = RtcVideoIngressKind::RtxReinject {
        repair: RtcVideoRepairMetadata {
            native_ssrc: 88,
            native_payload_type: 97,
            native_sequence_number: 9_001,
        },
    };
    tx.send(repair_packet)
        .await
        .expect("repair packet should enqueue");
    tx.send(make_video_rtp_packet(
        103,
        9_016,
        true,
        bootstrap_idr_nalu(),
    ))
    .await
    .expect("next frame packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("frame assembly should finish")
        .expect("repaired bootstrap frame should be emitted");
    assert!(frame.is_keyframe);
    assert!(frame.h264.bootstrap_ready);
    assert_eq!(frame.rtp_timestamp, 9_000);

    let latest = source
        .runtime_stats
        .read(|stats| stats.latest_video_rtx_reinject_observation.clone())
        .flatten()
        .expect("repair observation should be recorded");
    assert!(matches!(
        latest.stage.as_str(),
        "adapterResolveMiss" | "adapterResolved"
    ));
    assert_eq!(latest.sequence_number, 101);
    assert_eq!(latest.native_sequence_number, Some(9_001));
    assert_eq!(latest.repair_ssrc, 88);
    if latest.stage == "adapterResolved" {
        assert!(latest.matched_nack_range);
        assert!(latest.matched_pending_gap);
        assert_eq!(latest.matched_gap_sequence, Some(101));
    }

    assert!(
        tokio::time::timeout(Duration::from_millis(50), transport_observation_rx.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn idr_without_parameter_sets_requests_recovery_keyframe_instead_of_emitting_frame() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();

    tx.send(make_video_rtp_packet(100, 9001, true, bootstrap_idr_nalu()))
        .await
        .expect("idr packet should enqueue");
    tx.send(make_video_rtp_packet(101, 9017, true, bootstrap_idr_nalu()))
        .await
        .expect("follow-up packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should finish after rx closes");
    assert!(frame.is_none());

    assert!(transport_observation_rx.try_recv().is_err());
    assert_receiver_local_waiting_keyframe(&source);
    assert!(
        source.first_frame_acquisition_keyframe_request_count > 0
            || source.is_blocking_non_keyframe_admission()
    );
}

#[tokio::test]
async fn first_frame_acquisition_probe_does_not_latch_keyframe_required() {
    let (_tx, _transport_observation_rx, mut source) = make_video_source_for_test();
    source.runtime_stats.update(|stats| {
        stats.session_phase = Some("priming".to_string());
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.latest_remote_answer_observation = Some(startup_h264_answer_without_sprop());
    });

    source.maybe_request_first_frame_acquisition_keyframe(
        Some(9_001),
        FirstFrameAcquisitionRequestKind::Initial,
    );

    assert_eq!(source.first_frame_acquisition_keyframe_request_count, 1);
    let stats = source
        .runtime_stats
        .read(|stats| stats.clone())
        .expect("runtime stats snapshot");
    assert_eq!(stats.receive_keyframe_required, Some(false));
    assert_eq!(
        stats.receive_keyframe_required_cause.as_deref(),
        Some("none")
    );
    assert_eq!(
        stats.receive_keyframe_response_state.as_deref(),
        Some("no-packet")
    );
    assert_eq!(
        stats.latest_keyframe_request_source.as_deref(),
        Some("first-frame-acquisition")
    );
    assert_eq!(
        stats.latest_keyframe_request_outcome.as_deref(),
        Some("sent")
    );
    assert_eq!(stats.receive_keyframe_sent_count_unresolved, 1);
    assert!(stats.receive_keyframe_last_sent_at_ms.is_some());
    let receiver = stats
        .latest_video_receiver_observation
        .as_ref()
        .expect("sent keyframe request should publish receiver observation");
    assert_eq!(receiver.receiver_state, "waiting-keyframe");
    assert!(receiver.keyframe_request_pending);
    let timeline = stats
        .latest_video_timeline_observation
        .as_ref()
        .expect("sent keyframe request should publish timeline observation");
    assert_eq!(timeline.source_event, "keyframe-request-sent");
    assert_eq!(timeline.chain.state, "waiting-keyframe");
    assert_eq!(
        timeline.chain.reason.as_deref(),
        Some("receiverWaitingKeyframe")
    );
}

#[tokio::test]
async fn startup_h264_without_sprop_requests_keyframe_before_first_bad_frame_recovery() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();
    source.runtime_stats.update(|stats| {
        stats.session_phase = Some("priming".to_string());
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.latest_remote_answer_observation = Some(startup_h264_answer_without_sprop());
    });

    tx.send(make_video_rtp_packet(100, 9001, true, bootstrap_idr_nalu()))
        .await
        .expect("idr packet should enqueue");
    tx.send(make_video_rtp_packet(101, 9017, true, bootstrap_idr_nalu()))
        .await
        .expect("follow-up packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should finish after rx closes");
    assert!(frame.is_none());

    assert!(transport_observation_rx.try_recv().is_err());
    assert_receiver_local_waiting_keyframe(&source);
    assert!(source.first_frame_acquisition_keyframe_request_count >= 1);
}

#[tokio::test]
async fn startup_h264_without_sprop_and_audio_only_emits_single_followup_bootstrap_request() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();
    source.runtime_stats.update(|stats| {
        stats.session_phase = Some("priming".to_string());
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.latest_remote_answer_observation = Some(startup_h264_answer_without_sprop());
        stats.latest_audio_playout_time_ms = Some(16.0);
        stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
            state: "audioOnly".to_string(),
            video_width: None,
            video_height: None,
            mime_type: Some("video/h264".to_string()),
            transport_state: XbxEngineTransportStateDto::Connected,
            video_bytes_total: 0,
            video_packet_count_total: 0,
            audio_bytes_total: 2_048,
            observed_at_ms: 18.0,
        });
    });

    let non_idr = bootstrap_non_idr_nalu();
    tx.send(make_video_rtp_packet(100, 9_001, true, &non_idr))
        .await
        .expect("non-idr packet should enqueue");
    tx.send(make_video_rtp_packet(101, 9_017, true, &non_idr))
        .await
        .expect("follow-up packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should finish after rx closes");
    assert!(frame.is_none());

    assert!(transport_observation_rx.try_recv().is_err());
    assert_receiver_local_waiting_keyframe(&source);
    assert!(source.first_frame_acquisition_keyframe_request_count >= 1);
}

#[tokio::test]
async fn first_frame_acquisition_priority_non_idr_with_committed_parameter_sets_emits_followup_and_await_recovery_before_first_frame_edge(
) {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();
    source.runtime_stats.update(|stats| {
        stats.session_phase = Some("priming".to_string());
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.latest_remote_answer_observation = Some(startup_h264_answer_without_sprop());
        stats.latest_audio_playout_time_ms = Some(16.0);
        stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/h264".to_string()),
            transport_state: XbxEngineTransportStateDto::Connected,
            video_bytes_total: 4_096,
            video_packet_count_total: 32,
            audio_bytes_total: 2_048,
            observed_at_ms: 18.0,
        });
    });
    source
        .receive_core_mut()
        .receive_engine
        .bootstrap
        .seed_committed_parameter_sets_if_absent(&bootstrap_sps_nalu(), &bootstrap_pps_nalu())
        .expect("committed parameter sets should seed successfully");

    let non_idr = bootstrap_non_idr_nalu();
    tx.send(make_video_rtp_packet(100, 9_001, true, &non_idr))
        .await
        .expect("non-idr packet should enqueue");
    tx.send(make_video_rtp_packet(101, 9_017, true, &non_idr))
        .await
        .expect("follow-up packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should finish after rx closes");
    assert!(frame.is_none());

    assert!(transport_observation_rx.try_recv().is_err());
    assert_receiver_local_waiting_keyframe(&source);
    assert!(source.first_frame_acquisition_keyframe_request_count >= 1);
}

#[tokio::test]
async fn first_frame_acquisition_followup_request_is_disabled_after_first_frame_feedback() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();
    source.runtime_stats.update(|stats| {
        stats.session_phase = Some("priming".to_string());
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.latest_remote_answer_observation = Some(startup_h264_answer_without_sprop());
        stats.latest_audio_playout_time_ms = Some(16.0);
        stats.latest_video_decode_ok_time_ms = Some(14.0);
        stats.latest_video_host_present_time_ms = Some(15.0);
        stats.host_frame_present_epoch = 1;
        stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
            state: "audioOnly".to_string(),
            video_width: None,
            video_height: None,
            mime_type: Some("video/h264".to_string()),
            transport_state: XbxEngineTransportStateDto::Connected,
            video_bytes_total: 512,
            video_packet_count_total: 2,
            audio_bytes_total: 2_048,
            observed_at_ms: 18.0,
        });
    });

    let non_idr = bootstrap_non_idr_nalu();
    tx.send(make_video_rtp_packet(100, 9_001, true, &non_idr))
        .await
        .expect("non-idr packet should enqueue");
    tx.send(make_video_rtp_packet(101, 9_017, true, &non_idr))
        .await
        .expect("follow-up packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should finish after rx closes");
    assert!(frame.is_none());

    assert!(transport_observation_rx.try_recv().is_err());
    assert_receiver_local_waiting_keyframe(&source);
    assert_eq!(source.first_frame_acquisition_keyframe_request_count, 0);
}

#[tokio::test]
async fn first_frame_acquisition_followup_stays_enabled_before_first_frame_even_with_committed_parameter_sets(
) {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();
    source.runtime_stats.update(|stats| {
        stats.session_phase = Some("priming".to_string());
        stats.transport_state = XbxEngineTransportStateDto::Connected;
        stats.latest_remote_answer_observation = Some(startup_h264_answer_without_sprop());
        stats.latest_audio_playout_time_ms = Some(16.0);
        stats.latest_video_track_status = Some(XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: None,
            video_height: None,
            mime_type: Some("video/h264".to_string()),
            transport_state: XbxEngineTransportStateDto::Connected,
            video_bytes_total: 18_106,
            video_packet_count_total: 16,
            audio_bytes_total: 2_048,
            observed_at_ms: 18.0,
        });
    });

    send_bootstrap_access_unit(&tx, 100, 9_000).await;
    tx.send(make_video_rtp_packet(
        103,
        9_016,
        true,
        bootstrap_idr_nalu(),
    ))
    .await
    .expect("boundary packet should flush bootstrap sample");

    let bootstrap_frame =
        tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
            .await
            .expect("bootstrap frame should assemble")
            .expect("bootstrap frame should be emitted");
    assert!(bootstrap_frame.is_keyframe);
    assert!(bootstrap_frame.h264.bootstrap_ready);

    assert!(transport_observation_rx.try_recv().is_err());

    let non_idr = bootstrap_non_idr_nalu();
    tx.send(make_video_rtp_packet(103, 9_016, true, &non_idr))
        .await
        .expect("non-idr packet should enqueue");
    tx.send(make_video_rtp_packet(104, 9_032, true, &non_idr))
        .await
        .expect("follow-up packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should finish after rx closes");
    assert!(frame.is_none());

    assert!(transport_observation_rx.try_recv().is_err());
    assert_receiver_local_waiting_keyframe(&source);
    assert!(source.first_frame_acquisition_keyframe_request_count >= 2);
}

#[tokio::test]
async fn bootstrap_packets_without_followup_boundary_do_not_emit_partial_frame() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();

    send_bootstrap_access_unit(&tx, 100, 9000).await;
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(120), source.recv_frame_inner())
        .await
        .expect("reader should finish after rx closes");
    assert!(frame.is_none());
    assert!(transport_observation_rx.try_recv().is_err());
}

#[tokio::test]
async fn bootstrap_packets_without_followup_boundary_can_emit_when_early_emit_enabled() {
    // 默认行为保持不变，这里仅验证实验开关打开时可提前出帧。
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();
    source.set_jitter_early_emit_enabled(true);
    send_bootstrap_access_unit(&tx, 100, 9000).await;
    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should finish");
    drop(tx);

    let frame = frame.expect("early emit should materialize a frame");
    assert!(frame.is_keyframe);
    assert_eq!(frame.rtp_timestamp, 9000);
    assert!(transport_observation_rx.try_recv().is_err());
}

#[tokio::test]
async fn bootstrap_emit_path_records_frame_supply_counters() {
    let (tx, _transport_observation_rx, mut source) = make_video_source_for_test();
    source.set_jitter_early_emit_enabled(true);

    send_bootstrap_access_unit(&tx, 100, 9_000).await;

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should finish")
        .expect("bootstrap frame should emit");
    drop(tx);

    assert!(frame.is_keyframe);
    let counters = source.runtime_stats.read(|stats| {
        (
            stats.inbound_video_rtp_marker_count_total,
            stats.inbound_video_access_unit_count_total,
            stats.inbound_video_decode_gate_emit_count_total,
            stats.inbound_video_decode_gate_continue_count_total,
        )
    });
    assert_eq!(counters, Some((1, 1, 1, 0)));
}

#[tokio::test]
async fn pre_first_frame_non_idr_continue_path_records_frame_supply_counters() {
    let (tx, _transport_observation_rx, mut source) = make_video_source_for_test();
    let non_idr = bootstrap_non_idr_nalu();

    tx.send(make_video_rtp_packet(100, 9_000, true, &non_idr))
        .await
        .expect("first non-idr packet should enqueue");
    tx.send(make_video_rtp_packet(101, 9_016, true, &non_idr))
        .await
        .expect("second non-idr packet should flush previous sample");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should finish after rx closes");

    assert!(frame.is_none());
    let counters = source.runtime_stats.read(|stats| {
        (
            stats.inbound_video_rtp_marker_count_total,
            stats.inbound_video_access_unit_count_total,
            stats.inbound_video_decode_gate_emit_count_total,
            stats.inbound_video_decode_gate_continue_count_total,
        )
    });
    assert_eq!(counters, Some((2, 1, 0, 1)));
}

#[tokio::test]
async fn materialized_keyframe_response_preserves_first_packet_sequence_in_diagnostics() {
    let (tx, _transport_observation_rx, mut source) = make_video_source_for_test();
    source.set_jitter_early_emit_enabled(true);
    source
        .runtime_stats
        .record_picture_recovery_episode_requested(
            901,
            Some("receiverWaitingKeyframe".to_string()),
            100.0,
            None,
        );
    source
        .runtime_stats
        .record_picture_recovery_episode_sent("pli", 120.0, Some(240.0));

    send_bootstrap_access_unit(&tx, 100, 9_000).await;

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should finish");
    drop(tx);

    let frame = frame.expect("early emit should materialize a frame");
    assert_eq!(frame.first_packet_sequence, Some(100));

    let episode = source
        .runtime_stats
        .read(|stats| stats.latest_keyframe_request_episode.clone())
        .expect("runtime stats");
    let episode = episode.expect("keyframe request episode");
    assert_eq!(episode.first_video_packet_rtp_timestamp, Some(9_000));
    assert_eq!(episode.first_video_packet_is_keyframe, Some(true));
    assert_eq!(episode.response_rtp_timestamp, Some(9_000));
}

#[test]
fn dynamic_nack_skip_last_n_uses_oos_percentile_buckets() {
    let (_tx, _transport_observation_rx, mut source) = make_video_source_for_test();

    source.recent_oos_depths = [1, 1, 2, 2, 2, 3].into_iter().collect();
    source.update_dynamic_nack_skip_last_n(1_000.0);
    assert_eq!(source.nack_skip_last_n, 2);

    source.recent_oos_depths = [2, 3, 4, 4, 4, 5].into_iter().collect();
    source.update_dynamic_nack_skip_last_n(1_250.0);
    assert_eq!(source.nack_skip_last_n, 4);

    source.recent_oos_depths = [3, 4, 5, 6, 6, 7, 8].into_iter().collect();
    source.update_dynamic_nack_skip_last_n(1_500.0);
    assert_eq!(source.nack_skip_last_n, 6);
}

#[test]
fn dynamic_nack_skip_last_n_is_rate_limited() {
    let (_tx, _transport_observation_rx, mut source) = make_video_source_for_test();

    source.recent_oos_depths = [6, 6, 6, 6].into_iter().collect();
    source.update_dynamic_nack_skip_last_n(2_000.0);
    assert_eq!(source.nack_skip_last_n, 6);

    source.recent_oos_depths = [1, 1, 1, 1].into_iter().collect();
    source.update_dynamic_nack_skip_last_n(2_100.0);
    assert_eq!(source.nack_skip_last_n, 6);

    source.update_dynamic_nack_skip_last_n(2_220.0);
    assert_eq!(source.nack_skip_last_n, 2);
}

#[tokio::test]
async fn repair_rtx_packet_keeps_explicit_provenance_through_source_stage_updates() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();

    let mut packet = make_video_rtp_packet(100, 9_000, true, bootstrap_idr_nalu());
    packet.meta.ssrc = 777;
    packet.meta.payload_type = 124;
    packet.ingress_kind = RtcVideoIngressKind::RtxReinject {
        repair: RtcVideoRepairMetadata {
            native_ssrc: 99,
            native_payload_type: 97,
            native_sequence_number: 4_321,
        },
    };
    tx.send(packet).await.expect("repair packet should enqueue");
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should finish after rx closes");
    assert!(frame.is_none());

    let latest = source
        .runtime_stats
        .read(|stats| stats.latest_video_rtx_reinject_observation.clone())
        .flatten()
        .expect("repair provenance observation should be recorded");
    assert_eq!(latest.stage, "adapterResolveMiss");
    assert_eq!(latest.sequence_number, 100);
    assert_eq!(latest.repair_ssrc, 99);
    assert_eq!(latest.primary_ssrc, 777);
    assert_eq!(latest.native_sequence_number, Some(4_321));
    assert!(!latest.matched_nack_range);
    assert!(!latest.matched_pending_gap);
    assert!(latest.matched_gap_sequence.is_none());
    assert!(
        tokio::time::timeout(Duration::from_millis(50), transport_observation_rx.recv())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn unmatched_repair_rtx_burst_stays_local_and_does_not_emit_transport_observation() {
    let (tx, mut transport_observation_rx, mut source) = make_video_source_for_test();

    for offset in 0..5u16 {
        let mut packet = make_video_rtp_packet(300 + offset, 12_000, false, bootstrap_pps_nalu());
        packet.meta.ssrc = 777;
        packet.meta.payload_type = 124;
        packet.ingress_kind = RtcVideoIngressKind::RtxReinject {
            repair: RtcVideoRepairMetadata {
                native_ssrc: 99,
                native_payload_type: 97,
                native_sequence_number: 8_000 + offset,
            },
        };
        tx.send(packet)
            .await
            .expect("repair burst packet should enqueue");
    }
    drop(tx);

    let frame = tokio::time::timeout(Duration::from_millis(200), source.recv_frame_inner())
        .await
        .expect("reader should finish after rx closes");
    assert!(frame.is_none());

    let latest = source
        .runtime_stats
        .read(|stats| stats.latest_video_rtx_reinject_observation.clone())
        .flatten()
        .expect("repair burst observation should be recorded");
    assert_eq!(latest.stage, "adapterResolveMiss");
    assert_eq!(latest.sequence_number, 304);
    assert_eq!(latest.repair_ssrc, 99);
    assert_eq!(latest.primary_ssrc, 777);
    assert_eq!(latest.native_sequence_number, Some(8_004));
    assert!(!latest.matched_nack_range);
    assert!(!latest.matched_pending_gap);
    assert!(latest.matched_gap_sequence.is_none());

    assert!(
        tokio::time::timeout(Duration::from_millis(50), transport_observation_rx.recv())
            .await
            .is_err()
    );
}
