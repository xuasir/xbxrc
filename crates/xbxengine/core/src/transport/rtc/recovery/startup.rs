use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::recovery::escalation::VideoEscalationReason;
use crate::transport::rtc::recovery::escalation_label::escalation_structured_label;
use crate::transport::rtc::recovery::policy::RecoveryScenarioProfile;
use crate::transport::rtc::recovery::runtime_state::{
    displayed_idr_output_pipeline_active, renderer_shadow_blocks_serviceability,
    resolve_runtime_recovery_profile,
};
use crate::XbxEngineMediaRuntimeStats;

pub const STARTUP_LOW_QUALITY_RETRY_DELAY_MS: u64 = 320;
pub const STARTUP_LOW_QUALITY_FLOOR_KBPS: f64 = 8_000.0;
pub const STARTUP_LOW_QUALITY_RECOVERED_KBPS: f64 = 12_000.0;
const STARTUP_LOW_QUALITY_MAX_RETRIES: u8 = 2;
const RECOVERING_PHASE_WINDOW_MS: f64 = 1_500.0;
const STEADY_PRESENT_FPS: f64 = 40.0;
const RECOVERING_PRESENT_FPS: f64 = 35.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionPhase {
    Startup,
    Steady,
    Recovering,
}

impl SessionPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionPhase::Startup => "startup",
            SessionPhase::Steady => "steady",
            SessionPhase::Recovering => "recovering",
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StartupRecoveryProbe {
    armed_at: Option<Instant>,
    retry_count: u8,
}

impl StartupRecoveryProbe {
    pub fn arm(&mut self, now: Instant) {
        self.armed_at = Some(now);
        self.retry_count = 0;
    }

    pub fn clear(&mut self) {
        self.armed_at = None;
        self.retry_count = 0;
    }

    pub fn should_retry_low_quality(
        &mut self,
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        stream_started_at: Instant,
        startup_grace: Duration,
        retry_delay: Duration,
        low_bitrate_kbps: f64,
        recovered_bitrate_kbps: f64,
    ) -> bool {
        if stream_started_at.elapsed() >= startup_grace {
            self.clear();
            return false;
        }

        let Some(armed_at) = self.armed_at else {
            return false;
        };
        if self.retry_count >= STARTUP_LOW_QUALITY_MAX_RETRIES {
            return false;
        }

        let Some((effective_bitrate, waiting_for_clean_video)) =
            RuntimeStatsSink::read_shared(runtime_stats, |stats| {
                let effective_bitrate = extract_startup_recovery_bitrate_kbps(stats);
                let waiting_for_clean_video = matches!(
                    escalation_structured_label(stats),
                    Some(
                        "ingressWaitKeyframe" | "ingressFrameAbandoned" | "receiverWaitingKeyframe"
                    )
                ) || stats.direct_gaming_bitrate_band.as_deref()
                    == Some("startupLow");
                (effective_bitrate, waiting_for_clean_video)
            })
        else {
            return false;
        };
        if effective_bitrate
            .map(|value| value >= recovered_bitrate_kbps)
            .unwrap_or(false)
        {
            self.clear();
            return false;
        }

        if armed_at.elapsed() < retry_delay {
            return false;
        }
        let should_retry = effective_bitrate
            .map(|value| value < low_bitrate_kbps)
            .unwrap_or(waiting_for_clean_video)
            || (effective_bitrate == Some(0.0) && waiting_for_clean_video);
        if should_retry {
            self.retry_count = self.retry_count.saturating_add(1);
            self.armed_at = Some(Instant::now());
        }
        should_retry
    }
}

pub fn resolve_session_phase(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    stream_started_at: Instant,
    startup_grace: Duration,
) -> SessionPhase {
    RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        resolve_session_phase_from_stats(Some(stats), stream_started_at, startup_grace)
    })
    .unwrap_or_else(|| resolve_session_phase_from_stats(None, stream_started_at, startup_grace))
}

