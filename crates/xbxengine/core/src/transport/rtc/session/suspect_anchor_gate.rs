//! RFC 2026-05-13：`LocalSupplySuspect` → `TransportAwaitRecoveryKeyframe` 升级门（dwell + 锚点证据 + 输出停滞）。
//! 仅依赖 `XbxEngineMediaRuntimeStats` 与 `RecoveryScenarioProfile` 既有字段，不引入平行时钟。

use crate::api::backend::{XbxEngineAnchorCandidateState, XbxEngineMediaRuntimeStats};
use crate::transport::rtc::recovery::contract::has_current_clean_anchor_from_stats;
use crate::transport::rtc::recovery::coordinator::RecoveryOwnerSignal;
use crate::transport::rtc::recovery::escalation::VideoEscalationReason;
use crate::transport::rtc::recovery::policy::RecoveryScenarioProfile;

fn event_fresh(observed_at_ms: Option<f64>, now_ms: f64, fresh_ms: f64, floor_at_ms: f64) -> bool {
    observed_at_ms.is_some_and(|observed_at| {
        observed_at >= floor_at_ms && (now_ms - observed_at).max(0.0) <= fresh_ms
    })
}

fn continuation_only_anchor_missing_observation(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
    fresh_ms: f64,
    floor_at_ms: f64,
) -> bool {
    stats
        .latest_h264_inspection_observation
        .as_ref()
        .is_some_and(|inspection| {
            inspection.observed_at_ms >= floor_at_ms
                && (now_ms - inspection.observed_at_ms).max(0.0) <= fresh_ms
                && inspection.admission_accepted
                && inspection.committed_sps_present
                && inspection.committed_pps_present
                && inspection.delta_continuation_ready
                && !inspection.bootstrap_ready
                && matches!(
                    inspection.bootstrap_reject_reason.as_deref(),
                    Some("bootstrapMissingIdr" | "NonIdrVcl")
                )
                && inspection.continuation_verdict.as_deref() == Some("receiverLocalContinuation")
        })
}

/// 是否存在「fresh 的锚点阻塞证据」，满足后才允许从 Suspect 升级到 AwaitAnchor。
pub(crate) fn anchor_evidence_fresh_for_await_anchor(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
    profile: &RecoveryScenarioProfile,
    floor_at_ms: f64,
) -> bool {
    let fresh_ms = profile.playback_recovered_track_progress_fresh_ms;
    if continuation_only_anchor_missing_observation(stats, now_ms, fresh_ms, floor_at_ms) {
        return true;
    }
    if stats.video_decoder_recovery_state.as_deref() == Some("waiting-keyframe")
        && event_fresh(
            stats.video_decoder_recovery_state_changed_at_ms,
            now_ms,
            fresh_ms,
            floor_at_ms,
        )
    {
        return true;
    }
    if let Some(inspection) = stats.latest_h264_inspection_observation.as_ref() {
        if (now_ms - inspection.observed_at_ms).max(0.0) <= fresh_ms
            && inspection.observed_at_ms >= floor_at_ms
            && !inspection.bootstrap_ready
            && matches!(
                inspection.bootstrap_reject_reason.as_deref(),
                Some("bootstrapMissingIdr" | "NonIdrVcl")
            )
        {
            return true;
        }
    }
    if let Some(ledger) = stats.latest_anchor_candidate_ledger.as_ref() {
        if (now_ms - ledger.observed_at_ms).max(0.0) <= fresh_ms
            && ledger.observed_at_ms >= floor_at_ms
        {
            let reject = matches!(ledger.state, XbxEngineAnchorCandidateState::Rejected);
            if reject {
                return true;
            }
        }
    }
    false
}

/// decode / present 在最近 profile 窗口内是否「都没有新鲜前进」（相对 floor_at_ms）。
pub(crate) fn decode_and_present_stalled(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
    profile: &RecoveryScenarioProfile,
    floor_at_ms: f64,
) -> bool {
    let decode_fresh = event_fresh(
        stats.latest_video_decode_ok_time_ms,
        now_ms,
        profile.playback_recovered_decode_progress_fresh_ms,
        floor_at_ms,
    );
    let present_fresh = event_fresh(
        stats.latest_video_host_present_time_ms,
        now_ms,
        profile.playback_recovered_host_present_fresh_ms,
        floor_at_ms,
    );
    !decode_fresh && !present_fresh
}

/// 满足 dwell + 无 clean anchor + 停滞 + 锚点证据时，将 Suspect 升级为 `TransportAwaitRecoveryKeyframe`。
pub(crate) fn upgrade_local_supply_suspect_signal_if_ready(
    signal: RecoveryOwnerSignal,
    suspect_since_ms: Option<f64>,
    stats: &XbxEngineMediaRuntimeStats,
    profile: &RecoveryScenarioProfile,
) -> RecoveryOwnerSignal {
    if signal.reason != VideoEscalationReason::LocalSupplySuspect {
        return signal;
    }
    let Some(since_ms) = suspect_since_ms else {
        return signal;
    };
    let dwell_ms = profile.playback_recovered_track_progress_fresh_ms;
    if (signal.observed_at_ms - since_ms).max(0.0) < dwell_ms {
        return signal;
    }
    if has_current_clean_anchor_from_stats(stats) {
        return signal;
    }
    let floor_at_ms = stats
        .transport_recovery_episode_opened_at_ms
        .map_or(since_ms, |opened_at_ms| opened_at_ms.max(since_ms));
    if !decode_and_present_stalled(stats, signal.observed_at_ms, profile, floor_at_ms) {
        return signal;
    }
    if !anchor_evidence_fresh_for_await_anchor(stats, signal.observed_at_ms, profile, floor_at_ms) {
        return signal;
    }
    RecoveryOwnerSignal {
        reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
        reason_label: "receiverWaitingKeyframe".to_string(),
        observed_at_ms: signal.observed_at_ms,
        gap_severity: signal.gap_severity,
        repairability: signal.repairability,
    }
}

