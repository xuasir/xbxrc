#[cfg(test)]
mod tests {
    use xbxengine_protocol::XbxEngineTransportStateDto;

    use crate::transport::rtc::session::facts::compute_recovery_facts;
    use crate::transport::rtc::session::facts::{FrameValue, GapSeverity, RecoveryProgressLevel};
    use crate::{
        XbxEngineMediaRuntimeStats, XbxEngineVideoTimelineChainSnapshot,
        XbxEngineVideoTimelineGapSnapshot, XbxEngineVideoTimelineObservation,
    };

    #[test]
    fn test_compute_recovery_facts_normal_gap() {
        let timeline = XbxEngineVideoTimelineObservation {
            observation_id: 1,
            source_event: "frame-gap-detected".to_string(),
            chain: XbxEngineVideoTimelineChainSnapshot {
                state: "broken".to_string(),
                reason: Some("referenceGap".to_string()),
                chain_break_evidence: None,
                observed_at_ms: 1000.0,
            },
            gap: Some(XbxEngineVideoTimelineGapSnapshot {
                state: "unresolved".to_string(),
                sequence: Some(100),
                frame_rtp_timestamp: Some(12345),
                frame_importance: Some("reference".to_string()),
                budget_importance: None,
                evidence_importance: Some("reference".to_string()),
                gap_dependency_confidence: Some("bound".to_string()),
                observed_at_ms: 1000.0,
            }),
            frame: None,
            observed_at_ms: 1000.0,
        };

        let stats = XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 1,
            ..Default::default()
        };

        let facts = compute_recovery_facts(&timeline, &stats);