pub(crate) fn resolve_session_phase_from_stats(
    stats: Option<&XbxEngineMediaRuntimeStats>,
    stream_started_at: Instant,
    startup_grace: Duration,
) -> SessionPhase {
    if stream_started_at.elapsed() < startup_grace {
        let Some(stats) = stats else {
            return SessionPhase::Startup;
        };
        let profile = resolve_recovery_profile(stats);
        let effective_bitrate = extract_startup_recovery_bitrate_kbps(stats).unwrap_or(0.0);
        if effective_bitrate >= profile.startup_low_quality_recovered_kbps
            && stats.video_present_fps >= STEADY_PRESENT_FPS
        {
            return SessionPhase::Steady;
        }
        return SessionPhase::Startup;
    }

    let Some(stats) = stats else {
        return SessionPhase::Steady;
    };
    let profile = resolve_recovery_profile(stats);
    let effective_bitrate = extract_startup_recovery_bitrate_kbps(stats).unwrap_or(0.0);
    let render_age_ms = stats
        .latest_video_host_present_time_ms
        .map(|at_ms| (now_ms_f64() - at_ms).max(0.0))
        .unwrap_or(f64::INFINITY);
    let active_recovery_escalation = matches!(
        escalation_structured_label(stats),
        Some(
            "waitKeyframe"
                | "ingressWaitKeyframe"
                | "ingressFrameAbandoned"
                | "receiverWaitingKeyframe"
                | "transportExpiredDeadline"
                | "transportSevereDeadline"
                | "transportSampleLoss"
                | "decoderBackendFailure"
                | "ingressReconfigure"
                | "reconfigure"
                | "adapterThinStream"
                | "displaySupplyCritical"
                | "localSupplySuspect"
        )
    );
    let stalled_output = stats.video_decoder_stalled.unwrap_or(false)
        || renderer_shadow_blocks_serviceability(stats, now_ms_f64())
        || (render_age_ms >= RECOVERING_PHASE_WINDOW_MS
            && stats.video_present_fps < RECOVERING_PRESENT_FPS);
    let degraded_output = effective_bitrate > 0.0
        && effective_bitrate < profile.startup_low_quality_recovered_kbps
        && stats.video_present_fps < RECOVERING_PRESENT_FPS;
    if (active_recovery_escalation || stalled_output || degraded_output)
        && !should_hold_steady_session_phase_during_displayed_idr_serving(stats)
        && !recovery_exit_timed_fallback_allows_steady_phase(stats)
    {
        SessionPhase::Recovering
    } else {
        SessionPhase::Steady
    }
}

fn recovery_exit_timed_fallback_allows_steady_phase(stats: &XbxEngineMediaRuntimeStats) -> bool {
    use crate::transport::rtc::recovery::contract::{
        recovery_exit_path_from_stats, RecoveryExitPath, RecoveryExitThresholds,
    };
    matches!(
        recovery_exit_path_from_stats(stats, now_ms_f64(), RecoveryExitThresholds::default()),
        RecoveryExitPath::TimedFallback | RecoveryExitPath::DecodeOutput
    ) && matches!(
        escalation_structured_label(stats),
        Some("receiverWaitingKeyframe" | "ingressWaitKeyframe" | "ingressFrameAbandoned")
    )
}

fn should_hold_steady_session_phase_during_displayed_idr_serving(
    stats: &XbxEngineMediaRuntimeStats,
) -> bool {
    if !crate::transport::rtc::recovery::contract::displayed_idr_serving_from_stats(stats) {
        return false;
    }
    if crate::transport::rtc::recovery::contract::displayed_idr_serving_relaxation_blocked_from_stats(
        stats,
        now_ms_f64(),
    ) {
        return false;
    }
    let host_steady_cadence = matches!(stats.host_cadence_phase.as_deref(), Some("steady"));
    let escalation = escalation_structured_label(stats);
    let waiting_keyframe_escalation = matches!(
        escalation,
        Some("receiverWaitingKeyframe" | "ingressWaitKeyframe")
    );
    if host_steady_cadence && waiting_keyframe_escalation {
        return true;
    }
    if !displayed_idr_output_pipeline_active(stats, now_ms_f64()) {
        return false;
    }
    matches!(
        escalation,
        Some(
            "receiverWaitingKeyframe"
                | "ingressWaitKeyframe"
                | "localSupplySuspect"
                | "rebuildingSupplySuspect"
        )
    ) || matches!(
        stats.video_owner_state.as_deref(),
        Some("stable-serving" | "degraded-serving")
    )
}

pub fn should_suppress_startup_escalation(
    reason: &VideoEscalationReason,
    stream_started_at: Instant,
    startup_grace: Duration,
) -> bool {
    if stream_started_at.elapsed() >= startup_grace {
        return false;
    }

    matches!(reason, VideoEscalationReason::Reconfigure)
}

