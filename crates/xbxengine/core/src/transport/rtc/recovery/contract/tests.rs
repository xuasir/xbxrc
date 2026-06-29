use super::*;
use crate::transport::rtc::session::facts::recovery_episode::recovery_progress_allows_decoder_reset;
use crate::transport::rtc::session::facts::{
    derive_gap_severity_from_timeline_observation, recovery_progress_level_from_episode,
    recovery_progress_level_from_str, recovery_progress_missing_anchor, GapSeverity,
    RecoveryProgressLevel,
};
use crate::{
    XbxEngineDecodeOutputPathObservation, XbxEngineH264InspectionObservation,
    XbxEngineMediaRuntimeStats, XbxEngineVideoReceiverObservation,
    XbxEngineVideoTimelineChainSnapshot, XbxEngineVideoTimelineGapSnapshot,
    XbxEngineVideoTimelineObservation,
};

#[test]
fn clean_anchor_masks_stale_insert_gate_waiting_transport_await_issue() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 3;
    stats.video_anchor_clean_epoch = Some(3);
    stats.video_anchor_clean_observed_at_ms = Some(1_000.0);
    stats.video_anchor_clean_source_event = Some("decoded-usable-idr".into());
    stats.recovery_fresh_anchor_recovered_at_ms = Some(1_000.0);
    stats.receive_keyframe_required = Some(false);
    stats.latest_video_timeline_observation = Some(XbxEngineVideoTimelineObservation {
        observation_id: 1,
        source_event: "insert-gate-need-keyframe".into(),
        gap: None,
        frame: None,
        chain: XbxEngineVideoTimelineChainSnapshot {
            state: "waiting-keyframe".into(),
            reason: Some("receiverWaitingKeyframe".into()),
            chain_break_evidence: None,
            observed_at_ms: 1_010.0,
        },
        observed_at_ms: 1_010.0,
    });

    assert!(!has_current_transport_await_issue_from_stats(&stats));
}

#[test]
fn timeline_gap_without_reference_evidence_maps_to_repairable_gap() {
    let obs = XbxEngineVideoTimelineObservation {
        observation_id: 99,
        source_event: "gap-observed".into(),
        gap: Some(XbxEngineVideoTimelineGapSnapshot {
            state: "observed".into(),
            sequence: Some(10),
            frame_rtp_timestamp: Some(42),
            frame_importance: Some("delta".into()),
            budget_importance: Some("disposable".into()),
            evidence_importance: Some("unknown".into()),
            gap_dependency_confidence: Some("anonymous".into()),
            observed_at_ms: 0.0,
        }),
        frame: None,
        chain: XbxEngineVideoTimelineChainSnapshot {
            state: "receiving".into(),
            reason: None,
            chain_break_evidence: None,
            observed_at_ms: 0.0,
        },
        observed_at_ms: 0.0,
    };
    assert_eq!(
        derive_gap_severity_from_timeline_observation(&obs),
        GapSeverity::RepairableGap
    );
}

#[test]
fn chain_broken_reason_with_anonymous_budget_only_gap_maps_to_reference_severity() {
    let obs = XbxEngineVideoTimelineObservation {
        observation_id: 1,
        source_event: "t".into(),
        gap: Some(XbxEngineVideoTimelineGapSnapshot {
            state: "observed".into(),
            sequence: Some(1),
            frame_rtp_timestamp: None,
            frame_importance: Some("unknown".into()),
            budget_importance: Some("supply".into()),
            evidence_importance: Some("unknown".into()),
            gap_dependency_confidence: Some("anonymous".into()),
            observed_at_ms: 0.0,
        }),
        frame: None,
        chain: XbxEngineVideoTimelineChainSnapshot {
            state: "waiting-keyframe".into(),
            reason: Some("referenceChainUnrecoverable".into()),
            chain_break_evidence: None,
            observed_at_ms: 0.0,
        },
        observed_at_ms: 0.0,
    };
    assert_eq!(
        derive_gap_severity_from_timeline_observation(&obs),
        GapSeverity::ReferenceGap
    );
}

#[test]
fn fresh_invalid_bootstrap_breaks_sustaining_recovery_suppression_after_clean_anchor() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 7;
    stats.video_anchor_clean_epoch = Some(7);
    stats.video_anchor_clean_observed_at_ms = Some(100.0);
    stats.video_anchor_clean_source_event = Some("displayed-idr".into());
    stats.recovery_displayed_idr_at_ms = Some(100.0);
    stats.recovery_fresh_anchor_recovered_at_ms = Some(100.0);
    stats.latest_video_timeline_observation = Some(XbxEngineVideoTimelineObservation {
        observation_id: 1,
        source_event: "frame-complete-candidate-decode-feedback-blocked".into(),
        gap: Some(XbxEngineVideoTimelineGapSnapshot {
            state: "expired".into(),
            sequence: Some(1),
            frame_rtp_timestamp: None,
            frame_importance: Some("anchor".into()),
            budget_importance: Some("disposable".into()),
            evidence_importance: Some("anchor".into()),
            gap_dependency_confidence: Some("anonymous".into()),
            observed_at_ms: 180.0,
        }),
        frame: None,
        chain: XbxEngineVideoTimelineChainSnapshot {
            state: "sustaining-recovery".into(),
            reason: Some("recoverySustaining".into()),
            chain_break_evidence: None,
            observed_at_ms: 180.0,
        },
        observed_at_ms: 180.0,
    });
    stats.latest_h264_inspection_observation = Some(XbxEngineH264InspectionObservation {
        observation_id: 2,
        frame_rtp_timestamp: Some(7001),
        nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".into()],
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
        bootstrap_reject_reason: Some("NonIdrVcl".into()),
        admission_accepted: true,
        observed_at_ms: 190.0,
        ..Default::default()
    });

    assert!(!has_current_transport_await_issue_from_stats(&stats));
}

