use std::sync::Mutex;
use std::time::{Duration, Instant};

use xbxengine_protocol::XbxEngineTransportStateDto;

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::recovery::policy::{RecoveryScenarioProfile, ScenarioPolicyResolver};
use crate::transport::rtc::recovery::startup::{
    extract_startup_recovery_bitrate_kbps, resolve_session_phase, SessionPhase,
};
use crate::XbxEngineMediaRuntimeStats;

const HARD_STALL_DECODER_RESET_MS: f64 = 1_200.0;
const HARD_STALL_RECONNECT_MS: f64 = 3_000.0;
const AUDIO_ONLY_RECOVERY_LABEL_WINDOW_MS: f64 = HARD_STALL_RECONNECT_MS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecoveryCouplingMode {
    Healthy,
    StartupLowQuality,
    WaitingKeyframe,
    RecoveringReferenceChain,
    Stalled,
    ThinStream,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RecoveryCouplingState {
    pub(crate) mode: RecoveryCouplingMode,
    pub(crate) suppress_ramp_up: bool,
    pub(crate) prefer_hold: bool,
    pub(crate) allow_peak_range: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct RecoveryRuntimeState {
    pub(crate) phase: SessionPhase,
    pub(crate) recovery_policy_profile: &'static str,
    pub(crate) diagnosis_label: String,
    pub(crate) coupling: RecoveryCouplingState,
}

pub(crate) fn resolve_recovery_profile(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
) -> RecoveryScenarioProfile {
    let (session_target_type, transport_path) =
        RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            (
                stats.session_target_type.clone(),
                stats.transport_path.clone(),
            )
        })
        .unwrap_or((None, None));
    ScenarioPolicyResolver::resolve_recovery_profile(
        session_target_type.as_ref(),
        transport_path.as_deref(),
    )
}

pub(crate) fn current_profile_name(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
) -> &'static str {
    resolve_recovery_profile(runtime_stats).kind.as_str()
}

pub(crate) fn runtime_state_for_diagnosis(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    diagnosis_label: &str,
    stream_started_at: Instant,
    startup_grace: Duration,
) -> RecoveryRuntimeState {
    let phase = resolve_session_phase(runtime_stats, stream_started_at, startup_grace);
    let diagnosis_label = resolve_effective_diagnosis_label(runtime_stats, diagnosis_label);
    RecoveryRuntimeState {
        phase,
        recovery_policy_profile: current_profile_name(runtime_stats),
        diagnosis_label,
        coupling: current_coupling_state(runtime_stats, stream_started_at, startup_grace),
    }
}

pub(crate) fn current_coupling_state(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    stream_started_at: Instant,
    startup_grace: Duration,
) -> RecoveryCouplingState {
    let phase = resolve_session_phase(runtime_stats, stream_started_at, startup_grace);
    resolve_recovery_coupling_state(runtime_stats, phase)
}

fn resolve_effective_diagnosis_label(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    diagnosis_label: &str,
) -> String {
    if diagnosis_label == "adapterIdleTimeout"
        && RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            is_audio_only_track(stats) && has_recent_recovery_action(stats, unix_now_ms())
        })
        .unwrap_or(false)
    {
        return "healthy".to_string();
    }
    if diagnosis_label == "adapterIdleTimeout"
        && RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            has_fresh_media_output(stats, unix_now_ms())
                && !stats.video_decoder_stalled.unwrap_or(false)
                && !stats.video_renderer_stalled.unwrap_or(false)
        })
        .unwrap_or(false)
    {
        return "healthy".to_string();
    }
    if !matches!(
        diagnosis_label,
        "transportExpiredDeadline"
            | "transportSevereDeadline"
            | "transportSampleLoss"
            | "adapterIdleTimeout"
            | "transportAwaitRecoveryKeyframe"
            | "ingressWaitKeyframe"
    ) {
        return diagnosis_label.to_string();
    }
    let now_ms = unix_now_ms();
    RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        let profile = ScenarioPolicyResolver::resolve_recovery_profile(
            stats.session_target_type.as_ref(),
            stats.transport_path.as_deref(),
        );
        if decoder_backend_failure_signal_is_active(stats, profile, now_ms) {
            "decoderBackendFailure".to_string()
        } else {
            diagnosis_label.to_string()
        }
    })
    .unwrap_or_else(|| diagnosis_label.to_string())
}

