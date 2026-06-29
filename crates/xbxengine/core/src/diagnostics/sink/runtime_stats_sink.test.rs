use std::sync::{Arc, Mutex};

use crate::{
    XbxEngineH264InspectionObservation, XbxEngineMediaRuntimeStats,
    XbxEngineVideoTimelineChainSnapshot, XbxEngineVideoTimelineObservation,
};

use super::RuntimeStatsSink;

#[test]
fn unsolicited_bootstrap_idr_records_response_without_sent_episode() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.update(|stats| {
        stats.latest_h264_inspection_observation = Some(XbxEngineH264InspectionObservation {
            frame_rtp_timestamp: Some(9_000),
            is_idr: true,
            bootstrap_ready: true,
            admission_accepted: true,
            parameter_sets_changed: true,
            config_changed: true,
            observed_at_ms: 100.0,
            ..Default::default()
        });
    });
    sink.record_picture_recovery_episode_response_observed(
        120.0,
        Some(9_000),
        true,
        "firstKeyframeAccepted",
        Some(2),
        None,
        false,
        false,
    );

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let transition = stats
        .latest_picture_recovery_transition_observation
        .as_ref()
        .expect("unsolicited bootstrap transition");
    assert_eq!(transition.phase, "ResponseObserved");
    assert_eq!(transition.episode_id, None);
    assert_eq!(transition.rtp_timestamp, Some(9_000));
    assert_eq!(
        transition.from_phase.as_deref(),
        Some("BootstrapUnsolicited")
    );
}

#[test]
fn displayed_idr_fact_commits_fresh_anchor_on_host_visible() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_pending_displayed_idr_rtp(9_000);
    sink.seed_decoder_reference_sync_for_pending_idr(9_000, 150.0);
    sink.record_displayed_idr_fact(160.0, 9_000, Some(42));

    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.video_anchor_clean_epoch, Some(1));
    assert_eq!(
        stats.video_anchor_clean_source_event.as_deref(),
        Some("displayed-idr")
    );
    assert_eq!(stats.recovery_displayed_idr_rtp, Some(9_000));
    assert_eq!(stats.recovery_fresh_anchor_recovered_at_ms, Some(160.0));
    let transition = stats
        .latest_picture_recovery_transition_observation
        .as_ref()
        .expect("fresh anchor transition");
    assert_eq!(transition.phase, "FreshAnchorRecovered");
}

#[test]
fn repeated_begin_transport_recovery_episode_is_idempotent() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    assert_eq!(sink.begin_transport_recovery_episode(10.0), 1);
    assert_eq!(sink.begin_transport_recovery_episode(20.0), 1);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.transport_recovery_epoch, 1);
    assert!(stats.transport_recovery_episode_active);
    assert_eq!(stats.transport_recovery_episode_opened_at_ms, Some(10.0));
}

#[test]
fn clean_anchor_keeps_transport_recovery_episode_open_until_stable_settle() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_pending_displayed_idr_rtp(1);
    sink.seed_decoder_reference_sync_for_pending_idr(1, 15.0);
    sink.record_displayed_idr_fact(20.0, 1, None);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.video_anchor_clean_epoch, Some(1));
    assert_eq!(stats.video_anchor_clean_observed_at_ms, Some(20.0));
    assert_eq!(
        stats.video_anchor_clean_source_event.as_deref(),
        Some("displayed-idr")
    );
    assert!(stats.transport_recovery_episode_active);
    assert_eq!(stats.transport_recovery_episode_closed_at_ms, None);
    assert_eq!(stats.transport_recovery_episode_close_reason, None);
}

#[test]
fn clean_anchor_sets_retired_at_ms_on_latest_keyframe_episode() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_picture_recovery_episode_requested(
        7,
        Some("receiverWaitingKeyframe".to_string()),
        15.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 16.0, None);
    sink.record_picture_recovery_episode_response_observed(
        17.0,
        Some(77_001),
        true,
        "firstAcceptedIdr",
        Some(1),
        None,
        false,
        false,
    );
    sink.record_pending_displayed_idr_rtp(77_001);
    sink.seed_decoder_reference_sync_for_pending_idr(77_001, 15.0);
    sink.record_displayed_idr_fact(20.0, 77_001, None);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.video_anchor_clean_epoch, Some(1));
    assert_eq!(stats.recovery_fresh_anchor_recovered_at_ms, Some(20.0));
}

#[test]
fn advancing_transport_recovery_episode_clears_stale_anchor() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_pending_displayed_idr_rtp(77_001);
    sink.seed_decoder_reference_sync_for_pending_idr(77_001, 15.0);
    sink.record_displayed_idr_fact(20.0, 77_001, None);
    sink.update(|stats| {
        stats.latest_video_timeline_observation = Some(XbxEngineVideoTimelineObservation {
            observation_id: 7,
            source_event: "stale-gap".to_string(),
            gap: None,
            frame: None,
            chain: XbxEngineVideoTimelineChainSnapshot {
                state: "broken".to_string(),
                reason: Some("staleReferenceGap".to_string()),
                chain_break_evidence: Some("oldEpoch".to_string()),
                observed_at_ms: 20.0,
            },
            observed_at_ms: 20.0,
        });
        stats.reference_chain_state = Some("need-keyframe".to_string());
        stats.reference_chain_state_cause = Some("staleReferenceGap".to_string());
        stats.reference_chain_decoder_reference_synced = Some(true);
        stats.reference_chain_bootstrap_ready = Some(true);
        stats.reference_chain_has_active_gap = Some(true);
        stats.reference_chain_nack_exhausted = Some(true);
        stats.reference_chain_submit_age_ms = Some(1200.0);
        stats.latest_reference_chain_observation_source = Some("ledger".to_string());
        stats.latest_reference_chain_sparse_must_idr_mismatch = Some(true);
        stats.receive_sparse_must_idr_mismatch_total = 1690;
        stats.reference_stats_fallback_total = 3;
    });
    assert_eq!(sink.advance_transport_recovery_episode(30.0), 2);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.transport_recovery_epoch, 2);
    assert!(stats.transport_recovery_episode_active);
    assert_eq!(stats.transport_recovery_episode_opened_at_ms, Some(30.0));
    assert_eq!(stats.video_anchor_clean_epoch, None);
    assert_eq!(stats.video_anchor_clean_observed_at_ms, None);
    assert_eq!(stats.video_anchor_clean_source_event, None);
    assert_eq!(stats.recovery_displayed_idr_at_ms, None);
    assert!(stats.receive_display_state.is_none());
    assert!(stats.receive_keyframe_response_state.is_none());
    assert!(stats.latest_h264_inspection_observation.is_none());
    assert!(stats.receive_keyframe_last_sent_at_ms.is_none());
    assert!(stats.recovery_decoder_reference_synced_at_ms.is_none());
    assert!(stats.latest_video_decode_ok_time_ms.is_none());
    assert!(stats.latest_video_decode_ok_rtp_timestamp.is_none());
    assert!(stats.latest_video_timeline_observation.is_none());
    assert!(stats.reference_chain_state.is_none());
    assert!(stats.reference_chain_state_cause.is_none());
    assert!(stats.reference_chain_decoder_reference_synced.is_none());
    assert!(stats.reference_chain_bootstrap_ready.is_none());
    assert!(stats.reference_chain_has_active_gap.is_none());
    assert!(stats.reference_chain_nack_exhausted.is_none());
    assert!(stats.reference_chain_submit_age_ms.is_none());
    assert!(stats.latest_reference_chain_observation_source.is_none());
    assert!(stats
        .latest_reference_chain_sparse_must_idr_mismatch
        .is_none());
    assert_eq!(stats.receive_sparse_must_idr_mismatch_total, 0);
    assert_eq!(stats.reference_stats_fallback_total, 0);
}