#[test]
fn stale_invalid_bootstrap_does_not_break_sustaining_recovery_suppression() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 7;
    stats.video_anchor_clean_epoch = Some(7);
    stats.video_anchor_clean_observed_at_ms = Some(100.0);
    stats.video_anchor_clean_source_event = Some("displayed-idr".into());
    stats.recovery_displayed_idr_at_ms = Some(100.0);
    stats.recovery_fresh_anchor_recovered_at_ms = Some(100.0);
    stats.latest_video_timeline_observation = Some(XbxEngineVideoTimelineObservation {
        observation_id: 1,
        source_event: "frame-complete-candidate-decode-feedback-blocked".into(),
        gap: Some(XbxEngineVideoTimelineGapSnapshot {
            state: "expired".into(),
            sequence: Some(1),
            frame_rtp_timestamp: None,
            frame_importance: Some("anchor".into()),
            budget_importance: Some("disposable".into()),
            evidence_importance: Some("anchor".into()),
            gap_dependency_confidence: Some("anonymous".into()),
            observed_at_ms: 500.0,
        }),
        frame: None,
        chain: XbxEngineVideoTimelineChainSnapshot {
            state: "sustaining-recovery".into(),
            reason: Some("recoverySustaining".into()),
            chain_break_evidence: None,
            observed_at_ms: 500.0,
        },
        observed_at_ms: 500.0,
    });
    stats.latest_h264_inspection_observation = Some(XbxEngineH264InspectionObservation {
        observation_id: 2,
        frame_rtp_timestamp: Some(7001),
        nal_types: vec!["SliceLayerWithoutPartitioningNonIdr".into()],
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
        bootstrap_reject_reason: Some("NonIdrVcl".into()),
        admission_accepted: true,
        observed_at_ms: 190.0,
        ..Default::default()
    });

    assert!(!has_current_transport_await_issue_from_stats(&stats));
}

#[test]
fn recovery_progress_level_mapping_follows_rfc_order() {
    assert_eq!(
        recovery_progress_level_from_episode(
            "requested",
            Some("pending"),
            None,
            None,
            None,
            false,
            false
        ),
        Some(RecoveryProgressLevel::WaitingResponse)
    );
    assert_eq!(
        recovery_progress_level_from_episode(
            "response-observed",
            Some("on-time"),
            Some(false),
            None,
            None,
            false,
            false
        ),
        Some(RecoveryProgressLevel::ContinuationSeen)
    );
    assert_eq!(
        recovery_progress_level_from_episode(
            "packet-seen",
            Some("on-time"),
            Some(true),
            Some(10.0),
            None,
            false,
            false
        ),
        Some(RecoveryProgressLevel::AnchorSeen)
    );
    assert_eq!(
        recovery_progress_level_from_episode(
            "decoded",
            Some("on-time"),
            Some(true),
            Some(10.0),
            Some(20.0),
            false,
            false
        ),
        Some(RecoveryProgressLevel::Decoded)
    );
    assert_eq!(
        recovery_progress_level_from_episode(
            "decoded",
            Some("cleanAnchorCommitted"),
            Some(true),
            Some(10.0),
            Some(20.0),
            true,
            false
        ),
        Some(RecoveryProgressLevel::CleanAnchorCommitted)
    );
    assert_eq!(
        recovery_progress_level_from_episode(
            "decoded",
            Some("cleanAnchorCommitted"),
            Some(true),
            Some(10.0),
            Some(20.0),
            true,
            true
        ),
        Some(RecoveryProgressLevel::DisplayStable)
    );
}

#[test]
fn recovery_progress_gap_helpers_match_contract() {
    assert!(recovery_progress_missing_anchor(Some(
        RecoveryProgressLevel::WaitingResponse
    )));
    assert!(recovery_progress_missing_anchor(Some(
        RecoveryProgressLevel::ContinuationSeen
    )));
    assert!(!recovery_progress_missing_anchor(Some(
        RecoveryProgressLevel::AnchorSeen
    )));
    assert_eq!(
        recovery_progress_level_from_str("ContinuationSeen"),
        Some(RecoveryProgressLevel::ContinuationSeen)
    );
    assert_eq!(recovery_progress_level_from_str("unknown"), None);
    assert!(recovery_progress_allows_decoder_reset(Some(
        RecoveryProgressLevel::Decoded
    )));
    assert!(!recovery_progress_allows_decoder_reset(Some(
        RecoveryProgressLevel::ContinuationSeen
    )));
}

#[test]
fn recovery_display_facts_projects_from_stats() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 1;
    stats.video_anchor_clean_epoch = Some(1);
    stats.video_anchor_clean_observed_at_ms = Some(120.0);
    stats.video_anchor_clean_source_event = Some("decoded-usable-idr".to_string());
    stats.recovery_displayed_idr_at_ms = Some(120.0);
    stats.recovery_fresh_anchor_recovered_at_ms = Some(120.0);
    let display = RecoveryDisplayFacts::from_stats(&stats);
    assert_eq!(display.displayed_idr_at_ms, Some(120.0));
    assert!(display.has_established_displayed_idr());
    assert!(has_current_clean_anchor_from_stats(&stats));
}

#[test]
fn displayed_idr_alone_does_not_count_as_current_clean_anchor() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.recovery_displayed_idr_at_ms = Some(120.0);

    let display = RecoveryDisplayFacts::from_stats(&stats);
    assert!(display.has_established_displayed_idr());
    assert!(displayed_idr_serving_from_stats(&stats));
    assert_eq!(current_clean_anchor_observed_at_ms_from_stats(&stats), None);
    assert!(!has_current_clean_anchor_from_stats(&stats));
}

#[test]
fn stale_displayed_idr_before_current_episode_is_not_serving() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_episode_active = true;
    stats.transport_recovery_episode_opened_at_ms = Some(2_000.0);
    stats.recovery_displayed_idr_at_ms = Some(1_200.0);

    let display = RecoveryDisplayFacts::from_stats(&stats);
    assert_eq!(display.displayed_idr_at_ms, None);
    assert!(!display.has_established_displayed_idr());
    assert!(!displayed_idr_serving_from_stats(&stats));
}

#[test]
fn decoded_clean_anchor_counts_as_current_anchor_without_display_fact() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 7;
    stats.video_anchor_clean_epoch = Some(7);
    stats.video_anchor_clean_observed_at_ms = Some(1_200.0);
    stats.video_anchor_clean_source_event = Some("decoded-usable-idr".into());

    let display = RecoveryDisplayFacts::from_stats(&stats);
    assert_eq!(display.displayed_idr_at_ms, None);
    assert_eq!(
        current_clean_anchor_observed_at_ms_from_stats(&stats),
        Some(1_200.0)
    );
    assert!(has_current_clean_anchor_from_stats(&stats));
}

#[test]
fn fresh_anchor_counts_as_current_anchor_without_display_fact() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_episode_active = true;
    stats.transport_recovery_episode_opened_at_ms = Some(1_000.0);
    stats.recovery_fresh_anchor_recovered_at_ms = Some(1_200.0);

    let display = RecoveryDisplayFacts::from_stats(&stats);
    assert_eq!(display.displayed_idr_at_ms, None);
    assert_eq!(display.fresh_anchor_recovered_at_ms, Some(1_200.0));
    assert!(has_current_clean_anchor_from_stats(&stats));
}

#[test]
fn stale_fresh_anchor_before_current_episode_does_not_count_as_current_anchor() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_episode_active = true;
    stats.transport_recovery_episode_opened_at_ms = Some(2_000.0);
    stats.recovery_fresh_anchor_recovered_at_ms = Some(1_200.0);

    assert_eq!(current_clean_anchor_observed_at_ms_from_stats(&stats), None);
    assert!(!has_current_clean_anchor_from_stats(&stats));
}