pub fn should_fast_reset_startup_recovery(
    reason: &VideoEscalationReason,
    stream_started_at: Instant,
    startup_grace: Duration,
) -> bool {
    stream_started_at.elapsed() < startup_grace
        && matches!(
            reason,
            VideoEscalationReason::TransportSampleLoss
                | VideoEscalationReason::WaitKeyframe
                | VideoEscalationReason::DisplaySupplyCritical
                | VideoEscalationReason::LocalSupplySuspect
                | VideoEscalationReason::AdapterIdleTimeout
        )
}

pub fn extract_startup_recovery_bitrate_kbps(stats: &XbxEngineMediaRuntimeStats) -> Option<f64> {
    stats
        .inbound_video_bitrate_kbps
        .or_else(|| {
            stats
                .latest_video_bwe_observation
                .as_ref()
                .map(|observation| observation.actual_video_bitrate_kbps)
        })
        .or_else(|| {
            stats
                .latest_video_bwe_observation
                .as_ref()
                .and_then(|observation| observation.twcc_receive_bitrate_kbps)
        })
        .or_else(|| {
            stats
                .latest_video_twcc_observation
                .as_ref()
                .and_then(|observation| observation.receive_bitrate_kbps)
        })
}

fn now_ms_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

fn resolve_recovery_profile(stats: &XbxEngineMediaRuntimeStats) -> RecoveryScenarioProfile {
    resolve_runtime_recovery_profile(stats)
}

#[cfg(test)]
mod tests {
    use super::{
        extract_startup_recovery_bitrate_kbps, now_ms_f64, resolve_session_phase_from_stats,
        should_fast_reset_startup_recovery, should_suppress_startup_escalation, SessionPhase,
        StartupRecoveryProbe,
    };
    use crate::transport::rtc::recovery::escalation::VideoEscalationReason;
    use crate::{
        XbxEngineMediaRuntimeStats, XbxEngineVideoBweObservation, XbxEngineVideoTwccObservation,
    };
    use std::{
        sync::Mutex,
        time::{Duration, Instant},
    };

    #[test]
    fn startup_grace_does_not_suppress_wait_keyframe() {
        let stream_started_at = Instant::now();
        assert!(!should_suppress_startup_escalation(
            &VideoEscalationReason::WaitKeyframe,
            stream_started_at,
            Duration::from_secs(2),
        ));
    }

    #[test]
    fn startup_grace_only_suppresses_reconfigure() {
        let stream_started_at = Instant::now();
        assert!(!should_suppress_startup_escalation(
            &VideoEscalationReason::AdapterIdleTimeout,
            stream_started_at,
            Duration::from_secs(2),
        ));
        assert!(should_suppress_startup_escalation(
            &VideoEscalationReason::Reconfigure,
            stream_started_at,
            Duration::from_secs(2),
        ));
    }

    #[test]
    fn startup_grace_fast_resets_startup_recovery_reasons_only() {
        let stream_started_at = Instant::now();
        assert!(should_fast_reset_startup_recovery(
            &VideoEscalationReason::TransportSampleLoss,
            stream_started_at,
            Duration::from_secs(2),
        ));
        assert!(should_fast_reset_startup_recovery(
            &VideoEscalationReason::WaitKeyframe,
            stream_started_at,
            Duration::from_secs(2),
        ));
        assert!(should_fast_reset_startup_recovery(
            &VideoEscalationReason::DisplaySupplyCritical,
            stream_started_at,
            Duration::from_secs(2),
        ));
        assert!(should_fast_reset_startup_recovery(
            &VideoEscalationReason::AdapterIdleTimeout,
            stream_started_at,
            Duration::from_secs(2),
        ));
        assert!(!should_fast_reset_startup_recovery(
            &VideoEscalationReason::Reconfigure,
            stream_started_at,
            Duration::from_secs(2),
        ));
        std::thread::sleep(Duration::from_millis(20));
        assert!(!should_fast_reset_startup_recovery(
            &VideoEscalationReason::WaitKeyframe,
            stream_started_at,
            Duration::from_millis(5),
        ));
        assert!(!should_fast_reset_startup_recovery(
            &VideoEscalationReason::TransportSampleLoss,
            stream_started_at,
            Duration::from_millis(5),
        ));
    }