#[test]
fn epoch_advance_clears_stale_decode_before_new_round_can_complete() {
    use crate::transport::rtc::recovery::contract::receive_picture_recovery_complete_from_stats;

    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.update(|stats| {
        stats.recovery_decoder_reference_synced_at_ms = Some(15.0);
        stats.latest_video_decode_ok_time_ms = Some(15.0);
        stats.latest_video_decode_ok_rtp_timestamp = Some(77_001);
    });
    assert_eq!(sink.advance_transport_recovery_episode(30.0), 2);
    sink.update(|stats| {
        stats.receive_keyframe_response_state = Some("usable-idr".to_string());
        stats.video_anchor_clean_epoch = Some(2);
        stats.video_anchor_clean_observed_at_ms = Some(35.0);
    });

    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert!(stats.recovery_decoder_reference_synced_at_ms.is_none());
    assert!(!receive_picture_recovery_complete_from_stats(&stats));
}

#[test]
fn lifecycle_recovering_completes_active_episode() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.complete_transport_recovery_for_lifecycle_recovering(40.0);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert!(!stats.transport_recovery_episode_active);
    assert_eq!(stats.transport_recovery_episode_closed_at_ms, Some(40.0));
    assert_eq!(
        stats.transport_recovery_episode_close_reason.as_deref(),
        Some("lifecycleRecovering")
    );
    assert_eq!(stats.video_anchor_clean_epoch, None);
    assert_eq!(stats.video_anchor_clean_observed_at_ms, None);
    assert_eq!(stats.video_anchor_clean_source_event, None);
}

#[test]
fn playback_recovered_rejects_retained_host_present_from_previous_episode() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(1_000.0);
    sink.update(|stats| {
        stats.display_age_ms = Some(1_200.0);
        stats.last_displayed_at_ms = Some(900.0);
        stats.latest_video_host_present_time_ms = Some(900.0);
        stats.last_displayed_frame_seq = Some(407);
        stats.video_owner_state = Some("supply-starved".to_string());
        stats.video_owner_reason = Some("displaySupplyCritical".to_string());
    });
    sink.record_playback_recovered_fact(900.0, 15.0);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.recovery_playback_recovered_at_ms, None);
    assert!(stats
        .latest_picture_recovery_transition_observation
        .is_none());
}

#[test]
fn playback_recovered_accepts_current_fresh_host_present() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(1_000.0);
    sink.update(|stats| {
        stats.display_age_ms = Some(16.0);
        stats.last_displayed_at_ms = Some(1_020.0);
        stats.latest_video_host_present_time_ms = Some(1_020.0);
        stats.last_displayed_frame_seq = Some(408);
        stats.video_owner_state = Some("stable-serving".to_string());
        stats.video_owner_reason = Some("steady".to_string());
    });
    sink.record_playback_recovered_fact(1_020.0, 60.0);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.recovery_playback_recovered_at_ms, Some(1_020.0));
    let transition = stats
        .latest_picture_recovery_transition_observation
        .as_ref()
        .expect("playback recovered transition");
    assert_eq!(transition.phase, "PlaybackRecovered");
    assert_eq!(transition.recovery_epoch, Some(1));
    assert_eq!(transition.frame_seq, Some(408));
}

#[test]
fn stable_settle_completes_active_episode_after_clean_anchor() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_pending_displayed_idr_rtp(1);
    sink.seed_decoder_reference_sync_for_pending_idr(1, 15.0);
    sink.record_displayed_idr_fact(20.0, 1, None);
    sink.complete_transport_recovery_after_stable_settle(40.0);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert!(!stats.transport_recovery_episode_active);
    assert_eq!(stats.transport_recovery_episode_closed_at_ms, Some(40.0));
    assert_eq!(
        stats.transport_recovery_episode_close_reason.as_deref(),
        Some("stableServingSettled")
    );
    assert_eq!(stats.video_anchor_clean_epoch, Some(1));
}

#[test]
fn stale_epoch_clean_anchor_submission_does_not_promote_current_transport_recovery() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.advance_transport_recovery_episode(20.0);
    sink.record_displayed_idr_fact(30.0, 9_000, None);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.transport_recovery_epoch, 2);
    assert_eq!(stats.video_anchor_clean_epoch, None);
    assert_eq!(stats.recovery_displayed_idr_rtp, None);
    assert_eq!(stats.recovery_fresh_anchor_recovered_at_ms, None);
    assert_eq!(stats.recovery_pending_displayed_idr_rtp, None);
}