#[test]
fn transport_await_hard_bootstrap_evidence_uses_non_idr_reject() {
    let now_ms = 2_000.0;
    let stats = XbxEngineMediaRuntimeStats {
        transport_recovery_epoch: 41,
        latest_video_timeline_observation: Some(XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-await-recovery-anchor".into(),
            gap: None,
            frame: None,
            chain: XbxEngineVideoTimelineChainSnapshot {
                state: "recovering".into(),
                reason: Some("receiverWaitingKeyframe".into()),
                chain_break_evidence: None,
                observed_at_ms: now_ms - 8.0,
            },
            observed_at_ms: now_ms - 8.0,
        }),
        latest_h264_inspection_observation: Some(XbxEngineH264InspectionObservation {
            observation_id: 2,
            frame_rtp_timestamp: Some(3_333),
            nal_types: vec![],
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
            bootstrap_reject_reason: Some("NonIdrVcl".into()),
            admission_accepted: true,
            observed_at_ms: now_ms - 6.0,
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(transport_await_has_hard_bootstrap_evidence_from_stats(
        &stats, now_ms
    ));
}

#[test]
fn decoded_clean_anchor_pipeline_absorbs_soft_non_idr_transport_await() {
    let now_ms = 2_000.0;
    let stats = XbxEngineMediaRuntimeStats {
        transport_recovery_epoch: 7,
        video_anchor_clean_epoch: Some(7),
        video_anchor_clean_observed_at_ms: Some(now_ms - 10.0),
        video_anchor_clean_source_event: Some("decoded-usable-idr".into()),
        latest_video_decode_ok_time_ms: Some(now_ms - 8.0),
        latest_video_host_present_time_ms: Some(now_ms - 6.0),
        latest_h264_inspection_observation: Some(XbxEngineH264InspectionObservation {
            observation_id: 2,
            frame_rtp_timestamp: Some(3_334),
            is_idr: false,
            bootstrap_ready: false,
            bootstrap_reject_reason: Some("NonIdrVcl".into()),
            admission_accepted: true,
            observed_at_ms: now_ms - 4.0,
            ..Default::default()
        }),
        ..Default::default()
    };

    assert!(!transport_await_has_hard_bootstrap_evidence_from_stats(
        &stats, now_ms
    ));
}

#[test]
fn current_display_stable_suppresses_stale_remote_terminal_reason() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_recovery_epoch: 3,
        video_anchor_clean_epoch: Some(3),
        video_anchor_clean_observed_at_ms: Some(120.0),
        video_anchor_clean_source_event: Some("decoded-usable-idr".to_string()),
        latest_receive_picture_recovery_terminal_reason: Some("remote-no-response".to_string()),
        receive_keyframe_required: Some(true),
        receive_keyframe_response_state: Some("no-packet".to_string()),
        receive_display_state: Some("display-stable".to_string()),
        reference_chain_state: Some("need-keyframe".to_string()),
        receive_keyframe_sent_count_unresolved: 7,
        ..Default::default()
    };

    assert!(!remote_picture_recovery_terminal_active_from_stats(&stats));
}

#[test]
fn stale_display_stable_without_current_clean_anchor_keeps_remote_terminal_active() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_recovery_epoch: 4,
        receive_display_state: Some("display-stable".to_string()),
        latest_receive_picture_recovery_terminal_reason: Some("remote-no-response".to_string()),
        receive_keyframe_required: Some(true),
        receive_keyframe_response_state: Some("no-packet".to_string()),
        reference_chain_state: Some("need-keyframe".to_string()),
        receive_keyframe_sent_count_unresolved: 7,
        receive_picture_recovery_terminal_total: 1,
        ..Default::default()
    };

    assert!(remote_picture_recovery_terminal_active_from_stats(&stats));
}

#[test]
fn displayed_idr_alone_keeps_remote_terminal_active_without_current_clean_anchor() {
    let stats = XbxEngineMediaRuntimeStats {
        recovery_displayed_idr_at_ms: Some(120.0),
        latest_receive_picture_recovery_terminal_reason: Some("remote-no-response".to_string()),
        receive_keyframe_required: Some(true),
        receive_keyframe_response_state: Some("no-packet".to_string()),
        receive_display_state: Some("recovering".to_string()),
        reference_chain_state: Some("need-keyframe".to_string()),
        receive_keyframe_sent_count_unresolved: 7,
        receive_picture_recovery_terminal_total: 1,
        ..Default::default()
    };

    assert!(remote_picture_recovery_terminal_active_from_stats(&stats));
}

#[test]
fn display_stable_without_clean_anchor_observed_at_does_not_complete_recovery() {
    let stats = XbxEngineMediaRuntimeStats {
        transport_recovery_epoch: 3,
        recovery_displayed_idr_at_ms: Some(120.0),
        video_anchor_clean_epoch: Some(3),
        receive_keyframe_required: Some(false),
        receive_keyframe_response_state: Some("usable-idr".to_string()),
        receive_display_state: Some("display-stable".to_string()),
        ..Default::default()
    };

    assert!(!receive_picture_recovery_complete_at(3, &stats, 130.0));
}

#[test]
fn displayed_idr_serving_true_when_pending_idr_and_host_has_presented() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.recovery_pending_displayed_idr_rtp = Some(77_001);
    stats.host_frame_present_epoch = 1;
    stats.latest_video_host_present_time_ms = Some(1_000.0);
    stats.display_age_ms = Some(16.0);
    assert!(displayed_idr_serving_from_stats(&stats));
    assert_eq!(
        resolve_host_display_idr_anchor_rtp(&stats, Some(77_002)),
        Some(77_001)
    );
}

#[test]
fn displayed_idr_serving_false_when_pending_idr_has_only_stale_present_epoch() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.recovery_pending_displayed_idr_rtp = Some(77_001);
    stats.host_frame_present_epoch = 1;
    stats.latest_video_host_present_time_ms = Some(1_000.0);
    stats.display_age_ms = Some(1_200.0);
    assert!(!displayed_idr_serving_from_stats(&stats));
}

#[test]
fn displayed_idr_serving_false_without_host_present_epoch() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.recovery_pending_displayed_idr_rtp = Some(77_001);
    stats.host_frame_present_epoch = 0;
    assert!(!displayed_idr_serving_from_stats(&stats));
}

#[test]
fn collapse_waiting_keyframe_when_displayed_idr_serving_without_gap() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.recovery_displayed_idr_at_ms = Some(100.0);
    assert!(!displayed_idr_projection_can_show_repairing(
        &stats, 200.0, false, 0
    ));
    assert!(displayed_idr_projection_can_show_repairing(
        &stats, 200.0, false, 10
    ));
}