pub(crate) fn resolve_recovery_coupling_state(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    phase: SessionPhase,
) -> RecoveryCouplingState {
    // recovery 只向 BWE 暴露当前恢复耦合状态，不直接决定目标码率。
    let Some((
        diagnosis,
        _effective_bitrate_kbps,
        _recovery_profile,
        stable_output,
        fresh_output,
        startup_low,
        decoder_stalled,
        renderer_stalled,
    )) = RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        let diagnosis = stats.recovery_diagnosis.clone();
        let effective_bitrate_kbps = extract_startup_recovery_bitrate_kbps(stats).unwrap_or(0.0);
        let recovery_profile = ScenarioPolicyResolver::resolve_recovery_profile(
            stats.session_target_type.as_ref(),
            stats.transport_path.as_deref(),
        );
        let stable_output = effective_bitrate_kbps
            >= recovery_profile.startup_low_quality_recovered_kbps
            && stats.video_present_fps >= 50.0;
        let fresh_output = has_fresh_media_output(stats, unix_now_ms());
        let startup_low = phase == SessionPhase::Startup
            && stats.direct_gaming_bitrate_band.as_deref() == Some("startupLow");
        Some((
            diagnosis,
            effective_bitrate_kbps,
            recovery_profile,
            stable_output,
            fresh_output,
            startup_low,
            stats.video_decoder_stalled.unwrap_or(false),
            stats.video_renderer_stalled.unwrap_or(false),
        ))
    })
    .flatten()
    else {
        return RecoveryCouplingState {
            mode: RecoveryCouplingMode::Healthy,
            suppress_ramp_up: false,
            prefer_hold: false,
            allow_peak_range: true,
        };
    };
    let stale_idle_timeout = diagnosis.as_deref() == Some("adapterIdleTimeout")
        && fresh_output
        && !decoder_stalled
        && !renderer_stalled;

    if startup_low {
        return RecoveryCouplingState {
            mode: RecoveryCouplingMode::StartupLowQuality,
            suppress_ramp_up: true,
            prefer_hold: true,
            allow_peak_range: false,
        };
    }

    // steady 且输出恢复后尽快退出 coupling，避免恢复后继续长期 hold。
    if phase == SessionPhase::Steady
        && (diagnosis.is_none() || stale_idle_timeout)
        && stable_output
        && !decoder_stalled
        && !renderer_stalled
    {
        return RecoveryCouplingState {
            mode: RecoveryCouplingMode::Healthy,
            suppress_ramp_up: false,
            prefer_hold: false,
            allow_peak_range: true,
        };
    }

    match diagnosis.as_deref() {
        Some("waitKeyframe" | "transportAwaitRecoveryKeyframe" | "ingressWaitKeyframe") => {
            RecoveryCouplingState {
                mode: RecoveryCouplingMode::WaitingKeyframe,
                suppress_ramp_up: true,
                prefer_hold: true,
                allow_peak_range: false,
            }
        }
        Some("transportSampleLoss" | "reconfigure" | "ingressReconfigure") => {
            RecoveryCouplingState {
                mode: RecoveryCouplingMode::RecoveringReferenceChain,
                suppress_ramp_up: true,
                prefer_hold: true,
                allow_peak_range: false,
            }
        }
        Some("adapterThinStream" | "thinStream") => RecoveryCouplingState {
            mode: RecoveryCouplingMode::ThinStream,
            suppress_ramp_up: true,
            prefer_hold: true,
            allow_peak_range: false,
        },
        Some("adapterIdleTimeout" | "decoderBackendFailure") if !stale_idle_timeout => {
            RecoveryCouplingState {
                mode: RecoveryCouplingMode::Stalled,
                suppress_ramp_up: true,
                prefer_hold: true,
                allow_peak_range: false,
            }
        }
        Some("adapterIdleTimeout") if stale_idle_timeout => RecoveryCouplingState {
            mode: RecoveryCouplingMode::Healthy,
            suppress_ramp_up: false,
            prefer_hold: false,
            allow_peak_range: true,
        },
        Some("decoderBackendFailure") => RecoveryCouplingState {
            mode: RecoveryCouplingMode::Stalled,
            suppress_ramp_up: true,
            prefer_hold: true,
            allow_peak_range: false,
        },
        _ if phase == SessionPhase::Recovering && !stable_output => RecoveryCouplingState {
            mode: RecoveryCouplingMode::RecoveringReferenceChain,
            suppress_ramp_up: true,
            prefer_hold: true,
            allow_peak_range: false,
        },
        _ => RecoveryCouplingState {
            mode: RecoveryCouplingMode::Healthy,
            suppress_ramp_up: false,
            prefer_hold: false,
            allow_peak_range: true,
        },
    }
}