#[test]
fn keyframe_request_episode_packet_seen_and_decoded_resolve_verdict() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.record_picture_recovery_episode_requested(
        77,
        Some("receiverWaitingKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 120.0, Some(200.0));
    sink.record_picture_recovery_episode_packet_seen(150.0, Some(123456789), true, Some(321));
    sink.record_picture_recovery_episode_decoded(160.0, 123456789, 42);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let episode = stats
        .latest_keyframe_request_episode
        .as_ref()
        .expect("episode should exist");
    assert_eq!(episode.status, "succeeded");
    assert_eq!(episode.request_kind.as_deref(), Some("pli"));
    assert_eq!(episode.sent_at_ms, Some(120.0));
    assert_eq!(episode.deadline_at_ms, Some(200.0));
    assert_eq!(episode.first_keyframe_packet_at_ms, Some(150.0));
    assert_eq!(episode.first_keyframe_decoded_at_ms, Some(160.0));
    assert_eq!(episode.response_rtp_timestamp, Some(123456789));
    assert_eq!(episode.response_frame_seq, Some(42));
    assert_eq!(
        episode.response_verdict.as_deref(),
        Some("cleanAnchorCommitted")
    );
    assert!(episode.transport_detail.as_deref().is_some_and(|detail| {
        detail.contains("firstFrameLatencyTrace")
            && detail.contains("controlReadyToPliSentMs=none")
            && detail.contains("pliSentToFirstIdrPacketMs=30.0")
            && detail.contains("firstIdrPacketToFirstDecodeMs=10.0")
    }));
    assert_eq!(
        stats.latest_observation_label.as_deref(),
        Some("keyframeRequestEpisodeDecoded")
    );
}

#[test]
fn transport_await_refresh_reuses_same_episode_within_active_recovery() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_picture_recovery_episode_requested(
        1001,
        Some("receiverWaitingKeyframe".to_string()),
        20.0,
        Some(120.0),
    );
    sink.record_picture_recovery_episode_sent("pli", 21.0, Some(121.0));
    sink.record_picture_recovery_episode_requested(
        1002,
        Some("receiverWaitingKeyframe".to_string()),
        40.0,
        Some(140.0),
    );
    sink.record_picture_recovery_episode_sent("pli", 41.0, Some(141.0));
    sink.record_picture_recovery_episode_packet_seen(55.0, Some(777_111), true, Some(88));
    sink.record_picture_recovery_episode_decoded(60.0, 777_111, 9001);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let episode = stats
        .latest_keyframe_request_episode
        .as_ref()
        .expect("episode should exist");
    assert_eq!(episode.episode_id, 1001);
    assert_eq!(episode.requested_at_ms, 20.0);
    assert_eq!(episode.sent_at_ms, Some(21.0));
    assert_eq!(episode.deadline_at_ms, Some(120.0));
    assert_eq!(episode.first_keyframe_packet_at_ms, Some(55.0));
    assert_eq!(episode.first_keyframe_decoded_at_ms, Some(60.0));
    assert_eq!(episode.response_rtp_timestamp, Some(777_111));
    assert_eq!(episode.response_frame_seq, Some(9001));
}

#[test]
fn advancing_transport_recovery_retires_previous_transport_await_episode() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_picture_recovery_episode_requested(
        2001,
        Some("receiverWaitingKeyframe".to_string()),
        20.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 21.0, Some(121.0));
    sink.advance_transport_recovery_episode(50.0);
    sink.record_picture_recovery_episode_requested(
        2002,
        Some("receiverWaitingKeyframe".to_string()),
        60.0,
        None,
    );

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let latest = stats
        .latest_keyframe_request_episode
        .as_ref()
        .expect("latest episode should exist");
    assert_eq!(latest.episode_id, 2002);
    let previous = stats
        .recent_keyframe_request_episodes
        .iter()
        .find(|episode| episode.episode_id == 2001)
        .expect("previous episode should remain in recent list");
    assert_eq!(previous.retired_at_ms, Some(50.0));
    assert_eq!(
        previous.status_detail.as_deref(),
        Some("supersededByNewRecoveryEpoch")
    );
}