#[test]
fn collapse_enabled_when_clean_anchor_and_decoder_waiting_keyframe() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 1;
    stats.video_anchor_clean_epoch = Some(1);
    stats.video_anchor_clean_observed_at_ms = Some(100.0);
    stats.video_anchor_clean_source_event = Some("decoded-usable-idr".to_string());
    stats.recovery_displayed_idr_at_ms = Some(100.0);
    stats.recovery_fresh_anchor_recovered_at_ms = Some(100.0);
    stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
    assert!(displayed_idr_projection_can_show_repairing(
        &stats, 200.0, true, 10
    ));
}

#[test]
fn relaxation_unblocked_under_supply_break_when_submit_stale() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.recovery_displayed_idr_at_ms = Some(100.0);
    stats.recovery_playback_recovered_at_ms = Some(100.0);
    stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
    stats.submit_age_ms = Some(1_500.0);
    stats.video_renderer_stalled = Some(true);
    assert!(!displayed_idr_presentation_continuation_blocked_from_stats(
        &stats, 200.0
    ));
    assert!(displayed_idr_presentation_continuation_serviceable_from_stats(&stats, 200.0));
}

#[test]
fn relaxation_not_blocked_by_soft_bootstrap_missing_idr_when_displayed_idr_serving() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 3;
    stats.video_anchor_clean_epoch = Some(3);
    stats.video_anchor_clean_observed_at_ms = Some(100.0);
    stats.video_anchor_clean_source_event = Some("decoded-usable-idr".to_string());
    stats.recovery_displayed_idr_at_ms = Some(100.0);
    stats.recovery_fresh_anchor_recovered_at_ms = Some(100.0);
    stats.host_frame_present_epoch = 1;
    stats.latest_video_host_present_time_ms = Some(180.0);
    stats.latest_h264_inspection_observation = Some(XbxEngineH264InspectionObservation {
        observed_at_ms: 180.0,
        admission_accepted: false,
        bootstrap_ready: false,
        bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
        ..Default::default()
    });
    assert!(!transport_await_has_hard_bootstrap_evidence_from_stats(
        &stats, 200.0
    ));
    assert!(displayed_idr_presentation_continuation_serviceable_from_stats(&stats, 200.0));
}

#[test]
fn stale_displayed_idr_does_not_suppress_hard_bootstrap_evidence_under_supply_break() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.recovery_displayed_idr_at_ms = Some(100.0);
    stats.host_frame_present_epoch = 1;
    stats.submit_age_ms = Some(1_500.0);
    stats.video_renderer_stalled = Some(true);
    stats.latest_h264_inspection_observation = Some(XbxEngineH264InspectionObservation {
        observed_at_ms: 180.0,
        admission_accepted: false,
        bootstrap_ready: false,
        bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
        ..Default::default()
    });
    assert!(!displayed_idr_presentation_continuation_serviceable_from_stats(&stats, 200.0));
    assert!(transport_await_has_hard_bootstrap_evidence_from_stats(
        &stats, 200.0
    ));
}

#[test]
fn waiting_keyframe_without_idr_progress_blocks_decoder_reset() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
    assert!(!decoder_reset_permitted_from_stats(
        &stats, None, 1_000.0, false
    ));
    stats.latest_h264_inspection_observation = Some(XbxEngineH264InspectionObservation {
        observed_at_ms: 990.0,
        is_idr: true,
        admission_accepted: true,
        ..Default::default()
    });
    assert!(decoder_reset_permitted_from_stats(
        &stats, None, 1_000.0, false
    ));
}

#[test]
fn clean_anchor_keeps_displayed_idr_relaxation_under_waiting_keyframe() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 1;
    stats.video_anchor_clean_epoch = Some(1);
    stats.video_anchor_clean_observed_at_ms = Some(100.0);
    stats.video_anchor_clean_source_event = Some("decoded-usable-idr".to_string());
    stats.recovery_displayed_idr_at_ms = Some(100.0);
    stats.recovery_fresh_anchor_recovered_at_ms = Some(100.0);
    stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
    assert!(!displayed_idr_presentation_continuation_blocked_from_stats(
        &stats, 5_000.0
    ));
    assert!(displayed_idr_presentation_continuation_serviceable_from_stats(&stats, 5_000.0));
}

#[test]
fn submit_starved_after_present_projects_supply_break_without_waiting_keyframe() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.video_decoder_recovery_state = Some("nominal".to_string());
    stats.host_frame_present_epoch = 100;
    stats.recovery_playback_recovered_at_ms = Some(1.0);
    stats.submit_age_ms = Some(5_000.0);
    assert!(media_supply_submit_starved_from_stats(&stats, 10_000.0));
    assert_eq!(
        derive_presentation_supply_phase_from_stats(&stats, 10_000.0),
        PresentationSupplyPhase::SupplyBreak
    );
    assert_eq!(
        derive_decoder_health_from_stats(&stats, 10_000.0),
        DerivedDecoderHealth::SupplyStalled
    );
}

#[test]
fn supply_break_when_waiting_keyframe_and_submit_stalled() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
    stats.submit_age_ms = Some(5_000.0);
    stats.recovery_playback_recovered_at_ms = Some(1.0);
    assert!(recovery_supply_break_active_from_stats(&stats, 10_000.0));
    let snap =
        RecoveryContractSnapshot::from_stats(&stats, 10_000.0, RecoveryExitThresholds::default());
    assert!(snap.supply_break_active);
    assert_eq!(
        snap.presentation_supply_phase,
        PresentationSupplyPhase::SupplyBreak
    );
    assert_eq!(snap.surface_phase, RecoverySurfacePhase::AwaitIdr);
    assert_eq!(
        derive_recovery_surface_phase_from_stats(&stats, 10_000.0),
        RecoverySurfacePhase::AwaitIdr
    );
    assert_eq!(
        derive_decoder_health_from_stats(&stats, 10_000.0),
        DerivedDecoderHealth::SupplyStalled
    );
    assert!(sparse_idr_rhythm_from_stats(&stats, 10_000.0).active);
}

/// trace `1779953007765-1` 末段：曾上屏 + submit 饿死 + H264 bootstrap/continuation 阻断。
fn trace_tail_starved_stats(decoder_state: &str) -> XbxEngineMediaRuntimeStats {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.host_frame_present_epoch = 683;
    stats.submit_age_ms = Some(18_000.0);
    stats.recovery_playback_recovered_at_ms = Some(1.0);
    stats.media_supply_host_first_present_at_ms = Some(53_000.0);
    stats.latest_video_decode_ok_time_ms = Some(128_000.0);
    stats.video_decoder_recovery_state = Some(decoder_state.to_string());
    stats.latest_h264_inspection_observation = Some(XbxEngineH264InspectionObservation {
        bootstrap_ready: false,
        bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
        reject_classification: Some("outOfRecoveryContextContinuation".to_string()),
        delta_continuation_ready: true,
        committed_sps_present: true,
        committed_pps_present: true,
        slice_headers_valid: true,
        admission_accepted: false,
        observed_at_ms: 128_500.0,
        ..Default::default()
    });
    stats.latest_video_receiver_observation = Some(XbxEngineVideoReceiverObservation {
        observation_id: 1,
        receiver_state: "repairing".to_string(),
        gap_sequence: None,
        gap_span: None,
        nack_in_flight: false,
        keyframe_request_pending: true,
        bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
        observed_at_ms: 128_500.0,
    });
    stats
}

