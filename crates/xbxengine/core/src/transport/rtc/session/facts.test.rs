#[cfg(test)]
mod tests {
    use crate::transport::rtc::recovery::contract::{FrameValue, GapSeverity, RecoveryProgressLevel};
    use crate::transport::rtc::session::facts::compute_recovery_facts;
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
                reason: Some("transportAwaitRecoveryAnchor".to_string()),
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
            video_anchor_clean_epoch: Some(1),
            video_anchor_clean_source_event: Some("chain-clean-anchor-submitted".to_string()),
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
                state: "recovering".to_string(),
                reason: Some("transportAwaitRecoveryAnchor".to_string()),
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
        stats.latest_keyframe_request_episode = Some(crate::XbxEngineKeyframeRequestEpisodeObservation {
            episode_id: 9,
            request_reason: Some("transportAwaitRecoveryAnchor".to_string()),
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
        stats.video_anchor_clean_source_event = Some("chain-clean-anchor-submitted".to_string());
        stats.video_anchor_clean_observed_at_ms = Some(1350.0);
        let facts = compute_recovery_facts(&timeline, &stats);
        assert_eq!(
            facts.recovery_progress_level,
            Some(RecoveryProgressLevel::CleanAnchorCommitted)
        );
    }
}