    #[test]
    fn startup_low_quality_probe_retries_when_bitrate_stays_low() {
        let stream_started_at = Instant::now();
        let mut probe = StartupRecoveryProbe::default();
        probe.arm(Instant::now());
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.inbound_video_bitrate_kbps = Some(6_500.0);
        let runtime_stats = Mutex::new(stats);

        std::thread::sleep(Duration::from_millis(20));
        assert!(probe.should_retry_low_quality(
            &runtime_stats,
            stream_started_at,
            Duration::from_secs(2),
            Duration::from_millis(5),
            8_000.0,
            12_000.0,
        ));
        std::thread::sleep(Duration::from_millis(20));
        assert!(probe.should_retry_low_quality(
            &runtime_stats,
            stream_started_at,
            Duration::from_secs(2),
            Duration::from_millis(5),
            8_000.0,
            12_000.0,
        ));
        assert!(!probe.should_retry_low_quality(
            &runtime_stats,
            stream_started_at,
            Duration::from_secs(2),
            Duration::from_millis(5),
            8_000.0,
            12_000.0,
        ));
    }

    #[test]
    fn startup_low_quality_probe_clears_after_bitrate_recovers() {
        let stream_started_at = Instant::now();
        let mut probe = StartupRecoveryProbe::default();
        probe.arm(Instant::now());
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.latest_video_bwe_observation = Some(XbxEngineVideoBweObservation {
            observation_id: 1,
            mode: "twcc-gcc".to_string(),
            decision_reason: "hold".to_string(),
            target_remb_kbps: 30_000,
            observed_remb_kbps: None,
            actual_video_bitrate_kbps: 14_500.0,
            loss_ratio: 0.0,
            rtt_ms: None,
            transport_path: Some("Direct".to_string()),
            twcc_feedback_interval_ms: Some(100.0),
            twcc_observed_packet_count: Some(120),
            twcc_covered_sequence_span: Some(120),
            twcc_receive_bitrate_kbps: Some(14_800.0),
            twcc_delivery_ratio: Some(1.0),
            twcc_loss_ratio: Some(0.0),
            observed_at_ms: 1.0,
        });
        let runtime_stats = Mutex::new(stats);

        std::thread::sleep(Duration::from_millis(20));
        assert!(!probe.should_retry_low_quality(
            &runtime_stats,
            stream_started_at,
            Duration::from_secs(2),
            Duration::from_millis(5),
            8_000.0,
            12_000.0,
        ));
        assert!(matches!(probe, StartupRecoveryProbe { .. }));
    }

    #[test]
    fn startup_low_quality_probe_retries_zero_bitrate_while_waiting_keyframe() {
        let stream_started_at = Instant::now();
        let mut probe = StartupRecoveryProbe::default();
        probe.arm(Instant::now());
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.inbound_video_bitrate_kbps = Some(0.0);
        stats.recovery_active_escalation_reason = Some("ingressWaitKeyframe".to_string());
        stats.direct_gaming_bitrate_band = Some("startupLow".to_string());
        let runtime_stats = Mutex::new(stats);

        std::thread::sleep(Duration::from_millis(20));
        assert!(probe.should_retry_low_quality(
            &runtime_stats,
            stream_started_at,
            Duration::from_secs(2),
            Duration::from_millis(5),
            8_000.0,
            12_000.0,
        ));
    }

    #[test]
    fn startup_recovery_bitrate_prefers_real_video_rate() {
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.inbound_video_bitrate_kbps = Some(9_500.0);
        stats.latest_video_bwe_observation = Some(XbxEngineVideoBweObservation {
            observation_id: 1,
            mode: "twcc-gcc".to_string(),
            decision_reason: "hold".to_string(),
            target_remb_kbps: 30_000,
            observed_remb_kbps: None,
            actual_video_bitrate_kbps: 8_800.0,
            loss_ratio: 0.0,
            rtt_ms: None,
            transport_path: Some("Direct".to_string()),
            twcc_feedback_interval_ms: Some(100.0),
            twcc_observed_packet_count: Some(100),
            twcc_covered_sequence_span: Some(100),
            twcc_receive_bitrate_kbps: Some(10_000.0),
            twcc_delivery_ratio: Some(1.0),
            twcc_loss_ratio: Some(0.0),
            observed_at_ms: 1.0,
        });
        stats.latest_video_twcc_observation = Some(XbxEngineVideoTwccObservation {
            observation_id: 1,
            source: "local-feedback".to_string(),
            feedback_packet_count: 1,
            covered_sequence_start: 1,
            covered_sequence_end: 100,
            covered_sequence_span: 100,
            observed_packet_count: 100,
            observed_byte_count: 100_000,
            coverage_ratio: None,
            ledger_hit_ratio: None,
            feedback_interval_ms: Some(100.0),
            arrival_span_ms: Some(100.0),
            receive_bitrate_kbps: Some(10_500.0),
            twcc_sample_valid: true,

            twcc_invalid_reason: None,

            quality: crate::XbxEngineTwccObservationQuality::Stable,
            delivery_ratio: 1.0,
            packet_loss_ratio: 0.0,
            observed_at_ms: 1.0,
        });

        assert_eq!(extract_startup_recovery_bitrate_kbps(&stats), Some(9_500.0));
    }

