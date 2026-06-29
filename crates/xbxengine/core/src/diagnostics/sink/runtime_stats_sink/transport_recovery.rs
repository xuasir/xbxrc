// 由 `runtime_stats_sink` 模块拆分；采集面只写事实，不驱动控制决策。

use crate::transport::rtc::recovery::runtime_state::project_recovery_escalation_context;
use crate::{XbxEnginePictureRecoveryTransitionObservation, XbxEngineVideoEscalationObservation};

use super::support::*;
use super::RuntimeStatsSink;

const PLAYBACK_RECOVERED_MIN_PRESENT_FPS: f64 = 12.0;
const PLAYBACK_RECOVERED_MAX_PRESENT_FPS: f64 = 90.0;
const PLAYBACK_RECOVERED_MAX_PRESENT_AGE_MS: f64 = 300.0;

fn sanitize_playback_recovered_present_fps(
    stats: &crate::XbxEngineMediaRuntimeStats,
    present_fps: f64,
) -> Option<f64> {
    if present_fps >= PLAYBACK_RECOVERED_MIN_PRESENT_FPS
        && present_fps <= PLAYBACK_RECOVERED_MAX_PRESENT_FPS
    {
        return Some(present_fps);
    }
    let decode_fps = stats.video_decode_fps;
    if decode_fps >= PLAYBACK_RECOVERED_MIN_PRESENT_FPS
        && decode_fps <= PLAYBACK_RECOVERED_MAX_PRESENT_FPS
    {
        return Some(decode_fps);
    }
    if present_fps > 0.0 && present_fps < PLAYBACK_RECOVERED_MIN_PRESENT_FPS {
        return None;
    }
    None
}

fn host_present_qualifies_for_playback_recovered(
    stats: &crate::XbxEngineMediaRuntimeStats,
    observed_at_ms: f64,
) -> bool {
    if stats
        .transport_recovery_episode_opened_at_ms
        .is_some_and(|opened_at_ms| observed_at_ms < opened_at_ms)
    {
        return false;
    }
    if stats
        .display_age_ms
        .is_some_and(|age_ms| age_ms > PLAYBACK_RECOVERED_MAX_PRESENT_AGE_MS)
    {
        return false;
    }
    if matches!(stats.video_owner_state.as_deref(), Some("supply-starved")) {
        return false;
    }
    if matches!(
        stats.video_owner_reason.as_deref(),
        Some("displaySupplyCritical" | "hostPresentStalled")
    ) {
        return false;
    }
    true
}

impl RuntimeStatsSink {
    pub(crate) fn begin_transport_recovery_episode(&self, observed_at_ms: f64) -> u64 {
        let mut next_epoch = 0u64;
        self.update(|stats| {
            next_epoch = Self::apply_begin_transport_recovery_episode(stats, observed_at_ms);
        });
        next_epoch
    }

    pub(crate) fn advance_transport_recovery_episode(&self, observed_at_ms: f64) -> u64 {
        let mut next_epoch = 0u64;
        self.update(|stats| {
            next_epoch = Self::apply_advance_transport_recovery_episode(stats, observed_at_ms);
        });
        next_epoch
    }

    pub(crate) fn complete_transport_recovery_for_lifecycle_recovering(&self, observed_at_ms: f64) {
        self.update(|stats| {
            Self::apply_complete_transport_recovery_episode(
                stats,
                observed_at_ms,
                "lifecycleRecovering",
            );
            let observation = XbxEnginePictureRecoveryTransitionObservation {
                observation_id: Self::next_picture_recovery_transition_observation_id(stats),
                episode_id: stats
                    .latest_keyframe_request_episode
                    .as_ref()
                    .map(|episode| episode.episode_id),
                recovery_epoch: Some(stats.transport_recovery_epoch),
                phase: "EpisodeClosed".to_string(),
                from_phase: Some("Decoded".to_string()),
                to_phase: "EpisodeClosed".to_string(),
                cause: Some("lifecycleRecovering".to_string()),
                detail: None,
                rtp_timestamp: None,
                frame_seq: None,
                owner_state: stats.video_owner_state.clone(),
                transport_state: Some(format!("{:?}", stats.transport_state)),
                observed_at_ms,
            };
            stats.latest_picture_recovery_transition_observation = Some(observation);
            Self::refresh_first_frame_latency_observation(stats, observed_at_ms);
        });
    }

