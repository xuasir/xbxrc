use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationReason};
use crate::transport::rtc::recovery::hard_stall::HARD_STALL_DECODER_RESET_MS;
use crate::transport::rtc::recovery::runtime_state::unix_now_ms;
use crate::transport::rtc::recovery::startup::{resolve_session_phase, SessionPhase};
use crate::{XbxEngineMediaRuntimeStats, XbxEngineVideoNackObservation};

const RECENT_NACK_OUTCOME_WINDOW_MS: f64 = 180.0;
const RECENT_NACK_OUTCOME_WINDOW_MS_CLOUD: f64 = 520.0;
const CLOUD_STARTUP_NACK_BUDGET_WINDOW_MS: f64 = 1_200.0;
const CLOUD_STARTUP_NACK_BUDGET_THRESHOLD: u8 = 3;

pub(crate) enum RecentNackOutcomeResolution {
    Suppress(RecoveryAction),
    Escalate(VideoEscalationReason),
}

#[derive(Default)]
pub(crate) struct CloudStartupExpiredDeadlineBudget {
    first_seen_at_ms: Option<f64>,
    last_observation_id: Option<u64>,
    streak: u8,
}

struct RecentNackSnapshot {
    nack: XbxEngineVideoNackObservation,
    stalled_with_fresh_packets: bool,
    cloud_policy: bool,
}