#[test]
fn keyframe_request_episode_response_observed_tracks_non_keyframe_then_rejected_keyframe() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.record_picture_recovery_episode_requested(
        90,
        Some("receiverWaitingKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 120.0, Some(200.0));
    sink.record_picture_recovery_episode_response_observed(
        140.0,
        Some(123),
        false,
        "firstResponseNonKeyframe",
        Some(111),
        Some(5),
        true,
        false,
    );
    sink.record_picture_recovery_episode_response_observed(
        170.0,
        Some(456),
        true,
        "bootstrapMissingSps",
        Some(222),
        Some(7),
        true,
        true,
    );

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let episode = stats
        .latest_keyframe_request_episode
        .as_ref()
        .expect("episode should exist");
    assert_eq!(episode.status, "response-observed");
    assert_eq!(
        episode.status_detail.as_deref(),
        Some("bootstrapMissingSps")
    );
    assert_eq!(episode.first_video_packet_at_ms, Some(140.0));
    assert_eq!(episode.first_video_packet_rtp_timestamp, Some(123));
    assert_eq!(episode.first_video_packet_is_keyframe, Some(false));
    assert_eq!(episode.first_keyframe_packet_at_ms, Some(170.0));
    assert_eq!(episode.response_rtp_timestamp, Some(456));
    assert_eq!(episode.response_verdict.as_deref(), Some("pending"));
    assert_eq!(
        stats.latest_observation_label.as_deref(),
        Some("keyframeRequestEpisodeResponseObserved")
    );
    let summary = stats
        .latest_observation_summary
        .as_deref()
        .expect("response-observed summary");
    assert!(summary.contains("detail=bootstrapMissingSps"));
    assert!(summary.contains("sentToFirstPacketMs=50.0"));
    assert!(summary.contains("firstVideoPacketIsKeyframe=false"));
    assert!(summary.contains("firstVideoPacketSeq=111"));
    assert!(summary.contains("firstKeyframePacketSeq=222"));
    assert!(summary.contains("oosDepthP75=7"));
    assert!(summary.contains("firstKeyframeArrivalLagMs=50.0"));
    assert!(summary.contains("headMissingActive=true"));
    assert!(summary.contains("gapExpiredBeforeKeyframe=true"));
}

#[test]
fn newer_keyframe_response_advances_transport_await_owner_frame() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_picture_recovery_episode_requested(
        9010,
        Some("receiverWaitingKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
    sink.record_picture_recovery_episode_response_observed(
        150.0,
        Some(10_001),
        true,
        "firstAcceptedIdr",
        Some(11),
        None,
        false,
        false,
    );
    sink.record_picture_recovery_episode_response_observed(
        180.0,
        Some(10_101),
        true,
        "ownerFrameAdvanced",
        Some(22),
        None,
        false,
        false,
    );

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let episode = stats
        .latest_keyframe_request_episode
        .as_ref()
        .expect("episode should exist");
    assert_eq!(episode.response_rtp_timestamp, Some(10_101));
    assert_eq!(episode.first_keyframe_packet_at_ms, Some(180.0));
    assert_eq!(episode.first_keyframe_decoded_at_ms, None);
    assert_eq!(episode.status_detail.as_deref(), Some("ownerFrameAdvanced"));
    assert_eq!(episode.response_verdict.as_deref(), Some("pending"));
}

#[test]
fn serviceable_continuation_prevents_transport_await_owner_frame_advance() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_picture_recovery_episode_requested(
        9016,
        Some("receiverWaitingKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
    sink.record_picture_recovery_episode_response_observed(
        150.0,
        Some(10_001),
        true,
        "firstAcceptedIdr",
        Some(11),
        None,
        false,
        false,
    );
    sink.record_h264_inspection_observation(crate::XbxEngineH264InspectionObservation {
        observation_id: 1,
        observed_at_ms: 170.0,
        frame_rtp_timestamp: Some(10_333),
        admission_accepted: true,
        bootstrap_ready: false,
        bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
        committed_sps_present: true,
        committed_pps_present: true,
        delta_continuation_ready: true,
        continuation_verdict: Some("receiverLocalContinuation".to_string()),
        ..Default::default()
    });
    sink.record_picture_recovery_episode_response_observed(
        180.0,
        Some(10_101),
        true,
        "ownerFrameAdvanced",
        Some(22),
        None,
        false,
        false,
    );

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let episode = stats
        .latest_keyframe_request_episode
        .as_ref()
        .expect("episode should exist");
    assert_eq!(episode.response_rtp_timestamp, Some(10_001));
    assert_eq!(episode.first_keyframe_packet_at_ms, Some(150.0));
    assert_eq!(episode.status_detail.as_deref(), Some("firstAcceptedIdr"));
}

#[test]
fn stale_owner_decoded_does_not_update_current_transport_await_episode() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_picture_recovery_episode_requested(
        9011,
        Some("receiverWaitingKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
    sink.record_picture_recovery_episode_response_observed(
        150.0,
        Some(10_001),
        true,
        "firstAcceptedIdr",
        Some(11),
        None,
        false,
        false,
    );
    sink.record_picture_recovery_episode_response_observed(
        180.0,
        Some(10_101),
        true,
        "ownerFrameAdvanced",
        Some(22),
        None,
        false,
        false,
    );
    sink.record_picture_recovery_episode_decoded(210.0, 10_001, 77);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let episode = stats
        .latest_keyframe_request_episode
        .as_ref()
        .expect("episode should exist");
    assert_eq!(episode.response_rtp_timestamp, Some(10_101));
    assert_eq!(episode.first_keyframe_decoded_at_ms, None);
    assert_eq!(
        stats.latest_observation_label.as_deref(),
        Some("keyframeRequestEpisodeDecodedIgnored")
    );
}

#[test]
fn decoded_usable_idr_establishes_media_recovery_before_displayed_pending_idr() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_picture_recovery_episode_requested(
        9012,
        Some("receiverWaitingKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
    sink.record_picture_recovery_episode_response_observed(
        150.0,
        Some(10_001),
        true,
        "firstAcceptedIdr",
        Some(11),
        None,
        false,
        false,
    );
    sink.record_picture_recovery_episode_response_observed(
        180.0,
        Some(10_101),
        true,
        "ownerFrameAdvanced",
        Some(22),
        None,
        false,
        false,
    );
    sink.record_picture_recovery_episode_decoded(200.0, 10_101, 22);
    sink.record_pending_displayed_idr_rtp(10_101);

    {
        let stats = runtime_stats.lock().expect("runtime stats lock");
        assert_eq!(stats.video_anchor_clean_epoch, Some(1));
        assert_eq!(
            stats.video_anchor_clean_source_event.as_deref(),
            Some("decoded-usable-idr")
        );
        assert_eq!(stats.recovery_pending_displayed_idr_rtp, Some(10_101));
        assert_eq!(stats.recovery_fresh_anchor_recovered_at_ms, Some(200.0));
    }

    sink.record_displayed_idr_fact(250.0, 10_101, None);
    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.video_anchor_clean_epoch, Some(1));
    assert_eq!(stats.recovery_fresh_anchor_recovered_at_ms, Some(200.0));
    assert_eq!(stats.recovery_displayed_idr_rtp, Some(10_101));
    assert_eq!(
        stats.receive_display_state.as_deref(),
        Some("display-stable")
    );
    let transition = stats
        .latest_picture_recovery_transition_observation
        .as_ref()
        .expect("display stable transition");
    assert_eq!(transition.to_phase, "DisplayStable");
    assert_eq!(
        transition.from_phase.as_deref(),
        Some("CleanAnchorCommitted")
    );
}

#[test]
fn decoded_keyframe_media_recovery_survives_host_display_lag() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_picture_recovery_episode_requested(
        9013,
        Some("receiverWaitingKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
    sink.record_picture_recovery_episode_response_observed(
        150.0,
        Some(10_201),
        true,
        "firstAcceptedIdr",
        Some(11),
        None,
        false,
        false,
    );
    sink.record_picture_recovery_episode_decoded(180.0, 10_201, 77);
    sink.record_pending_displayed_idr_rtp(10_201);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.video_anchor_clean_epoch, Some(1));
    assert_eq!(
        stats.video_anchor_clean_source_event.as_deref(),
        Some("decoded-usable-idr")
    );
    assert_eq!(stats.recovery_pending_displayed_idr_rtp, Some(10_201));
    assert_eq!(stats.recovery_fresh_anchor_recovered_at_ms, Some(180.0));
}

#[test]
fn displayed_idr_fact_does_not_commit_after_recovery_epoch_advances() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_picture_recovery_episode_requested(
        9014,
        Some("receiverWaitingKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
    sink.record_picture_recovery_episode_response_observed(
        150.0,
        Some(10_301),
        true,
        "firstAcceptedIdr",
        Some(11),
        None,
        false,
        false,
    );
    sink.record_picture_recovery_episode_decoded(180.0, 10_301, 77);
    sink.record_pending_displayed_idr_rtp(10_301);
    assert_eq!(sink.advance_transport_recovery_episode(230.0), 2);

    sink.record_displayed_idr_fact(240.0, 10_301, None);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(stats.transport_recovery_epoch, 2);
    assert_eq!(stats.video_anchor_clean_epoch, None);
    assert_eq!(stats.recovery_pending_displayed_idr_rtp, None);
}

#[test]
fn keyframe_request_episode_decoded_after_timeout_clears_missed_verdict() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.record_picture_recovery_episode_requested(
        901,
        Some("receiverWaitingKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 120.0, Some(200.0));
    sink.record_picture_recovery_episode_timeout(200.0);

    {
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let episode = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode");
        assert_eq!(episode.response_verdict.as_deref(), Some("missed"));
    }

    sink.record_picture_recovery_episode_decoded(210.0, 999_001, 1001);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let episode = stats
        .latest_keyframe_request_episode
        .as_ref()
        .expect("episode");
    assert_eq!(episode.status, "succeeded");
    assert_eq!(
        episode.response_verdict.as_deref(),
        Some("cleanAnchorCommitted")
    );
    assert_eq!(episode.lifecycle_phase.as_deref(), Some("success"));
}

#[test]
fn keyframe_request_episode_timeout_skipped_when_transport_clean_anchor_already_observed() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_picture_recovery_episode_requested(
        902,
        Some("receiverWaitingKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 120.0, Some(500.0));
    sink.record_pending_displayed_idr_rtp(1);
    sink.seed_decoder_reference_sync_for_pending_idr(1, 170.0);
    sink.record_displayed_idr_fact(180.0, 1, None);

    sink.record_picture_recovery_episode_timeout(600.0);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let episode = stats
        .latest_keyframe_request_episode
        .as_ref()
        .expect("episode");
    assert_eq!(episode.status, "succeeded");
    assert_eq!(
        episode.response_verdict.as_deref(),
        Some("cleanAnchorCommitted")
    );
}

#[test]
fn first_frame_latency_prefers_clean_anchor_gap_over_missing_pli_when_decode_exists() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_picture_recovery_episode_requested(
        903,
        Some("receiverWaitingKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_packet_seen(150.0, Some(123456789), true, Some(321));
    sink.record_picture_recovery_episode_decoded(160.0, 123456789, 42);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let observation = stats
        .latest_first_frame_latency_observation
        .as_ref()
        .expect("first frame latency observation");
    assert_eq!(
        observation.terminal_phase.as_deref(),
        Some("CleanAnchorCommitted")
    );
    assert_eq!(
        observation.incomplete_reason.as_deref(),
        Some("noDisplayStable")
    );
    assert_eq!(observation.control_ready_to_pli_sent_ms, None);
    assert_eq!(observation.pli_sent_to_first_idr_packet_ms, None);
    assert_eq!(observation.first_idr_packet_to_first_decode_ms, Some(10.0));
}

#[test]
fn first_frame_latency_observation_records_complete_stage_breakdown() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    RuntimeStatsSink::update_shared(&runtime_stats, |stats| {
        stats.control_ready_at_ms = Some(100.0);
    });
    sink.record_picture_recovery_episode_requested(
        904,
        Some("receiverWaitingKeyframe".to_string()),
        110.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
    sink.record_picture_recovery_episode_packet_seen(150.0, Some(456_789), true, Some(322));
    sink.record_picture_recovery_episode_decoded(165.0, 456_789, 43);
    sink.record_pending_displayed_idr_rtp(456_789);
    sink.seed_decoder_reference_sync_for_pending_idr(456_789, 170.0);
    sink.record_displayed_idr_fact(180.0, 456_789, None);
    sink.complete_transport_recovery_after_stable_settle(210.0);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let observation = stats
        .latest_first_frame_latency_observation
        .as_ref()
        .expect("first frame latency observation");
    assert_eq!(observation.episode_id, Some(904));
    assert_eq!(observation.recovery_epoch, Some(1));
    assert_eq!(observation.control_ready_to_pli_sent_ms, Some(20.0));
    assert_eq!(observation.pli_sent_to_first_idr_packet_ms, Some(30.0));
    assert_eq!(observation.first_idr_packet_to_first_decode_ms, Some(15.0));
    assert_eq!(
        observation.first_decode_to_clean_anchor_committed_ms,
        Some(0.0)
    );
    assert_eq!(
        observation.clean_anchor_committed_to_display_stable_ms,
        Some(45.0)
    );
    assert_eq!(observation.terminal_phase.as_deref(), Some("DisplayStable"));
    assert_eq!(observation.incomplete_reason, None);

    let episode = stats
        .latest_keyframe_request_episode
        .as_ref()
        .expect("episode");
    assert!(episode.transport_detail.as_deref().is_some_and(|detail| {
        detail.contains("firstFrameLatencyTrace")
            && detail.contains("controlReadyToPliSentMs=20.0")
            && detail.contains("pliSentToFirstIdrPacketMs=30.0")
            && detail.contains("firstIdrPacketToFirstDecodeMs=15.0")
            && detail.contains("firstDecodeToCleanAnchorCommittedMs=0.0")
            && detail.contains("cleanAnchorCommittedToDisplayStableMs=45.0")
    }));
}

#[test]
fn first_frame_latency_observation_marks_no_idr_packet_when_only_pli_was_sent() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    RuntimeStatsSink::update_shared(&runtime_stats, |stats| {
        stats.control_ready_at_ms = Some(100.0);
    });
    sink.record_picture_recovery_episode_requested(
        905,
        Some("receiverWaitingKeyframe".to_string()),
        110.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 120.0, Some(260.0));

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let observation = stats
        .latest_first_frame_latency_observation
        .as_ref()
        .expect("first frame latency observation");
    assert_eq!(observation.episode_id, Some(905));
    assert_eq!(
        observation.terminal_phase.as_deref(),
        Some("WaitingResponse")
    );
    assert_eq!(
        observation.incomplete_reason.as_deref(),
        Some("noIdrPacket")
    );
    assert_eq!(observation.control_ready_to_pli_sent_ms, Some(20.0));
    assert_eq!(observation.pli_sent_to_first_idr_packet_ms, None);
    assert_eq!(observation.first_idr_packet_to_first_decode_ms, None);
    assert_eq!(observation.first_decode_to_clean_anchor_committed_ms, None);
    assert_eq!(
        observation.clean_anchor_committed_to_display_stable_ms,
        None
    );

    let episode = stats
        .latest_keyframe_request_episode
        .as_ref()
        .expect("episode");
    assert!(episode.transport_detail.as_deref().is_some_and(|detail| {
        detail.contains("firstFrameLatencyTrace")
            && detail.contains("controlReadyToPliSentMs=20.0")
            && detail.contains("pliSentToFirstIdrPacketMs=none")
    }));
}

#[test]
fn first_frame_latency_observation_marks_continuation_seen_while_awaiting_idr() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    RuntimeStatsSink::update_shared(&runtime_stats, |stats| {
        stats.control_ready_at_ms = Some(100.0);
    });
    sink.record_picture_recovery_episode_requested(
        906,
        Some("receiverWaitingKeyframe".to_string()),
        110.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 120.0, Some(320.0));
    sink.record_picture_recovery_episode_response_observed(
        155.0,
        Some(123_456),
        false,
        "continuationOnlyWhileAwaitingIdr",
        Some(333),
        Some(4),
        false,
        false,
    );

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let observation = stats
        .latest_first_frame_latency_observation
        .as_ref()
        .expect("first frame latency observation");
    assert_eq!(observation.episode_id, Some(906));
    assert_eq!(
        observation.terminal_phase.as_deref(),
        Some("ContinuationSeen")
    );
    assert_eq!(
        observation.incomplete_reason.as_deref(),
        Some("continuationOnlyAwaitingIdr")
    );
    assert_eq!(observation.control_ready_to_pli_sent_ms, Some(20.0));
    assert_eq!(observation.pli_sent_to_first_idr_packet_ms, None);
    assert_eq!(observation.first_idr_packet_to_first_decode_ms, None);

    let episode = stats
        .latest_keyframe_request_episode
        .as_ref()
        .expect("episode");
    assert_eq!(episode.first_video_packet_at_ms, Some(155.0));
    assert_eq!(episode.first_video_packet_is_keyframe, Some(false));
    assert_eq!(episode.first_keyframe_packet_at_ms, None);
    assert!(episode.transport_detail.as_deref().is_some_and(|detail| {
        detail.contains("firstFrameLatencyTrace")
            && detail.contains("controlReadyToPliSentMs=20.0")
            && detail.contains("pliSentToFirstIdrPacketMs=none")
    }));
}

#[test]
fn keyframe_request_episode_timeout_marks_missed_when_no_response_arrives() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.record_picture_recovery_episode_requested(
        88,
        Some("receiverWaitingKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("control", 120.0, Some(200.0));
    sink.record_picture_recovery_episode_timeout(199.0);

    {
        let stats = runtime_stats.lock().expect("runtime stats lock");
        let episode = stats
            .latest_keyframe_request_episode
            .as_ref()
            .expect("episode should exist");
        assert_eq!(episode.status, "sent");
        assert_eq!(episode.response_verdict.as_deref(), Some("pending"));
    }

    sink.record_picture_recovery_episode_timeout(200.0);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let episode = stats
        .latest_keyframe_request_episode
        .as_ref()
        .expect("episode should exist");
    assert_eq!(episode.status, "missed");
    assert_eq!(episode.response_verdict.as_deref(), Some("missed"));
    assert_eq!(
        stats.latest_observation_label.as_deref(),
        Some("keyframeRequestEpisodeMissed")
    );
}

#[test]
fn keyframe_request_episode_deferred_marks_unsent_terminal() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.record_picture_recovery_episode_requested(
        89,
        Some("ingressWaitKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_deferred(120.0, "familyInFlight:controlPending");

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let episode = stats
        .latest_keyframe_request_episode
        .as_ref()
        .expect("episode should exist");
    assert_eq!(episode.status, "deferred");
    assert_eq!(
        episode.response_verdict.as_deref(),
        Some("transportDeferred")
    );
    assert_eq!(
        stats.latest_observation_label.as_deref(),
        Some("keyframeRequestEpisodeDeferred")
    );
}

#[test]
fn keyframe_request_episode_unsent_expiry_marks_terminal() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.record_picture_recovery_episode_requested(
        90,
        Some("ingressWaitKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_unsent_expired(360.0);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let episode = stats
        .latest_keyframe_request_episode
        .as_ref()
        .expect("episode should exist");
    assert_eq!(episode.status, "expired-unsent");
    assert_eq!(episode.response_verdict.as_deref(), Some("unsentExpired"));
    assert_eq!(
        stats.latest_observation_label.as_deref(),
        Some("keyframeRequestEpisodeUnsentExpired")
    );
}

#[test]
fn keyframe_response_observed_keeps_lifecycle_in_sync_between_latest_and_recent() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());
    sink.record_picture_recovery_episode_requested(
        1,
        Some("receiverWaitingKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 110.0, Some(500.0));
    sink.record_picture_recovery_episode_response_observed(
        120.0,
        Some(999),
        false,
        "firstResponseNonKeyframe",
        Some(44),
        Some(3),
        false,
        false,
    );
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let latest = stats
        .latest_keyframe_request_episode
        .as_ref()
        .expect("latest episode");
    let recent = stats
        .recent_keyframe_request_episodes
        .iter()
        .find(|e| e.episode_id == 1)
        .expect("recent episode");
    assert_eq!(latest.lifecycle_phase, recent.lifecycle_phase);
    assert_eq!(latest.lifecycle_phase.as_deref(), Some("packetSeen"));
}

#[test]
fn h264_inspection_binds_episode_when_frame_rtp_matches_response() {
    use crate::XbxEngineH264InspectionObservation;

    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());
    sink.record_picture_recovery_episode_requested(
        42,
        Some("receiverWaitingKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 110.0, None);
    sink.record_picture_recovery_episode_packet_seen(120.0, Some(777), true, Some(88));
    sink.record_h264_inspection_observation(XbxEngineH264InspectionObservation {
        observation_id: 1,
        frame_rtp_timestamp: Some(777),
        nal_types: Vec::new(),
        nal_count: 0,
        vcl_nal_count: 0,
        has_inband_sps: false,
        has_inband_pps: false,
        committed_sps_present: false,
        committed_pps_present: false,
        slice_headers_valid: true,
        delta_continuation_ready: true,
        parameter_sets_changed: false,
        config_changed: false,
        is_idr: true,
        sample_width: None,
        sample_height: None,
        bootstrap_ready: true,
        bootstrap_reject_reason: None,
        admission_accepted: true,
        observed_at_ms: 125.0,
        ..Default::default()
    });
    let stats = runtime_stats.lock().expect("runtime stats lock");
    let h264 = stats
        .latest_h264_inspection_observation
        .as_ref()
        .expect("h264 observation");
    assert_eq!(h264.bound_episode_id, Some(42));
    assert!(h264.bound_as_recovery_response.unwrap_or(false));
    assert_eq!(
        stats.latest_observation_label.as_deref(),
        Some("h264InspectionObserved")
    );
    let summary = stats
        .latest_observation_summary
        .as_deref()
        .expect("h264 summary");
    assert!(summary.contains("rtpTimestamp=777"));
    assert!(summary.contains("isIdr=true"));
    assert!(summary.contains("boundEpisodeId=42"));
    assert!(summary.contains("boundAsRecoveryResponse=true"));
}

#[test]
fn h264_inspection_marks_post_recovery_degradation_after_stable_settle() {
    use crate::XbxEngineH264InspectionObservation;

    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_picture_recovery_episode_requested(
        43,
        Some("receiverWaitingKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 110.0, Some(300.0));
    sink.record_picture_recovery_episode_packet_seen(140.0, Some(888), true, Some(99));
    sink.record_picture_recovery_episode_decoded(160.0, 888, 44);
    sink.complete_transport_recovery_after_stable_settle(190.0);

    sink.record_h264_inspection_observation(XbxEngineH264InspectionObservation {
        observation_id: 2,
        frame_rtp_timestamp: Some(888),
        nal_types: Vec::new(),
        nal_count: 0,
        vcl_nal_count: 0,
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
        bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
        continuation_verdict: Some("receiverLocalContinuation".to_string()),
        admission_accepted: true,
        observed_at_ms: 200.0,
        ..Default::default()
    });

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let h264 = stats
        .latest_h264_inspection_observation
        .as_ref()
        .expect("h264 observation");
    assert_eq!(h264.bound_episode_id, Some(43));
    assert_eq!(h264.bound_recovery_epoch, Some(1));
    assert_eq!(h264.is_post_recovery_degradation, Some(true));
    assert_eq!(
        h264.reject_classification.as_deref(),
        Some("receiverLocalContinuation")
    );
}

#[test]
fn h264_continuation_binds_most_progressed_active_transport_await_episode() {
    use crate::XbxEngineH264InspectionObservation;

    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);

    sink.record_picture_recovery_episode_requested(
        42,
        Some("receiverWaitingKeyframe".to_string()),
        100.0,
        Some(400.0),
    );
    sink.record_picture_recovery_episode_sent("pli", 110.0, Some(400.0));
    sink.record_picture_recovery_episode_packet_seen(140.0, Some(0x1111_0001), true, Some(12));
    sink.update(|stats| {
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 84,
                request_reason: Some("displaySupplyCritical".to_string()),
                request_kind: Some("pli".to_string()),
                status: "sent".to_string(),
                status_detail: None,
                requested_at_ms: 200.0,
                sent_at_ms: Some(210.0),
                deadline_at_ms: Some(500.0),
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
                ..Default::default()
            });
    });

    sink.record_h264_inspection_observation(XbxEngineH264InspectionObservation {
        observation_id: 3,
        frame_rtp_timestamp: Some(0x1111_1001),
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
        bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
        continuation_verdict: Some("receiverLocalContinuation".to_string()),
        admission_accepted: true,
        observed_at_ms: 230.0,
        ..Default::default()
    });

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let h264 = stats
        .latest_h264_inspection_observation
        .as_ref()
        .expect("h264 observation");
    assert_eq!(h264.bound_episode_id, Some(42));
    assert_eq!(h264.bound_recovery_epoch, Some(1));
    assert!(h264.bound_as_recovery_response.unwrap_or(false));
    assert_eq!(
        h264.reject_classification.as_deref(),
        Some("receiverLocalContinuation")
    );
}

#[test]
fn missed_transport_await_episode_keeps_serviceable_continuation_family_binding() {
    use crate::XbxEngineH264InspectionObservation;

    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_picture_recovery_episode_requested(
        9101,
        Some("receiverWaitingKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 120.0, Some(160.0));
    sink.record_picture_recovery_episode_response_observed(
        140.0,
        Some(20_001),
        true,
        "firstAcceptedIdr",
        Some(11),
        None,
        false,
        false,
    );
    sink.record_picture_recovery_episode_timeout(180.0);
    sink.record_h264_inspection_observation(XbxEngineH264InspectionObservation {
        observation_id: 4,
        frame_rtp_timestamp: Some(20_333),
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
        bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
        continuation_verdict: Some("receiverLocalContinuation".to_string()),
        admission_accepted: true,
        observed_at_ms: 190.0,
        ..Default::default()
    });

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let episode = stats
        .latest_keyframe_request_episode
        .as_ref()
        .expect("episode should exist");
    assert_eq!(episode.status, "missed");
    let h264 = stats
        .latest_h264_inspection_observation
        .as_ref()
        .expect("h264 observation");
    assert_eq!(h264.bound_episode_id, Some(9101));
    assert!(h264.bound_as_recovery_response.unwrap_or(false));
    assert_eq!(h264.bound_response_rtp_timestamp, Some(20_001));
    assert_eq!(
        h264.reject_classification.as_deref(),
        Some("receiverLocalContinuation")
    );
}

#[test]
fn playback_recovered_keeps_unresolved_sent_transport_await_continuation_bound() {
    use crate::XbxEngineH264InspectionObservation;

    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_picture_recovery_episode_requested(
        9201,
        Some("receiverWaitingKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
    sink.update(|stats| {
        stats.recovery_playback_recovered_at_ms = Some(260.0);
        stats.recovery_playback_recovered_phase = Some("PlaybackRecovered".to_string());
    });
    sink.record_h264_inspection_observation(XbxEngineH264InspectionObservation {
        observation_id: 5,
        frame_rtp_timestamp: Some(30_333),
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
        bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
        continuation_verdict: Some("receiverLocalContinuation".to_string()),
        admission_accepted: true,
        observed_at_ms: 15_200.0,
        ..Default::default()
    });

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let episode = stats
        .latest_keyframe_request_episode
        .as_ref()
        .expect("episode should exist");
    assert_eq!(episode.status, "sent");
    let h264 = stats
        .latest_h264_inspection_observation
        .as_ref()
        .expect("h264 observation");
    assert_eq!(h264.bound_episode_id, Some(9201));
    assert_eq!(h264.bound_episode_status.as_deref(), Some("sent"));
    assert_eq!(h264.bound_recovery_epoch, Some(1));
    assert!(h264.bound_as_recovery_response.unwrap_or(false));
}

#[test]
fn stale_playback_recovered_does_not_bind_unresolved_transport_await_continuation() {
    use crate::XbxEngineH264InspectionObservation;

    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_picture_recovery_episode_requested(
        9203,
        Some("receiverWaitingKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
    sink.update(|stats| {
        stats.recovery_playback_recovered_at_ms = Some(80.0);
        stats.recovery_playback_recovered_phase = Some("PlaybackRecovered".to_string());
    });
    sink.record_h264_inspection_observation(XbxEngineH264InspectionObservation {
        observation_id: 8,
        frame_rtp_timestamp: Some(30_334),
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
        bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
        continuation_verdict: Some("receiverLocalContinuation".to_string()),
        admission_accepted: true,
        observed_at_ms: 15_200.0,
        ..Default::default()
    });

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let h264 = stats
        .latest_h264_inspection_observation
        .as_ref()
        .expect("h264 observation");
    assert_ne!(h264.bound_episode_id, Some(9203));
    assert_ne!(h264.bound_recovery_epoch, Some(1));
    assert_ne!(h264.bound_as_recovery_response, Some(true));
}

#[test]
fn playback_recovered_before_current_episode_does_not_bind_transport_await_continuation() {
    use crate::XbxEngineH264InspectionObservation;

    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_picture_recovery_episode_requested(
        9204,
        Some("receiverWaitingKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
    sink.update(|stats| {
        stats.transport_recovery_episode_opened_at_ms = Some(500.0);
        stats.recovery_playback_recovered_at_ms = Some(260.0);
        stats.recovery_playback_recovered_phase = Some("PlaybackRecovered".to_string());
    });
    sink.record_h264_inspection_observation(XbxEngineH264InspectionObservation {
        observation_id: 9,
        frame_rtp_timestamp: Some(30_335),
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
        bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
        continuation_verdict: Some("receiverLocalContinuation".to_string()),
        admission_accepted: true,
        observed_at_ms: 15_200.0,
        ..Default::default()
    });

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let h264 = stats
        .latest_h264_inspection_observation
        .as_ref()
        .expect("h264 observation");
    assert_ne!(h264.bound_episode_id, Some(9204));
    assert_ne!(h264.bound_recovery_epoch, Some(1));
    assert_ne!(h264.bound_as_recovery_response, Some(true));
}

#[test]
fn sent_transport_await_bridge_persists_after_first_serviceable_continuation_binding() {
    use crate::XbxEngineH264InspectionObservation;

    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_picture_recovery_episode_requested(
        9202,
        Some("receiverWaitingKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
    sink.update(|stats| {
        stats.latest_h264_inspection_observation = Some(XbxEngineH264InspectionObservation {
            observation_id: 6,
            frame_rtp_timestamp: Some(31_001),
            committed_sps_present: true,
            committed_pps_present: true,
            delta_continuation_ready: true,
            continuation_verdict: Some("receiverLocalContinuation".to_string()),
            bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
            admission_accepted: true,
            observed_at_ms: 500.0,
            bound_episode_id: Some(9202),
            bound_recovery_epoch: Some(1),
            bound_as_recovery_response: Some(true),
            ..Default::default()
        });
    });
    sink.record_h264_inspection_observation(XbxEngineH264InspectionObservation {
        observation_id: 7,
        frame_rtp_timestamp: Some(31_333),
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
        bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
        continuation_verdict: Some("receiverLocalContinuation".to_string()),
        admission_accepted: true,
        observed_at_ms: 20_500.0,
        ..Default::default()
    });

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let h264 = stats
        .latest_h264_inspection_observation
        .as_ref()
        .expect("h264 observation");
    assert_eq!(h264.bound_episode_id, Some(9202));
    assert_eq!(h264.bound_episode_status.as_deref(), Some("sent"));
    assert!(h264.bound_as_recovery_response.unwrap_or(false));
    assert_eq!(
        h264.reject_classification.as_deref(),
        Some("receiverLocalContinuation")
    );
}

#[test]
fn transport_recovery_family_continuation_follows_latest_decoded_transport_expired_owner() {
    use crate::XbxEngineH264InspectionObservation;

    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_picture_recovery_episode_requested(
        3446,
        Some("receiverWaitingKeyframe".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
    sink.record_picture_recovery_episode_packet_seen(150.0, Some(2_436_161_177), true, None);
    sink.record_picture_recovery_episode_decoded(170.0, 2_436_161_177, 1201);

    sink.record_picture_recovery_episode_requested(
        3593,
        Some("transportExpiredDeadline".to_string()),
        200.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 205.0, Some(260.0));
    sink.record_picture_recovery_episode_packet_seen(210.0, Some(2_441_661_257), true, None);
    sink.record_picture_recovery_episode_decoded(211.0, 2_441_661_257, 1342);

    sink.record_h264_inspection_observation(XbxEngineH264InspectionObservation {
        observation_id: 8,
        frame_rtp_timestamp: Some(2_441_664_407),
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
        bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
        continuation_verdict: Some("receiverLocalContinuation".to_string()),
        admission_accepted: true,
        observed_at_ms: 212.0,
        ..Default::default()
    });

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let h264 = stats
        .latest_h264_inspection_observation
        .as_ref()
        .expect("h264 observation");
    assert_eq!(h264.bound_episode_id, Some(3593));
    assert_eq!(h264.bound_episode_status.as_deref(), Some("succeeded"));
    assert_eq!(h264.bound_response_rtp_timestamp, Some(2_441_661_257));
    assert!(h264.bound_as_recovery_response.unwrap_or(false));
    assert!(
        stats.latest_picture_recovery_blocker_observation.is_none(),
        "admitted soft continuation must not record picture blocker"
    );
}

#[test]
fn transport_expired_deadline_decoded_ignores_stale_owner_rtp() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.begin_transport_recovery_episode(10.0);
    sink.record_picture_recovery_episode_requested(
        9301,
        Some("transportExpiredDeadline".to_string()),
        100.0,
        None,
    );
    sink.record_picture_recovery_episode_sent("pli", 120.0, Some(300.0));
    sink.record_picture_recovery_episode_packet_seen(150.0, Some(40_001), true, None);
    sink.record_picture_recovery_episode_decoded(170.0, 50_001, 1444);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let episode = stats
        .latest_keyframe_request_episode
        .as_ref()
        .expect("episode should exist");
    assert_eq!(episode.response_rtp_timestamp, Some(40_001));
    assert_eq!(episode.first_keyframe_decoded_at_ms, None);
    assert_eq!(
        stats.latest_observation_label.as_deref(),
        Some("keyframeRequestEpisodeDecodedIgnored")
    );
}

#[test]
fn rx_closed_keeps_close_intent_upstream_cause_after_other_observations() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.record_video_ingress_close_intent(10.0, "rebuildPeerConnection");
    sink.record_feedback_target_availability(
        11.0,
        "videoRtcpFeedback",
        "degraded",
        "twccReceiverMappingMissing",
    );
    sink.record_video_ingress_rx_closed(12.0, None);

    let stats = runtime_stats.lock().expect("runtime stats lock");
    let termination = stats
        .latest_video_ingress_termination_observation
        .as_ref()
        .expect("termination observation");
    assert_eq!(termination.cause, "upstreamSenderDropped");
    assert_eq!(
        termination.upstream_cause.as_deref(),
        Some("rebuildPeerConnection")
    );
}

#[test]
fn video_rtcp_send_failure_updates_feedback_target_availability() {
    let runtime_stats = Arc::new(Mutex::new(XbxEngineMediaRuntimeStats::default()));
    let sink = RuntimeStatsSink::new(runtime_stats.clone());

    sink.record_video_rtcp_send_failure(20.0, "xbxEngineRtcVideoRtcpFeedbackTargetUnavailable");

    let stats = runtime_stats.lock().expect("runtime stats lock");
    assert_eq!(
        stats.latest_feedback_target_availability_target.as_deref(),
        Some("videoRtcpFeedback")
    );
    assert_eq!(
        stats.latest_feedback_target_availability_state.as_deref(),
        Some("unavailable")
    );
    assert_eq!(
        stats.latest_feedback_target_availability_reason.as_deref(),
        Some("xbxEngineRtcVideoRtcpFeedbackTargetUnavailable")
    );
}