    pub(crate) fn record_pending_displayed_idr_rtp(&self, rtp_timestamp: u32) {
        self.update(|stats| {
            stats.recovery_pending_displayed_idr_rtp = Some(rtp_timestamp);
        });
    }

    /// 测试/合成路径：pending displayed-idr 须与解码器参考链同步后才可 commit fresh anchor。
    #[cfg(test)]
    pub(crate) fn seed_decoder_reference_sync_for_pending_idr(
        &self,
        rtp_timestamp: u32,
        observed_at_ms: f64,
    ) {
        self.update(|stats| {
            stats.recovery_decoder_reference_synced_at_ms = Some(observed_at_ms);
            stats.latest_video_decode_ok_time_ms = Some(observed_at_ms);
            stats.latest_video_decode_ok_rtp_timestamp = Some(rtp_timestamp);
        });
    }

    pub(crate) fn record_displayed_idr_fact(
        &self,
        observed_at_ms: f64,
        rtp_timestamp: u32,
        frame_seq: Option<u64>,
    ) {
        self.update(|stats| {
            if !host_display_rtp_qualifies_for_fresh_anchor(stats, rtp_timestamp, observed_at_ms) {
                return;
            }
            if stats.receive_display_state.as_deref() == Some("display-stable")
                && stats.recovery_displayed_idr_rtp == Some(rtp_timestamp)
            {
                return;
            }
            let already_has_clean_anchor =
                stats.video_anchor_clean_epoch == Some(stats.transport_recovery_epoch);
            stats.recovery_displayed_idr_rtp = Some(rtp_timestamp);
            stats.recovery_displayed_idr_at_ms = Some(observed_at_ms);
            stats.receive_display_state = Some("display-stable".to_string());
            stats.recovery_pending_displayed_idr_rtp = None;
            if !already_has_clean_anchor {
                Self::apply_transport_clean_anchor(stats, observed_at_ms, "displayed-idr");
            }
            let observation = XbxEnginePictureRecoveryTransitionObservation {
                observation_id: Self::next_picture_recovery_transition_observation_id(stats),
                episode_id: None,
                recovery_epoch: Some(stats.transport_recovery_epoch),
                phase: if already_has_clean_anchor {
                    "DisplayStable".to_string()
                } else {
                    "FreshAnchorRecovered".to_string()
                },
                from_phase: Some(
                    if already_has_clean_anchor {
                        "CleanAnchorCommitted"
                    } else {
                        "Decoded"
                    }
                    .to_string(),
                ),
                to_phase: if already_has_clean_anchor {
                    "DisplayStable".to_string()
                } else {
                    "FreshAnchorRecovered".to_string()
                },
                cause: Some("displayed-idr".to_string()),
                detail: Some(
                    if already_has_clean_anchor {
                        "displayGate"
                    } else {
                        "hostVisible"
                    }
                    .to_string(),
                ),
                rtp_timestamp: Some(rtp_timestamp),
                frame_seq,
                owner_state: stats.video_owner_state.clone(),
                transport_state: Some(format!("{:?}", stats.transport_state)),
                observed_at_ms,
            };
            stats.latest_picture_recovery_transition_observation = Some(observation);
            Self::refresh_first_frame_latency_observation(stats, observed_at_ms);
            emit_picture_recovery_closure_probe(
                &*stats,
                "clean-anchor",
                observed_at_ms,
                stats.latest_keyframe_request_episode.as_ref(),
                stats.latest_anchor_candidate_ledger.as_ref(),
            );
        });
    }