    #[test]
    fn session_phase_exits_startup_after_output_stabilizes() {
        let stream_started_at = Instant::now();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.inbound_video_bitrate_kbps = Some(14_000.0);
        stats.video_present_fps = 59.0;
        assert_eq!(
            resolve_session_phase_from_stats(
                Some(&stats),
                stream_started_at,
                Duration::from_secs(2),
            ),
            SessionPhase::Steady
        );
    }

    #[test]
    fn session_phase_marks_active_recovery_escalation_as_recovering() {
        let stream_started_at = Instant::now() - Duration::from_secs(5);
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.inbound_video_bitrate_kbps = Some(9_000.0);
        stats.video_present_fps = 42.0;
        stats.recovery_active_escalation_reason = Some("receiverWaitingKeyframe".to_string());
        assert_eq!(
            resolve_session_phase_from_stats(
                Some(&stats),
                stream_started_at,
                Duration::from_secs(2),
            ),
            SessionPhase::Recovering
        );
    }

    #[test]
    fn session_phase_holds_steady_on_waiting_keyframe_when_displayed_idr_and_host_steady_cadence() {
        let stream_started_at = Instant::now() - Duration::from_secs(5);
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.recovery_displayed_idr_at_ms = Some(now_ms_f64());
        stats.host_cadence_phase = Some("steady".to_string());
        stats.recovery_active_escalation_reason = Some("receiverWaitingKeyframe".to_string());
        stats.inbound_video_bitrate_kbps = Some(14_000.0);
        stats.video_present_fps = 18.0;
        assert_eq!(
            resolve_session_phase_from_stats(
                Some(&stats),
                stream_started_at,
                Duration::from_secs(2),
            ),
            SessionPhase::Steady
        );
    }

    #[test]
    fn session_phase_holds_steady_when_displayed_idr_pipeline_active() {
        let stream_started_at = Instant::now() - Duration::from_secs(5);
        let mut stats = XbxEngineMediaRuntimeStats::default();
        let now_ms = now_ms_f64();
        stats.recovery_displayed_idr_at_ms = Some(now_ms);
        stats.latest_video_decode_ok_time_ms = Some(now_ms);
        stats.latest_video_host_present_time_ms = Some(now_ms);
        stats.video_owner_state = Some("stable-serving".to_string());
        stats.recovery_active_escalation_reason = Some("receiverWaitingKeyframe".to_string());
        stats.inbound_video_bitrate_kbps = Some(14_000.0);
        stats.video_present_fps = 18.0;
        assert_eq!(
            resolve_session_phase_from_stats(
                Some(&stats),
                stream_started_at,
                Duration::from_secs(2),
            ),
            SessionPhase::Steady
        );
    }

    #[test]
    fn session_phase_ignores_stale_adapter_idle_timeout_when_output_is_fresh() {
        let stream_started_at = Instant::now() - Duration::from_secs(5);
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.inbound_video_bitrate_kbps = Some(14_000.0);
        stats.video_present_fps = 59.0;
        stats.recovery_active_escalation_reason = Some("adapterIdleTimeout".to_string());
        assert_eq!(
            resolve_session_phase_from_stats(
                Some(&stats),
                stream_started_at,
                Duration::from_secs(2),
            ),
            SessionPhase::Steady
        );
    }

    #[test]
    fn session_phase_ignores_shadow_renderer_stall_when_host_present_is_fresh() {
        let stream_started_at = Instant::now() - Duration::from_secs(5);
        let now_ms = crate::transport::rtc::stats::now_ms_f64();
        let mut stats = XbxEngineMediaRuntimeStats::default();
        stats.inbound_video_bitrate_kbps = Some(14_000.0);
        stats.video_present_fps = 59.0;
        stats.video_renderer_stalled = Some(true);
        stats.host_no_pending_pressure_level = Some("normal".to_string());
        stats.latest_video_host_present_time_ms = Some(now_ms);
        stats.latest_video_decode_ok_time_ms = Some(now_ms);
        assert_eq!(
            resolve_session_phase_from_stats(
                Some(&stats),
                stream_started_at,
                Duration::from_secs(2),
            ),
            SessionPhase::Steady
        );
    }
}