pub(crate) fn resolve_recent_nack_outcome(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    reason: &VideoEscalationReason,
    stream_started_at: Instant,
    startup_grace: Duration,
    cloud_budget: &mut CloudStartupExpiredDeadlineBudget,
) -> Option<RecentNackOutcomeResolution> {
    let snapshot = read_recent_nack_snapshot(runtime_stats)?;
    let is_delta = snapshot.nack.frame_is_keyframe == Some(false)
        && matches!(snapshot.nack.frame_importance.as_deref(), Some("delta"));
    let is_important = snapshot.nack.frame_is_keyframe == Some(true)
        || matches!(
            snapshot.nack.frame_importance.as_deref(),
            Some("reference" | "keyframe")
        );
    let is_cloud_startup = snapshot.cloud_policy
        && matches!(
            resolve_session_phase(runtime_stats, stream_started_at, startup_grace),
            SessionPhase::Startup
        );

    match snapshot.nack.action.as_str() {
        "recovered" | "recoveredLate" => {
            cloud_budget.clear();
            if is_important
                && matches!(
                    reason,
                    VideoEscalationReason::WaitKeyframe
                        | VideoEscalationReason::TransportAwaitRecoveryKeyframe
                        | VideoEscalationReason::DisplaySupplyCritical
                        | VideoEscalationReason::AdapterIdleTimeout
                )
            {
                Some(RecentNackOutcomeResolution::Suppress(
                    RecoveryAction::WaitForBurst,
                ))
            } else {
                Some(RecentNackOutcomeResolution::Suppress(
                    RecoveryAction::CooldownSuppressed,
                ))
            }
        }
        "expiredDeadline" | "expiredMaxAge" => {
            if is_delta {
                if is_cloud_startup
                    && matches!(reason, VideoEscalationReason::TransportExpiredDeadline)
                {
                    if cloud_budget.update(&snapshot.nack) {
                        return Some(RecentNackOutcomeResolution::Escalate(
                            VideoEscalationReason::TransportExpiredDeadline,
                        ));
                    }
                    return Some(RecentNackOutcomeResolution::Suppress(
                        RecoveryAction::CooldownSuppressed,
                    ));
                }
                if snapshot.stalled_with_fresh_packets
                    && matches!(
                        reason,
                        VideoEscalationReason::TransportExpiredDeadline
                            | VideoEscalationReason::TransportSampleLoss
                            | VideoEscalationReason::WaitKeyframe
                            | VideoEscalationReason::AdapterIdleTimeout
                    )
                {
                    return Some(RecentNackOutcomeResolution::Escalate(
                        VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                    ));
                }
                Some(RecentNackOutcomeResolution::Suppress(
                    RecoveryAction::CooldownSuppressed,
                ))
            } else if is_important
                && matches!(
                    reason,
                    VideoEscalationReason::DisplaySupplyCritical
                        | VideoEscalationReason::TransportSampleLoss
                        | VideoEscalationReason::TransportAwaitRecoveryKeyframe
                        | VideoEscalationReason::WaitKeyframe
                )
            {
                let escalated_reason =
                    if matches!(reason, VideoEscalationReason::DisplaySupplyCritical) {
                        VideoEscalationReason::DisplaySupplyCritical
                    } else {
                        VideoEscalationReason::TransportSampleLoss
                    };
                Some(RecentNackOutcomeResolution::Escalate(escalated_reason))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn read_recent_nack_snapshot(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
) -> Option<RecentNackSnapshot> {
    let now_ms = unix_now_ms();
    RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        let nack = stats.latest_video_nack_observation.clone()?;
        let cloud_policy = matches!(
            stats.session_target_type,
            Some(xbxengine_protocol::XbxEngineTargetTypeDto::Cloud)
        ) || matches!(
            stats.transport_policy_profile.as_deref(),
            Some("cloudGaming")
        );
        let recent_window_ms = if cloud_policy {
            RECENT_NACK_OUTCOME_WINDOW_MS_CLOUD
        } else {
            RECENT_NACK_OUTCOME_WINDOW_MS
        };
        if now_ms - nack.observed_at_ms > recent_window_ms {
            return None;
        }
        let present_age_ms = stats
            .latest_video_host_present_time_ms
            .map(|at_ms| (now_ms - at_ms).max(0.0))
            .unwrap_or(f64::INFINITY);
        let packet_age_ms = stats
            .latest_video_packet_arrival_time_ms
            .map(|at_ms| (now_ms - at_ms).max(0.0))
            .unwrap_or(f64::INFINITY);
        Some(RecentNackSnapshot {
            nack,
            stalled_with_fresh_packets: stats.video_renderer_stalled.unwrap_or(false)
                && present_age_ms >= HARD_STALL_DECODER_RESET_MS
                && packet_age_ms <= HARD_STALL_DECODER_RESET_MS,
            cloud_policy,
        })
    })
    .flatten()
}

impl CloudStartupExpiredDeadlineBudget {
    fn update(&mut self, nack: &XbxEngineVideoNackObservation) -> bool {
        let current_observation_id = nack.observation_id;
        if self.last_observation_id == Some(current_observation_id) {
            return self.streak >= CLOUD_STARTUP_NACK_BUDGET_THRESHOLD;
        }

        let now_ms = nack.observed_at_ms.max(unix_now_ms());
        let same_window = self
            .first_seen_at_ms
            .map(|first_seen_at_ms| {
                now_ms - first_seen_at_ms <= CLOUD_STARTUP_NACK_BUDGET_WINDOW_MS
            })
            .unwrap_or(false);
        if same_window {
            self.streak = self.streak.saturating_add(1).max(1);
        } else {
            self.first_seen_at_ms = Some(now_ms);
            self.streak = 1;
        }
        self.last_observation_id = Some(current_observation_id);
        self.streak >= CLOUD_STARTUP_NACK_BUDGET_THRESHOLD
    }

    fn clear(&mut self) {
        self.first_seen_at_ms = None;
        self.last_observation_id = None;
        self.streak = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        resolve_recent_nack_outcome, CloudStartupExpiredDeadlineBudget, RecentNackOutcomeResolution,
    };
    use crate::transport::rtc::recovery::escalation::VideoEscalationReason;
    use crate::transport::rtc::recovery::runtime_state::unix_now_ms;
    use crate::{XbxEngineMediaRuntimeStats, XbxEngineVideoNackObservation};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    fn important_expired_nack(now_ms: f64) -> XbxEngineVideoNackObservation {
        XbxEngineVideoNackObservation {
            observation_id: 7,
            action: "expiredDeadline".to_string(),
            source: "sampleLoss".to_string(),
            first_sequence: 11,
            last_sequence: 12,
            packet_count: 2,
            retry_count: 1,
            frame_rtp_timestamp: Some(42),
            frame_is_keyframe: Some(true),
            frame_importance: Some("keyframe".to_string()),
            deadline_at_ms: None,
            estimated_recovery_arrival_ms: None,
            nack_disposition: Some("attempted".to_string()),
            frame_playout_deadline_at_ms: None,
            frame_unrecoverable_reason: None,
            frame_budget: None,
            observed_at_ms: now_ms,
        }
    }

    #[test]
    fn display_supply_critical_nack_outcome_is_not_rewritten_to_transport_await() {
        let now_ms = unix_now_ms();
        let stats = Mutex::new(XbxEngineMediaRuntimeStats {
            latest_video_nack_observation: Some(important_expired_nack(now_ms)),
            ..XbxEngineMediaRuntimeStats::default()
        });

        let resolution = resolve_recent_nack_outcome(
            &stats,
            &VideoEscalationReason::DisplaySupplyCritical,
            Instant::now() - Duration::from_secs(3),
            Duration::from_millis(800),
            &mut CloudStartupExpiredDeadlineBudget::default(),
        );

        assert!(matches!(
            resolution,
            Some(RecentNackOutcomeResolution::Escalate(
                VideoEscalationReason::DisplaySupplyCritical
            ))
        ));
    }
}