    pub(crate) fn record_playback_recovered_fact(&self, observed_at_ms: f64, present_fps: f64) {
        self.update(|stats| {
            if stats.recovery_playback_recovered_at_ms.is_some() {
                return;
            }
            if !host_present_qualifies_for_playback_recovered(stats, observed_at_ms) {
                return;
            }
            let effective_fps = sanitize_playback_recovered_present_fps(stats, present_fps);
            let Some(effective_fps) = effective_fps else {
                return;
            };
            stats.recovery_playback_recovered_at_ms = Some(observed_at_ms);
            stats.recovery_playback_recovered_phase = Some("hostPresent".to_string());
            let observation = XbxEnginePictureRecoveryTransitionObservation {
                observation_id: Self::next_picture_recovery_transition_observation_id(stats),
                episode_id: None,
                recovery_epoch: Some(stats.transport_recovery_epoch),
                phase: "PlaybackRecovered".to_string(),
                from_phase: Some("Decoded".to_string()),
                to_phase: "PlaybackRecovered".to_string(),
                cause: Some("hostPresent".to_string()),
                detail: Some(format!("presentFps={effective_fps:.1}")),
                rtp_timestamp: stats.recovery_displayed_idr_rtp,
                frame_seq: stats.last_displayed_frame_seq,
                owner_state: stats.video_owner_state.clone(),
                transport_state: Some(format!("{:?}", stats.transport_state)),
                observed_at_ms,
            };
            stats.latest_picture_recovery_transition_observation = Some(observation);
        });
    }

    pub(crate) fn complete_transport_recovery_after_stable_settle(&self, observed_at_ms: f64) {
        self.update(|stats| {
            Self::apply_complete_transport_recovery_episode(
                stats,
                observed_at_ms,
                "stableServingSettled",
            );
            let display_stable = XbxEnginePictureRecoveryTransitionObservation {
                observation_id: Self::next_picture_recovery_transition_observation_id(stats),
                episode_id: stats
                    .latest_keyframe_request_episode
                    .as_ref()
                    .map(|episode| episode.episode_id),
                recovery_epoch: Some(stats.transport_recovery_epoch),
                phase: "DisplayStable".to_string(),
                from_phase: Some("FreshAnchorRecovered".to_string()),
                to_phase: "DisplayStable".to_string(),
                cause: Some("stableServingSettled".to_string()),
                detail: Some("displayGate".to_string()),
                rtp_timestamp: None,
                frame_seq: None,
                owner_state: stats.video_owner_state.clone(),
                transport_state: Some(format!("{:?}", stats.transport_state)),
                observed_at_ms,
            };
            stats.latest_picture_recovery_transition_observation = Some(display_stable);
            Self::refresh_first_frame_latency_observation(stats, observed_at_ms);
        });
    }

    pub(crate) fn record_transport_command_semantic(
        &self,
        command_name: &str,
        status_name: &str,
        status_detail: Option<&str>,
        semantic_detail: Option<&str>,
        _observed_at_ms: f64,
    ) {
        self.update(|stats| {
            let mut summary = format!("command={command_name} status={status_name}");
            if let Some(detail) = status_detail {
                summary.push_str(" detail=");
                summary.push_str(detail);
            }
            if let Some(semantic) = semantic_detail {
                summary.push_str(" semantic=");
                summary.push_str(semantic);
            }
            if stats.latest_observation_label.is_none() {
                stats.latest_observation_label = Some("rtcTransportCommandSemantic".to_string());
            }
            stats.latest_observation_summary = Some(summary);
        });
    }

    pub(crate) fn record_recovery_escalation_success(
        &self,
        observation_id: u64,
        reason: String,
        action: impl Into<String>,
        observed_at_ms: f64,
        advances_recovery_epoch: bool,
    ) {
        if advances_recovery_epoch {
            self.advance_transport_recovery_episode(observed_at_ms);
        }
        let action = action.into();
        self.update(|stats| {
            let context = project_recovery_escalation_context(stats, &reason, &action);
            stats.latest_video_escalation_observation = Some(XbxEngineVideoEscalationObservation {
                observation_id,
                reason,
                action,
                recovery_stage: context.stage,
                recovery_chain_value: context.chain_value,
                recovery_failure_cost: context.failure_cost,
                recovery_window_source: context.window_source,
                observed_at_ms,
            });
            stats.transport_recovery_epoch_at_last_escalation = stats.transport_recovery_epoch;
        });
    }
}