pub(crate) fn unix_now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

pub(crate) fn has_fresh_media_output(stats: &XbxEngineMediaRuntimeStats, now_ms: f64) -> bool {
    const FRESH_MEDIA_OUTPUT_WINDOW_MS: f64 = 500.0;
    let present_fresh = stats
        .latest_video_present_time_ms
        .map(|at_ms| now_ms - at_ms < FRESH_MEDIA_OUTPUT_WINDOW_MS)
        .unwrap_or(false);
    let decode_fresh = stats
        .latest_video_decode_ok_time_ms
        .map(|at_ms| now_ms - at_ms < FRESH_MEDIA_OUTPUT_WINDOW_MS)
        .unwrap_or(false);
    present_fresh || decode_fresh || stats.video_present_fps >= 10.0
}

pub(crate) fn decoder_backend_failure_signal_is_active(
    stats: &XbxEngineMediaRuntimeStats,
    profile: RecoveryScenarioProfile,
    now_ms: f64,
) -> bool {
    if stats.transport_state != XbxEngineTransportStateDto::Connected {
        return false;
    }
    if stats.video_decoder_hardware_failure_streak
        < profile.decoder_backend_failure_min_consecutive_failures
    {
        return false;
    }
    let failure_age_ms = stats
        .latest_video_decoder_hardware_failure_time_ms
        .map(|at_ms| (now_ms - at_ms).max(0.0))
        .unwrap_or(f64::INFINITY);
    if failure_age_ms > profile.decoder_backend_failure_recent_window_ms {
        return false;
    }
    let packet_age_ms = stats
        .latest_video_packet_arrival_time_ms
        .map(|at_ms| (now_ms - at_ms).max(0.0))
        .unwrap_or(f64::INFINITY);
    if packet_age_ms > profile.decoder_backend_failure_max_packet_age_ms {
        return false;
    }
    let decode_age_ms = stats
        .latest_video_decode_ok_time_ms
        .map(|at_ms| (now_ms - at_ms).max(0.0))
        .unwrap_or(f64::INFINITY);
    let present_age_ms = stats
        .latest_video_present_time_ms
        .map(|at_ms| (now_ms - at_ms).max(0.0))
        .unwrap_or(f64::INFINITY);
    let pipeline_not_advancing = stats.video_renderer_stalled.unwrap_or(false)
        || decode_age_ms >= HARD_STALL_DECODER_RESET_MS
        || present_age_ms >= HARD_STALL_DECODER_RESET_MS;
    if !pipeline_not_advancing {
        return false;
    }
    let Some((delivery_ratio, loss_ratio)) = extract_twcc_health_ratios(stats) else {
        return false;
    };
    delivery_ratio >= profile.decoder_backend_failure_min_twcc_delivery_ratio
        && loss_ratio <= profile.decoder_backend_failure_max_twcc_loss_ratio
}

fn has_recent_recovery_action(stats: &XbxEngineMediaRuntimeStats, now_ms: f64) -> bool {
    stats
        .latest_video_escalation_observation
        .as_ref()
        .is_some_and(|observation| {
            observation.action != "cooldownSuppressed"
                && now_ms - observation.observed_at_ms <= AUDIO_ONLY_RECOVERY_LABEL_WINDOW_MS
        })
}

fn is_audio_only_track(stats: &XbxEngineMediaRuntimeStats) -> bool {
    stats
        .latest_video_track_status
        .as_ref()
        .is_some_and(|status| {
            status.state == "audioOnly"
                && status.video_bytes_total == 0
                && status.audio_bytes_total > 0
                && status.transport_state == XbxEngineTransportStateDto::Connected
        })
}

fn extract_twcc_health_ratios(stats: &XbxEngineMediaRuntimeStats) -> Option<(f64, f64)> {
    if let Some(observation) = stats.latest_video_twcc_observation.as_ref() {
        return Some((observation.delivery_ratio, observation.packet_loss_ratio));
    }
    stats
        .latest_video_bwe_observation
        .as_ref()
        .and_then(|observation| {
            observation
                .twcc_delivery_ratio
                .zip(observation.twcc_loss_ratio)
        })
}