        assert_eq!(facts.gap_severity, Some(GapSeverity::ReferenceGap));
        assert_eq!(facts.frame_value, Some(FrameValue::Reference));
        assert!(!facts.episode_stalled);
        assert!(!facts.has_current_clean_anchor);
    }

    #[test]
    fn test_compute_recovery_facts_episode_stalled() {
        let timeline = XbxEngineVideoTimelineObservation {
            observation_id: 2,
            source_event: "frame-await-recovery-anchor".to_string(),
            chain: XbxEngineVideoTimelineChainSnapshot {
                state: "broken".to_string(),
                reason: Some("receiverWaitingKeyframe".to_string()),
                chain_break_evidence: None,
                observed_at_ms: 5000.0,
            },
            gap: None,
            frame: None,
            observed_at_ms: 5000.0,
        };

        let stats = XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 1,
            video_anchor_clean_observed_at_ms: Some(1000.0), // 4秒前
            recovery_displayed_idr_at_ms: Some(1000.0),      // 4秒前
            recovery_fresh_anchor_recovered_at_ms: Some(1000.0), // 4秒前
            video_anchor_clean_epoch: Some(1),
            video_anchor_clean_source_event: Some("displayed-idr".to_string()),
            ..Default::default()
        };

        let facts = compute_recovery_facts(&timeline, &stats);

        assert!(facts.episode_stalled);
        assert_eq!(facts.gap_severity, Some(GapSeverity::RecoveryBlocked));
    }

    #[test]
    fn test_compute_recovery_facts_progress_tracks_continuation_then_clean_anchor() {
        let timeline = XbxEngineVideoTimelineObservation {
            observation_id: 3,
            source_event: "frame-await-recovery-anchor".to_string(),
            chain: XbxEngineVideoTimelineChainSnapshot {
                state: "waiting-keyframe".to_string(),
                reason: Some("receiverWaitingKeyframe".to_string()),
                chain_break_evidence: None,
                observed_at_ms: 1300.0,
            },
            gap: None,
            frame: None,
            observed_at_ms: 1300.0,
        };
        let mut stats = XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 9,
            ..Default::default()
        };
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 9,
                request_reason: Some("receiverWaitingKeyframe".to_string()),
                request_kind: Some("pli".to_string()),
                status: "response-observed".to_string(),
                status_detail: Some("firstResponseNonKeyframe".to_string()),
                requested_at_ms: 1000.0,
                sent_at_ms: Some(1010.0),
                deadline_at_ms: Some(1500.0),
                transport_detail: None,
                first_video_packet_at_ms: Some(1200.0),
                first_video_packet_rtp_timestamp: Some(111),
                first_video_packet_is_keyframe: Some(false),
                first_keyframe_packet_at_ms: None,
                first_keyframe_decoded_at_ms: None,
                response_rtp_timestamp: Some(111),
                response_frame_seq: None,
                response_verdict: Some("on-time".to_string()),
                lifecycle_phase: None,
                retired_at_ms: None,
            });

        let facts = compute_recovery_facts(&timeline, &stats);
        assert_eq!(
            facts.recovery_progress_level,
            Some(RecoveryProgressLevel::ContinuationSeen)
        );

        stats.video_anchor_clean_epoch = Some(9);
        stats.video_anchor_clean_source_event = Some("displayed-idr".to_string());
        stats.video_anchor_clean_observed_at_ms = Some(1350.0);
        stats.recovery_displayed_idr_at_ms = Some(1350.0);
        stats.recovery_fresh_anchor_recovered_at_ms = Some(1350.0);
        let facts = compute_recovery_facts(&timeline, &stats);
        assert_eq!(
            facts.recovery_progress_level,
            Some(RecoveryProgressLevel::CleanAnchorCommitted)
        );
    }

    #[test]
    fn retired_success_episode_does_not_mask_new_transport_await_progress() {
        let timeline = XbxEngineVideoTimelineObservation {
            observation_id: 4,
            source_event: "frame-await-recovery-anchor".to_string(),
            chain: XbxEngineVideoTimelineChainSnapshot {
                state: "waiting-keyframe".to_string(),
                reason: Some("receiverWaitingKeyframe".to_string()),
                chain_break_evidence: None,
                observed_at_ms: 2_000.0,
            },
            gap: None,
            frame: None,
            observed_at_ms: 2_000.0,
        };
        let mut stats = XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 10,
            ..Default::default()
        };
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 10,
                request_reason: Some("receiverWaitingKeyframe".to_string()),
                request_kind: Some("pli".to_string()),
                status: "decoded".to_string(),
                status_detail: None,
                requested_at_ms: 1_000.0,
                sent_at_ms: Some(1_010.0),
                deadline_at_ms: Some(1_500.0),
                transport_detail: None,
                first_video_packet_at_ms: Some(1_200.0),
                first_video_packet_rtp_timestamp: Some(111),
                first_video_packet_is_keyframe: Some(true),
                first_keyframe_packet_at_ms: Some(1_200.0),
                first_keyframe_decoded_at_ms: Some(1_220.0),
                response_rtp_timestamp: Some(111),
                response_frame_seq: Some(5),
                response_verdict: Some("cleanAnchorCommitted".to_string()),
                lifecycle_phase: Some("success".to_string()),
                retired_at_ms: Some(1_300.0),
            });

        let facts = compute_recovery_facts(&timeline, &stats);
        assert_eq!(facts.recovery_progress_level, None);
        assert_eq!(facts.recovery_episode_stage, None);
    }

    #[test]
    fn bootstrap_missing_idr_after_decoded_stays_in_decoded_during_hold_window() {
        let timeline = XbxEngineVideoTimelineObservation {
            observation_id: 5,
            source_event: "frame-complete-candidate".to_string(),
            chain: XbxEngineVideoTimelineChainSnapshot {
                state: "receiving".to_string(),
                reason: None,
                chain_break_evidence: None,
                observed_at_ms: 2_200.0,
            },
            gap: None,
            frame: None,
            observed_at_ms: 2_200.0,
        };
        let mut stats = XbxEngineMediaRuntimeStats {
            transport_recovery_epoch: 11,
            ..Default::default()
        };
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 11,
                request_reason: Some("receiverWaitingKeyframe".to_string()),
                request_kind: Some("pli".to_string()),
                status: "decoded".to_string(),
                status_detail: None,
                requested_at_ms: 2_000.0,
                sent_at_ms: Some(2_010.0),
                deadline_at_ms: Some(2_900.0),
                transport_detail: None,
                first_video_packet_at_ms: Some(2_120.0),
                first_video_packet_rtp_timestamp: Some(111),
                first_video_packet_is_keyframe: Some(true),
                first_keyframe_packet_at_ms: Some(2_120.0),
                first_keyframe_decoded_at_ms: Some(2_140.0),
                response_rtp_timestamp: Some(111),
                response_frame_seq: Some(5),
                response_verdict: Some("on-time".to_string()),
                lifecycle_phase: Some("decoded".to_string()),
                retired_at_ms: None,
            });
        stats.latest_h264_inspection_observation =
            Some(crate::XbxEngineH264InspectionObservation {
                observation_id: 21,
                frame_rtp_timestamp: Some(222),
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
                admission_accepted: true,
                continuation_verdict: Some("receiverLocalContinuation".to_string()),
                observed_at_ms: 2_180.0,
                bound_episode_id: Some(7),
                ..Default::default()
            });

        let facts = compute_recovery_facts(&timeline, &stats);
        assert_eq!(
            facts.recovery_progress_level,
            Some(RecoveryProgressLevel::Decoded)
        );
        assert_eq!(facts.recovery_episode_progress_at_ms, Some(2_140.0));
    }

    #[test]
    fn recovery_progress_does_not_downgrade_from_unbound_continuation_observation() {
        let timeline = crate::XbxEngineVideoTimelineObservation {
            observation_id: 11,
            source_event: "frame-observed".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "waiting-keyframe".to_string(),
                reason: Some("receiverWaitingKeyframe".to_string()),
                chain_break_evidence: None,
                observed_at_ms: 2_150.0,
            },
            observed_at_ms: 2_150.0,
        };
        let mut stats = crate::XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 3;
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 7,
                request_reason: Some("receiverWaitingKeyframe".to_string()),
                request_kind: Some("pli".to_string()),
                status: "decoded".to_string(),
                status_detail: None,
                requested_at_ms: 2_000.0,
                sent_at_ms: Some(2_010.0),
                deadline_at_ms: Some(2_900.0),
                transport_detail: None,
                first_video_packet_at_ms: Some(2_120.0),
                first_video_packet_rtp_timestamp: Some(111),
                first_video_packet_is_keyframe: Some(true),
                first_keyframe_packet_at_ms: Some(2_120.0),
                first_keyframe_decoded_at_ms: Some(2_140.0),
                response_rtp_timestamp: Some(111),
                response_frame_seq: Some(5),
                response_verdict: Some("on-time".to_string()),
                lifecycle_phase: Some("decoded".to_string()),
                retired_at_ms: None,
            });
        stats.latest_h264_inspection_observation =
            Some(crate::XbxEngineH264InspectionObservation {
                observation_id: 21,
                frame_rtp_timestamp: Some(222),
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
                admission_accepted: true,
                continuation_verdict: Some("receiverLocalContinuation".to_string()),
                observed_at_ms: 2_180.0,
                bound_episode_id: None,
                ..Default::default()
            });

        let facts = compute_recovery_facts(&timeline, &stats);
        assert_eq!(
            facts.recovery_progress_level,
            Some(RecoveryProgressLevel::Decoded)
        );
        assert_eq!(facts.recovery_episode_progress_at_ms, Some(2_140.0));
    }

    #[test]
    fn continuation_only_serviceable_output_advances_to_playback_recovered() {
        let timeline = crate::XbxEngineVideoTimelineObservation {
            observation_id: 12,
            source_event: "frame-observed".to_string(),
            gap: None,
            frame: None,
            chain: crate::XbxEngineVideoTimelineChainSnapshot {
                state: "waiting-keyframe".to_string(),
                reason: Some("receiverWaitingKeyframe".to_string()),
                chain_break_evidence: None,
                observed_at_ms: 2_420.0,
            },
            observed_at_ms: 2_420.0,
        };
        let mut stats = crate::XbxEngineMediaRuntimeStats::default();
        stats.transport_recovery_epoch = 4;
        stats.latest_keyframe_request_episode =
            Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
                episode_id: 8,
                request_reason: Some("receiverWaitingKeyframe".to_string()),
                request_kind: Some("pli".to_string()),
                status: "decoded".to_string(),
                status_detail: None,
                requested_at_ms: 2_000.0,
                sent_at_ms: Some(2_010.0),
                deadline_at_ms: Some(2_900.0),
                transport_detail: None,
                first_video_packet_at_ms: Some(2_120.0),
                first_video_packet_rtp_timestamp: Some(111),
                first_video_packet_is_keyframe: Some(true),
                first_keyframe_packet_at_ms: Some(2_120.0),
                first_keyframe_decoded_at_ms: Some(2_140.0),
                response_rtp_timestamp: Some(111),
                response_frame_seq: Some(5),
                response_verdict: Some("on-time".to_string()),
                lifecycle_phase: Some("decoded".to_string()),
                retired_at_ms: None,
            });
        stats.latest_h264_inspection_observation =
            Some(crate::XbxEngineH264InspectionObservation {
                observation_id: 22,
                frame_rtp_timestamp: Some(333),
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
                admission_accepted: true,
                continuation_verdict: Some("receiverLocalContinuation".to_string()),
                observed_at_ms: 2_360.0,
                bound_episode_id: Some(8),
                ..Default::default()
            });
        stats.latest_video_decode_ok_time_ms = Some(2_380.0);
        stats.latest_video_host_present_time_ms = Some(2_400.0);
        stats.latest_video_track_status = Some(crate::XbxEngineVideoTrackStatus {
            state: "remoteTrackAttached".to_string(),
            video_width: Some(1920),
            video_height: Some(1080),
            mime_type: Some("video/h264".to_string()),
            transport_state: XbxEngineTransportStateDto::Connected,
            video_bytes_total: 48_000,
            video_packet_count_total: 512,
            audio_bytes_total: 0,
            observed_at_ms: 2_395.0,
        });

        let facts = compute_recovery_facts(&timeline, &stats);
        assert_eq!(
            facts.recovery_progress_level,
            Some(RecoveryProgressLevel::PlaybackRecovered)
        );
        assert_eq!(facts.recovery_episode_progress_at_ms, Some(2_400.0));
    }
}