fn assert_trace_tail_supply_break(stats: &XbxEngineMediaRuntimeStats) {
    const NOW_MS: f64 = 148_000.0;
    assert!(media_supply_submit_starved_from_stats(stats, NOW_MS));
    assert_eq!(
        derive_presentation_supply_phase_from_stats(stats, NOW_MS),
        PresentationSupplyPhase::SupplyBreak
    );
}

#[test]
fn trace_tail_submit_starved_recovering_supply_break_without_sparse_rhythm() {
    let stats = trace_tail_starved_stats("recovering");
    assert_trace_tail_supply_break(&stats);
    assert!(!sparse_idr_rhythm_from_stats(&stats, 148_000.0).active);
}

#[test]
fn trace_tail_submit_starved_waiting_keyframe_supply_break_and_sparse_rhythm() {
    let stats = trace_tail_starved_stats("waiting-keyframe");
    assert!(media_supply_submit_starved_from_stats(&stats, 148_000.0));
    assert_eq!(
        derive_presentation_supply_phase_from_stats(&stats, 148_000.0),
        PresentationSupplyPhase::SupplyBreak
    );
    assert_eq!(
        derive_recovery_surface_phase_from_stats(&stats, 148_000.0),
        RecoverySurfacePhase::AwaitIdr
    );
    assert!(sparse_idr_rhythm_from_stats(&stats, 148_000.0).active);
}

#[test]
fn sparse_idr_pressure_inactive_when_decoder_reference_synced() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
    stats.recovery_displayed_idr_at_ms = Some(900.0);
    stats.recovery_playback_recovered_at_ms = Some(1.0);
    stats.media_supply_host_first_present_at_ms = Some(1.0);
    stats.host_mailbox_enqueue_count_total = 1;
    stats.host_frame_present_epoch = 1;
    stats.latest_video_decode_ok_time_ms = Some(990.0);
    stats.submit_age_ms = Some(10.0);
    stats.recovery_decoder_reference_synced_at_ms = Some(999.0);
    assert!(!sparse_idr_pressure_active_from_stats(&stats, 1_000.0));
}

#[test]
fn sparse_idr_rhythm_pli_not_due_immediately_after_sent() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
    stats.host_frame_present_epoch = 1;
    stats.media_supply_host_first_present_at_ms = Some(-10_000.0);
    stats.latest_video_decode_ok_time_ms = Some(990.0);
    stats.submit_age_ms = Some(10.0);
    stats.receive_keyframe_last_sent_at_ms = Some(1_000.0);
    let rhythm = sparse_idr_rhythm_from_stats(&stats, 1_010.0);
    assert!(rhythm.active);
    assert!(!rhythm.pli_due);
}

#[test]
fn submit_starved_without_bootstrap_stays_out_of_reference_chain_control() {
    use super::reference_chain::{derive_reference_chain_state_from_stats, ReferenceChainState};

    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.host_frame_present_epoch = 1;
    stats.recovery_decoder_reference_synced_at_ms = Some(4_990.0);
    stats.submit_age_ms = Some(2_000.0);
    stats.latest_h264_inspection_observation = Some(crate::XbxEngineH264InspectionObservation {
        bootstrap_ready: false,
        ..Default::default()
    });
    let obs = derive_reference_chain_state_from_stats(&stats, 5_000.0, 100.0);
    assert_eq!(obs.state, ReferenceChainState::Continuous);
    assert_eq!(obs.cause, "reference-continuous");
}

#[test]
fn stale_prior_output_before_current_episode_keeps_reference_chain_priming_unknown() {
    use super::reference_chain::{derive_reference_chain_state_from_stats, ReferenceChainState};

    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_episode_active = true;
    stats.transport_recovery_episode_opened_at_ms = Some(2_000.0);
    stats.recovery_displayed_idr_at_ms = Some(1_000.0);
    stats.recovery_playback_recovered_at_ms = Some(1_000.0);
    stats.latest_video_receiver_observation = Some(XbxEngineVideoReceiverObservation {
        observation_id: 1,
        receiver_state: "waiting-keyframe".to_string(),
        gap_sequence: Some(10),
        gap_span: Some(4),
        nack_in_flight: true,
        keyframe_request_pending: true,
        bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
        observed_at_ms: 2_100.0,
    });
    stats.latest_video_timeline_observation = Some(XbxEngineVideoTimelineObservation {
        observation_id: 1,
        source_event: "gap".to_string(),
        gap: None,
        frame: None,
        chain: XbxEngineVideoTimelineChainSnapshot {
            state: "waiting-keyframe".to_string(),
            reason: Some("receiverWaitingKeyframe".to_string()),
            chain_break_evidence: None,
            observed_at_ms: 2_100.0,
        },
        observed_at_ms: 2_100.0,
    });

    let obs = derive_reference_chain_state_from_stats(&stats, 2_100.0, 100.0);
    assert_eq!(obs.state, ReferenceChainState::Unknown);
    assert_eq!(obs.cause, "bootstrap-missing-priming");
}

#[test]
fn need_keyframe_reference_state_blocks_non_idr_decodable_to_feed() {
    use super::insert::{decodable_to_feed, DecodableFeedContext};
    use super::reference_chain::ReferenceChainState;
    use crate::media::video::h264::inspection::H264AccessUnitInspection;

    let inspection = H264AccessUnitInspection {
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
    let ctx = DecodableFeedContext {
        decoder_reference_synced: true,
        first_frame_acquired: true,
        hard_gap_blocks_delta: false,
    };
    assert!(!decodable_to_feed(
        &inspection,
        &ctx,
        PacketRecoveryActionStage::Steady,
        ReferenceChainState::NeedKeyframe,
    ));
}

#[test]
fn sparse_idr_rhythm_nack_accel_requires_wait_keyframe_stage() {
    let rhythm = SparseIdrRhythm {
        active: true,
        pli_due: true,
        action_stage: PacketRecoveryActionStage::NackPending,
        pli_interval_ms: 24.0,
    };
    assert!(!rhythm.nack_escalation_immediate_eligible());
    let rhythm = SparseIdrRhythm {
        action_stage: PacketRecoveryActionStage::WaitKeyframe,
        ..rhythm
    };
    assert!(rhythm.nack_escalation_immediate_eligible());
}

#[test]
fn recovery_exit_timed_fallback_when_submit_stalled_without_anchor() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
    stats.submit_age_ms = Some(2_000.0);
    stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
    assert_eq!(
        recovery_exit_path_from_stats(&stats, 5_000.0, RecoveryExitThresholds::default()),
        RecoveryExitPath::TimedFallback
    );
}