/// 供 `recovery_anchor_evidence` 写入：与 RFC §5 `anchor_evidence` 枚举对齐的粗标签。
pub(crate) fn recovery_anchor_evidence_trace_code(
    stats: &XbxEngineMediaRuntimeStats,
) -> Option<String> {
    if stats
        .latest_h264_inspection_observation
        .as_ref()
        .is_some_and(|inspection| {
            inspection.admission_accepted
                && inspection.committed_sps_present
                && inspection.committed_pps_present
                && inspection.delta_continuation_ready
                && !inspection.bootstrap_ready
                && matches!(
                    inspection.bootstrap_reject_reason.as_deref(),
                    Some("bootstrapMissingIdr" | "NonIdrVcl")
                )
                && inspection.continuation_verdict.as_deref() == Some("receiverLocalContinuation")
        })
    {
        return Some("receiverLocalContinuation".to_string());
    }
    if stats
        .video_decoder_recovery_state
        .as_deref()
        .is_some_and(|s| s == "waiting-keyframe")
    {
        return Some("decoderWaitingKeyframe".to_string());
    }
    if let Some(inspection) = stats.latest_h264_inspection_observation.as_ref() {
        if !inspection.bootstrap_ready {
            if let Some(reason) = inspection.bootstrap_reject_reason.as_ref() {
                return Some(reason.clone());
            }
        }
    }
    if let Some(ledger) = stats.latest_anchor_candidate_ledger.as_ref() {
        if matches!(ledger.state, XbxEngineAnchorCandidateState::Rejected) {
            return Some("anchorReject".to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::upgrade_local_supply_suspect_signal_if_ready;
    use crate::api::backend::XbxEngineH264InspectionObservation;
    use crate::transport::rtc::recovery::coordinator::RecoveryOwnerSignal;
    use crate::transport::rtc::recovery::escalation::VideoEscalationReason;
    use crate::transport::rtc::recovery::policy::ScenarioPolicyResolver;
    use crate::XbxEngineMediaRuntimeStats;
    use xbxengine_protocol::XbxEngineRemoteProfileKindDto;

    fn cloud_profile() -> crate::transport::rtc::recovery::policy::RecoveryScenarioProfile {
        ScenarioPolicyResolver::resolve_recovery_profile_by_kind(
            XbxEngineRemoteProfileKindDto::CloudGaming,
        )
    }

    fn suspect_signal(observed_at_ms: f64) -> RecoveryOwnerSignal {
        RecoveryOwnerSignal {
            reason: VideoEscalationReason::LocalSupplySuspect,
            reason_label: "localSupplySuspect".to_string(),
            observed_at_ms,
            gap_severity: None,
            repairability: None,
        }
    }

    fn continuation_only_inspection(observed_at_ms: f64) -> XbxEngineH264InspectionObservation {
        XbxEngineH264InspectionObservation {
            observation_id: 1,
            frame_rtp_timestamp: None,
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
            bootstrap_reject_reason: Some("bootstrapMissingIdr".to_string()),
            continuation_verdict: Some("receiverLocalContinuation".to_string()),
            admission_accepted: true,
            observed_at_ms,
            bound_episode_id: Some(7),
            bound_episode_status: None,
            bound_as_recovery_response: None,
            bound_response_rtp_timestamp: None,
            bound_recovery_epoch: None,
            episode_phase_at_observation: None,
            is_post_recovery_degradation: None,
            reject_classification: None,
        }
    }

    #[test]
    fn suspect_gate_ignores_anchor_evidence_older_than_suspect_start() {
        let profile = cloud_profile();
        let suspect_since_ms = 1_000.0;
        let observed_at_ms = suspect_since_ms + profile.playback_recovered_track_progress_fresh_ms;
        let stats = XbxEngineMediaRuntimeStats {
            transport_recovery_episode_opened_at_ms: Some(100.0),
            latest_h264_inspection_observation: Some(continuation_only_inspection(900.0)),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let upgraded = upgrade_local_supply_suspect_signal_if_ready(
            suspect_signal(observed_at_ms),
            Some(suspect_since_ms),
            &stats,
            &profile,
        );

        assert_eq!(upgraded.reason, VideoEscalationReason::LocalSupplySuspect);
    }

    #[test]
    fn suspect_gate_upgrades_on_continuation_only_idr_dependency() {
        let profile = cloud_profile();
        let suspect_since_ms = 1_000.0;
        let observed_at_ms = suspect_since_ms + profile.playback_recovered_track_progress_fresh_ms;
        let stats = XbxEngineMediaRuntimeStats {
            transport_recovery_episode_opened_at_ms: Some(900.0),
            latest_h264_inspection_observation: Some(continuation_only_inspection(
                observed_at_ms - 1.0,
            )),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let upgraded = upgrade_local_supply_suspect_signal_if_ready(
            suspect_signal(observed_at_ms),
            Some(suspect_since_ms),
            &stats,
            &profile,
        );

        assert_eq!(
            upgraded.reason,
            VideoEscalationReason::TransportAwaitRecoveryKeyframe
        );
        assert_eq!(upgraded.reason_label, "receiverWaitingKeyframe");
    }
}