#[test]
fn displayed_idr_relaxation_unblocked_under_timed_fallback() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.recovery_displayed_idr_at_ms = Some(100.0);
    stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
    stats.submit_age_ms = Some(2_000.0);
    stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
    assert!(!displayed_idr_presentation_continuation_blocked_from_stats(
        &stats, 5_000.0
    ));
}

#[test]
fn stale_submit_break_disabled_under_timed_fallback() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.recovery_displayed_idr_at_ms = Some(100.0);
    stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
    stats.submit_age_ms = Some(10_000.0);
    stats.video_decoder_stalled = Some(true);
    stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
    assert!(!displayed_idr_presentation_continuation_blocked_from_stats(
        &stats, 20_000.0
    ));
}

#[test]
fn recovery_exit_timed_fallback_over_stale_displayed_idr_fact() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.recovery_displayed_idr_at_ms = Some(100.0);
    stats.recovery_playback_recovered_at_ms = Some(100.0);
    stats.latest_video_decode_ok_time_ms = Some(4_900.0);
    stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
    stats.submit_age_ms = Some(2_000.0);
    stats.transport_state = xbxengine_protocol::XbxEngineTransportStateDto::Connected;
    assert_eq!(
        recovery_exit_path_from_stats(&stats, 5_000.0, RecoveryExitThresholds::default()),
        RecoveryExitPath::TimedFallback
    );
}

#[test]
fn recovery_exit_decode_output_when_decode_and_host_output_fresh() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.latest_video_decode_ok_time_ms = Some(4_900.0);
    stats.host_frame_present_epoch = 3;
    stats.latest_video_host_present_time_ms = Some(4_920.0);
    stats.recovery_playback_recovered_at_ms = Some(4_920.0);
    stats.submit_age_ms = Some(400.0);
    assert_eq!(
        recovery_exit_path_from_stats(&stats, 5_000.0, RecoveryExitThresholds::default()),
        RecoveryExitPath::DecodeOutput
    );
}

#[test]
fn recovery_exit_awaits_anchor_when_playback_recovered_is_stale() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.latest_video_decode_ok_time_ms = Some(4_900.0);
    stats.host_frame_present_epoch = 3;
    stats.latest_video_host_present_time_ms = Some(3_000.0);
    stats.recovery_playback_recovered_at_ms = Some(100.0);
    stats.submit_age_ms = Some(400.0);
    assert_eq!(
        recovery_exit_path_from_stats(&stats, 5_000.0, RecoveryExitThresholds::default()),
        RecoveryExitPath::AwaitingAnchor
    );
}

#[test]
fn parameter_sets_change_strict_active_within_window() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.recovery_displayed_idr_at_ms = Some(500.0);
    stats.host_frame_present_epoch = 1;
    stats.recovery_playback_recovered_at_ms = Some(600.0);
    stats.latest_video_decode_ok_time_ms = Some(1_250.0);
    stats.submit_age_ms = Some(80.0);
    stats.video_parameter_sets_changed_at_ms = Some(1_000.0);
    assert!(parameter_sets_change_strict_active_from_stats(
        &stats, 1_300.0, 50.0
    ));
    assert!(!parameter_sets_change_strict_active_from_stats(
        &stats, 1_500.0, 50.0
    ));
}

#[test]
fn parameter_sets_change_strict_inactive_during_priming() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.video_parameter_sets_changed_at_ms = Some(1_000.0);
    stats.host_frame_present_epoch = 1;
    assert!(!parameter_sets_change_strict_active_from_stats(
        &stats, 1_200.0, 50.0
    ));
    assert_eq!(
        derive_presentation_supply_phase_from_stats(&stats, 1_200.0),
        PresentationSupplyPhase::Priming
    );
}

#[test]
fn presentation_supply_stays_steady_when_reference_chain_is_continuous() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.host_frame_present_epoch = 2;
    stats.recovery_displayed_idr_at_ms = Some(100.0);
    stats.recovery_fresh_anchor_recovered_at_ms = Some(100.0);
    stats.recovery_playback_recovered_at_ms = Some(100.0);
    stats.recovery_decoder_reference_synced_at_ms = Some(980.0);
    stats.latest_video_decode_ok_time_ms = Some(980.0);
    stats.latest_video_decode_ok_rtp_timestamp = Some(7_001);
    stats.submit_age_ms = Some(40.0);
    stats.video_parameter_sets_changed_at_ms = Some(900.0);

    let reference = derive_reference_chain_state_from_stats(&stats, 1_000.0, 50.0);
    assert_eq!(reference.state, ReferenceChainState::Continuous);
    assert_eq!(
        derive_presentation_supply_phase_from_stats(&stats, 1_000.0),
        PresentationSupplyPhase::Steady
    );
    assert_eq!(
        derive_recovery_surface_phase_from_stats(&stats, 1_000.0),
        RecoverySurfacePhase::Steady
    );
    assert!(!sparse_idr_rhythm_from_stats(&stats, 1_000.0).active);
}

#[test]
fn sparse_idr_tracks_receive_keyframe_required_without_reference_fallback() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.host_frame_present_epoch = 2;
    stats.recovery_displayed_idr_at_ms = Some(100.0);
    stats.recovery_fresh_anchor_recovered_at_ms = Some(100.0);
    stats.recovery_playback_recovered_at_ms = Some(100.0);
    stats.receive_keyframe_required = Some(true);
    stats.reference_chain_state = Some("continuous".to_string());

    assert!(sparse_idr_rhythm_from_stats(&stats, 1_000.0).active);
}

#[test]
fn media_supply_priming_absorbs_repairing_until_segment_ages_healthy() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.host_frame_present_epoch = 68;
    stats.media_supply_host_first_present_at_ms = Some(1_900.0);
    stats.recovery_displayed_idr_at_ms = Some(1_000.0);
    stats.recovery_playback_recovered_at_ms = Some(1_500.0);
    stats.latest_video_decode_ok_time_ms = Some(1_950.0);
    stats.submit_age_ms = Some(800.0);
    stats.latest_video_receiver_observation = Some(crate::XbxEngineVideoReceiverObservation {
        observation_id: 1,
        receiver_state: "repairing".to_string(),
        gap_sequence: Some(10),
        gap_span: Some(2),
        nack_in_flight: true,
        keyframe_request_pending: false,
        bootstrap_reject_reason: None,
        observed_at_ms: 1_900.0,
    });
    stats.latest_video_timeline_observation = Some(XbxEngineVideoTimelineObservation {
        observation_id: 1,
        source_event: "gap".to_string(),
        gap: Some(XbxEngineVideoTimelineGapSnapshot {
            state: "pending".to_string(),
            sequence: Some(10),
            frame_rtp_timestamp: None,
            frame_importance: None,
            budget_importance: None,
            evidence_importance: None,
            gap_dependency_confidence: None,
            observed_at_ms: 1_900.0,
        }),
        frame: None,
        chain: XbxEngineVideoTimelineChainSnapshot {
            state: "repairing".to_string(),
            reason: None,
            chain_break_evidence: None,
            observed_at_ms: 1_900.0,
        },
        observed_at_ms: 1_900.0,
    });
    assert_eq!(
        derive_presentation_supply_phase_from_stats(&stats, 2_000.0),
        PresentationSupplyPhase::Priming
    );
    stats.submit_age_ms = Some(120.0);
    assert_eq!(
        derive_presentation_supply_phase_from_stats(&stats, 2_000.0),
        PresentationSupplyPhase::Priming
    );
    stats.latest_video_decode_ok_time_ms = Some(6_950.0);
    assert_eq!(
        derive_presentation_supply_phase_from_stats(&stats, 7_000.0),
        PresentationSupplyPhase::Repairing,
        "active gap stays in presentation repairing; keyframe policy remains receive-local"
    );
}

#[test]
fn waiting_keyframe_projects_surface_await_idr_during_acquisition_window() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.host_frame_present_epoch = 84;
    stats.media_supply_host_first_present_at_ms = Some(46_000.0);
    stats.recovery_displayed_idr_at_ms = Some(46_000.0);
    stats.latest_video_decode_ok_time_ms = Some(46_020.0);
    stats.submit_age_ms = Some(2_000.0);
    stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
    assert_eq!(
        derive_presentation_supply_phase_from_stats(&stats, 46_200.0),
        PresentationSupplyPhase::SupplyBreak
    );
    assert_eq!(
        derive_recovery_surface_phase_from_stats(&stats, 46_200.0),
        RecoverySurfacePhase::AwaitIdr
    );
}

#[test]
fn idr_recovery_active_from_stats_when_decoder_waiting_keyframe() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.host_frame_present_epoch = 1;
    stats.recovery_displayed_idr_at_ms = Some(1_000.0);
    stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
    assert!(idr_recovery_active_from_stats(&stats, 2_000.0));
}

#[test]
fn clean_anchor_masks_stale_decoder_waiting_for_recovery_contract() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 7;
    stats.video_anchor_clean_epoch = Some(7);
    stats.video_anchor_clean_observed_at_ms = Some(1_100.0);
    stats.video_anchor_clean_source_event = Some("decoded-usable-idr".to_string());
    stats.recovery_fresh_anchor_recovered_at_ms = Some(1_100.0);
    stats.recovery_decoder_reference_synced_at_ms = Some(1_100.0);
    stats.latest_video_decode_ok_time_ms = Some(1_100.0);
    stats.latest_video_decode_ok_rtp_timestamp = Some(88_001);
    stats.host_frame_present_epoch = 1;
    stats.submit_age_ms = Some(16.0);
    stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
    stats.latest_decode_output_path_observation = Some(XbxEngineDecodeOutputPathObservation {
        observation_id: 1,
        verdict: "backend-no-output".to_string(),
        detail: "backendNoOutputAfterWaitingKeyframeContinuation".to_string(),
        frame_rtp_timestamp: 88_001,
        is_keyframe: false,
        status: None,
        send_packet_status: None,
        receive_frame_status: None,
        backend_no_output_streak: Some(CONTINUATION_NO_OUTPUT_REQUEST_IDR_STREAK),
        input_frames_since_last_decoded: Some(8),
        bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
        observed_at_ms: 1_090.0,
    });

    assert!(!decoder_waiting_keyframe_control_active_from_stats(
        &stats, 1_120.0
    ));
    assert!(!decoder_no_output_request_idr_control_active_from_stats(
        &stats, 1_120.0
    ));
    assert_eq!(
        derive_packet_recovery_action_stage_from_stats(&stats, 1_120.0, 50.0),
        PacketRecoveryActionStage::Steady
    );
    let reference = derive_reference_chain_state_from_stats(&stats, 1_120.0, 50.0);
    assert_eq!(reference.state, ReferenceChainState::Continuous);
    assert_eq!(reference.cause, "reference-continuous");
    assert!(!idr_recovery_active_from_stats(&stats, 1_120.0));
    assert_eq!(
        derive_presentation_supply_phase_from_stats(&stats, 1_120.0),
        PresentationSupplyPhase::Steady
    );
}

#[test]
fn stale_fresh_anchor_before_current_episode_does_not_clear_decoder_debt() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_epoch = 8;
    stats.transport_recovery_episode_active = true;
    stats.transport_recovery_episode_opened_at_ms = Some(2_000.0);
    stats.recovery_fresh_anchor_recovered_at_ms = Some(1_100.0);
    stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
    stats.latest_decode_output_path_observation = Some(XbxEngineDecodeOutputPathObservation {
        observation_id: 2,
        verdict: "backend-no-output".to_string(),
        detail: "backendNoOutputAfterWaitingKeyframeContinuation".to_string(),
        frame_rtp_timestamp: 88_002,
        is_keyframe: false,
        status: None,
        send_packet_status: None,
        receive_frame_status: None,
        backend_no_output_streak: Some(CONTINUATION_NO_OUTPUT_REQUEST_IDR_STREAK),
        input_frames_since_last_decoded: Some(8),
        bootstrap_reject_reason: Some("NonIdrVcl".to_string()),
        observed_at_ms: 2_090.0,
    });

    assert!(decoder_waiting_keyframe_control_active_from_stats(
        &stats, 2_120.0
    ));
    assert!(decoder_no_output_request_idr_control_active_from_stats(
        &stats, 2_120.0
    ));
    assert_eq!(
        derive_packet_recovery_action_stage_from_stats(&stats, 2_120.0, 50.0),
        PacketRecoveryActionStage::RequestIdr
    );
}

#[test]
fn idr_recovery_active_tracks_receive_keyframe_required_only() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.host_frame_present_epoch = 1;
    stats.recovery_displayed_idr_at_ms = Some(1_000.0);
    stats.receive_keyframe_required = Some(true);
    stats.reference_chain_state = Some("continuous".to_string());
    assert!(idr_recovery_active_from_stats(&stats, 2_000.0));

    stats.receive_keyframe_required = Some(false);
    stats.reference_chain_state = Some("need-keyframe".to_string());
    assert!(!idr_recovery_active_from_stats(&stats, 2_000.0));
}

#[test]
fn receive_media_recovery_pressure_uses_ledger_not_derived_health() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.derived_decoder_health = Some("await-idr".to_string());
    stats.receive_keyframe_required = Some(false);
    stats.video_decoder_recovery_state = None;
    assert!(!receive_media_recovery_pressure_from_stats(&stats, 2_000.0));

    stats.receive_keyframe_required = Some(true);
    assert!(receive_media_recovery_pressure_from_stats(&stats, 2_000.0));
}

#[test]
fn receive_presentation_holds_steady_without_displayed_idr_projection() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.host_frame_present_epoch = 1;
    stats.latest_video_host_present_time_ms = Some(1_880.0);
    stats.receive_keyframe_required = Some(true);
    assert!(receive_presentation_holds_steady_session_phase_from_stats(
        &stats, 2_000.0
    ));
    assert!(!displayed_idr_serving_from_stats(&stats));
}

#[test]
fn stale_playback_recovered_does_not_hold_steady_session_phase() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.host_frame_present_epoch = 1;
    stats.latest_video_host_present_time_ms = Some(1_000.0);
    stats.recovery_playback_recovered_at_ms = Some(1_000.0);

    assert!(!receive_presentation_holds_steady_session_phase_from_stats(
        &stats, 2_000.0
    ));
}

#[test]
fn stale_playback_recovered_before_current_episode_does_not_hold_steady_session_phase() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.transport_recovery_episode_active = true;
    stats.transport_recovery_episode_opened_at_ms = Some(2_000.0);
    stats.host_frame_present_epoch = 1;
    stats.latest_video_host_present_time_ms = Some(2_050.0);
    stats.recovery_playback_recovered_at_ms = Some(1_000.0);

    assert!(!receive_presentation_holds_steady_session_phase_from_stats(
        &stats, 2_100.0
    ));
}

#[test]
fn sync_derived_fields_sets_active_recovery_session_phase() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.host_frame_present_epoch = 1;
    stats.recovery_displayed_idr_at_ms = Some(1_000.0);
    stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
    sync_derived_recovery_contract_fields(&mut stats, 2_000.0);
    assert_eq!(stats.session_phase.as_deref(), Some("active-recovery"));
    assert_eq!(stats.media_supply_phase.as_deref(), Some("priming"));
    assert_eq!(stats.recovery_surface_phase.as_deref(), Some("await-idr"));
}

#[test]
fn media_supply_acquisition_window_forces_priming_despite_healthy_segment_ages() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.host_frame_present_epoch = 84;
    stats.media_supply_host_first_present_at_ms = Some(46_000.0);
    stats.latest_video_decode_ok_time_ms = Some(46_020.0);
    stats.submit_age_ms = Some(29.0);
    stats.latest_video_receiver_observation = Some(crate::XbxEngineVideoReceiverObservation {
        observation_id: 1,
        receiver_state: "repairing".to_string(),
        gap_sequence: Some(1),
        gap_span: None,
        nack_in_flight: true,
        keyframe_request_pending: true,
        bootstrap_reject_reason: None,
        observed_at_ms: 46_100.0,
    });
    assert_eq!(
        derive_presentation_supply_phase_from_stats(&stats, 46_200.0),
        PresentationSupplyPhase::Priming
    );
}

#[test]
fn media_supply_stays_presentation_only_when_waiting_with_ps_strict() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.host_frame_present_epoch = 2;
    stats.recovery_playback_recovered_at_ms = Some(100.0);
    stats.recovery_displayed_idr_at_ms = Some(100.0);
    stats.latest_video_decode_ok_time_ms = Some(950.0);
    stats.submit_age_ms = Some(80.0);
    stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
    stats.video_parameter_sets_changed_at_ms = Some(900.0);
    assert_eq!(
        derive_presentation_supply_phase_from_stats(&stats, 1_000.0),
        PresentationSupplyPhase::Steady
    );
    assert_eq!(
        derive_recovery_surface_phase_from_stats(&stats, 1_000.0),
        RecoverySurfacePhase::AwaitIdr
    );
}

#[test]
fn repairing_missing_idr_pressure_forces_keyframe_only_mode() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.recovery_displayed_idr_at_ms = Some(100.0);
    stats.host_frame_present_epoch = 1;
    stats.recovery_playback_recovered_at_ms = Some(200.0);
    stats.latest_video_receiver_observation = Some(crate::XbxEngineVideoReceiverObservation {
        observation_id: 1,
        receiver_state: "repairing".to_string(),
        gap_sequence: Some(10),
        gap_span: Some(2),
        nack_in_flight: true,
        keyframe_request_pending: false,
        bootstrap_reject_reason: None,
        observed_at_ms: 900.0,
    });
    stats.latest_h264_inspection_observation = Some(XbxEngineH264InspectionObservation {
        observed_at_ms: 950.0,
        is_idr: false,
        bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
        ..Default::default()
    });
    stats.latest_video_timeline_observation = Some(XbxEngineVideoTimelineObservation {
        observation_id: 1,
        source_event: "gap".to_string(),
        gap: Some(XbxEngineVideoTimelineGapSnapshot {
            state: "pending".to_string(),
            sequence: Some(10),
            frame_rtp_timestamp: None,
            frame_importance: None,
            budget_importance: None,
            evidence_importance: None,
            gap_dependency_confidence: None,
            observed_at_ms: 900.0,
        }),
        frame: None,
        chain: XbxEngineVideoTimelineChainSnapshot {
            state: "receiving".to_string(),
            reason: None,
            chain_break_evidence: None,
            observed_at_ms: 900.0,
        },
        observed_at_ms: 900.0,
    });
    stats.video_decoder_recovery_state = Some("waiting-keyframe".to_string());
    assert_eq!(
        resolve_gap_vs_keyframe_mode(&stats, 1_000.0, 50.0),
        GapVsKeyframeMode::KeyframeOnly
    );
}

#[test]
fn relaxation_still_blocked_by_invalid_bootstrap_when_displayed_idr_serving() {
    let mut stats = XbxEngineMediaRuntimeStats::default();
    stats.recovery_displayed_idr_at_ms = Some(100.0);
    stats.latest_h264_inspection_observation = Some(XbxEngineH264InspectionObservation {
        observed_at_ms: 180.0,
        admission_accepted: false,
        bootstrap_ready: false,
        bootstrap_reject_reason: Some("bootstrapMissingSps".to_string()),
        ..Default::default()
    });
    assert!(transport_await_has_hard_bootstrap_evidence_from_stats(
        &stats, 200.0
    ));
    assert!(!displayed_idr_presentation_continuation_serviceable_from_stats(&stats, 200.0));
}
