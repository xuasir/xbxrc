use std::sync::Mutex;
#[cfg(test)]
use std::time::{Duration, Instant};

use xbxengine_protocol::{XbxEngineRemoteProfileKindDto, XbxEngineTransportStateDto};

use crate::runtime_stats_sink::RuntimeStatsSink;
use crate::transport::rtc::recovery::escalation_label::escalation_structured_label;
use crate::transport::rtc::recovery::policy::{RecoveryScenarioProfile, ScenarioPolicyResolver};
use crate::transport::rtc::recovery::remote_profile_runtime::{
    classify_runtime_remote_profile, resolve_runtime_baseline_profile_kind,
};
#[cfg(test)]
use crate::transport::rtc::recovery::startup::resolve_session_phase;
use crate::transport::rtc::recovery::startup::{
    extract_startup_recovery_bitrate_kbps, SessionPhase,
};
use crate::XbxEngineMediaRuntimeStats;

const HARD_STALL_DECODER_RESET_MS: f64 = 1_200.0;
const HARD_STALL_RECONNECT_MS: f64 = 3_000.0;
const AUDIO_ONLY_RECOVERY_LABEL_WINDOW_MS: f64 = HARD_STALL_RECONNECT_MS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecoveryOwnerMode {
    Healthy,
    StartupLowQuality,
    WaitingKeyframe,
    RecoveringReferenceChain,
    Stalled,
    ThinStream,
}

#[derive(Clone, Debug)]
pub(crate) struct RecoveryInputProfile {
    pub(crate) baseline: String,
    pub(crate) effective_label: String,
}

#[derive(Clone, Debug)]
pub(crate) struct RecoveryPrimaryView {
    pub(crate) owner_state: String,
    pub(crate) owner_reason: String,
}

#[derive(Clone, Debug)]
pub(crate) struct RecoveryEscalationContext {
    pub(crate) stage: String,
    pub(crate) chain_value: String,
    pub(crate) failure_cost: String,
    pub(crate) window_source: String,
}

#[derive(Clone, Debug)]
pub(crate) struct RecoveryRuntimeState {
    pub(crate) phase: SessionPhase,
    pub(crate) input_profile: RecoveryInputProfile,
    pub(crate) diagnosis_label: String,
    pub(crate) primary_view: RecoveryPrimaryView,
}

pub(crate) fn resolve_recovery_profile(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
) -> RecoveryScenarioProfile {
    RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        resolve_runtime_recovery_profile(stats)
    })
    .unwrap_or_else(|| {
        ScenarioPolicyResolver::resolve_recovery_profile_by_kind(
            XbxEngineRemoteProfileKindDto::resolve(None, None),
        )
    })
}

pub(crate) fn project_runtime_state_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
) -> RecoveryRuntimeState {
    let now_ms = unix_now_ms();
    let phase = project_phase_from_stats(stats);
    // 有效标签与吸收规则只基于结构化链；`recovery_diagnosis` 不参与 `RecoveryRuntimeState` 推导（展示由 diagnostics 单独解析）。
    let raw_for_effective_label = escalation_structured_label(stats).unwrap_or("healthy");
    let diagnosis_label =
        resolve_effective_diagnosis_label_from_stats(stats, raw_for_effective_label, now_ms);
    RecoveryRuntimeState {
        phase,
        input_profile: resolve_input_profile_from_stats(stats, now_ms),
        primary_view: primary_view_from_stats(stats, phase, &diagnosis_label),
        diagnosis_label,
    }
}

pub(crate) fn project_recovery_escalation_context(
    stats: &XbxEngineMediaRuntimeStats,
    reason: &str,
    action: &str,
) -> RecoveryEscalationContext {
    RecoveryEscalationContext {
        stage: recovery_stage_label(stats).to_string(),
        chain_value: recovery_chain_value_label(reason).to_string(),
        failure_cost: recovery_failure_cost_label(action).to_string(),
        window_source: recovery_window_source_label(stats, reason).to_string(),
    }
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) mod test_support {
    use super::*;

    pub(crate) fn runtime_state_for_diagnosis(
        runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
        diagnosis_label: &str,
        stream_started_at: Instant,
        startup_grace: Duration,
    ) -> RecoveryRuntimeState {
        let phase = resolve_session_phase(runtime_stats, stream_started_at, startup_grace);
        let diagnosis_label =
            resolve_effective_diagnosis_label_for_test(runtime_stats, diagnosis_label);
        let input_profile = RuntimeStatsSink::read_shared(runtime_stats, |stats| {
            resolve_input_profile_from_stats(stats, unix_now_ms())
        })
        .unwrap_or_else(|| {
            let baseline = resolve_recovery_profile(runtime_stats)
                .kind
                .as_str()
                .to_string();
            RecoveryInputProfile {
                effective_label: baseline.clone(),
                baseline,
            }
        });
        RecoveryRuntimeState {
            phase,
            input_profile,
            primary_view: current_primary_view_for_test(
                runtime_stats,
                phase,
                &diagnosis_label,
                stream_started_at,
                startup_grace,
            ),
            diagnosis_label,
        }
    }
}

#[cfg(test)]
fn current_primary_view_for_test(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    phase: SessionPhase,
    diagnosis_label: &str,
    stream_started_at: Instant,
    startup_grace: Duration,
) -> RecoveryPrimaryView {
    let owner_mode = current_owner_mode_for_test(runtime_stats, stream_started_at, startup_grace);
    RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        Some(primary_view_from_stats_with_fallback(
            stats,
            phase,
            diagnosis_label,
            owner_mode,
        ))
    })
    .unwrap_or_else(|| {
        Some(primary_view_from_mode(
            phase,
            diagnosis_label,
            RecoveryOwnerMode::Healthy,
        ))
    })
    .expect("primary view fallback")
}

#[cfg(test)]
fn current_owner_mode_for_test(
    runtime_stats: &Mutex<XbxEngineMediaRuntimeStats>,
    stream_started_at: Instant,
    startup_grace: Duration,
) -> RecoveryOwnerMode {
    let phase = resolve_session_phase(runtime_stats, stream_started_at, startup_grace);
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
        let diagnosis = escalation_structured_label(stats).map(str::to_string);
        let effective_bitrate_kbps = extract_startup_recovery_bitrate_kbps(stats).unwrap_or(0.0);
        let baseline_profile = resolve_runtime_baseline_profile_kind(stats);
        let recovery_profile =
            ScenarioPolicyResolver::resolve_recovery_profile_by_kind(baseline_profile);
        let fresh_output = has_fresh_media_output(stats, unix_now_ms());
        let stable_output = effective_bitrate_kbps
            >= recovery_profile.startup_low_quality_recovered_kbps
            && fresh_output
            && has_serviceable_display_continuity(stats);
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
            renderer_shadow_blocks_serviceability(stats, unix_now_ms()),
        ))
    })
    .flatten()
    else {
        return RecoveryOwnerMode::Healthy;
    };
    resolve_recovery_owner_mode_by_signals(
        diagnosis.as_deref(),
        phase,
        stable_output,
        fresh_output,
        startup_low,
        decoder_stalled,
        renderer_stalled,
    )
}

#[cfg(test)]
fn resolve_effective_diagnosis_label_for_test(
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
                && !renderer_shadow_blocks_serviceability(stats, unix_now_ms())
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
            | "receiverWaitingKeyframe"
            | "ingressWaitKeyframe"
            | "ingressFrameAbandoned"
    ) {
        return diagnosis_label.to_string();
    }
    let now_ms = unix_now_ms();
    RuntimeStatsSink::read_shared(runtime_stats, |stats| {
        resolve_effective_diagnosis_label_from_stats(stats, diagnosis_label, now_ms)
    })
    .unwrap_or_else(|| diagnosis_label.to_string())
}

fn resolve_input_profile_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> RecoveryInputProfile {
    let classified = classify_runtime_remote_profile(Some(stats), now_ms);
    let baseline = stats
        .baseline_remote_profile
        .clone()
        .or_else(|| {
            classified
                .as_ref()
                .map(|profile| profile.baseline.as_str().to_string())
        })
        .unwrap_or_else(|| {
            resolve_runtime_baseline_profile_kind(stats)
                .as_str()
                .to_string()
        });
    let effective_label = stats
        .effective_remote_profile_label
        .clone()
        .or_else(|| classified.map(|profile| profile.effective_label()))
        .unwrap_or_else(|| baseline.clone());
    RecoveryInputProfile {
        baseline,
        effective_label,
    }
}

fn primary_view_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    phase: SessionPhase,
    diagnosis_label: &str,
) -> RecoveryPrimaryView {
    let owner_mode = current_owner_mode_from_stats(stats, phase);
    primary_view_from_stats_with_fallback(stats, phase, diagnosis_label, owner_mode)
}

fn primary_view_from_stats_with_fallback(
    stats: &XbxEngineMediaRuntimeStats,
    phase: SessionPhase,
    diagnosis_label: &str,
    owner_mode: RecoveryOwnerMode,
) -> RecoveryPrimaryView {
    if let Some(owner_state) = stats.video_owner_state.clone() {
        return RecoveryPrimaryView {
            owner_reason: stats
                .video_owner_reason
                .clone()
                .unwrap_or_else(|| diagnosis_label.to_string()),
            owner_state,
        };
    }
    primary_view_from_mode(phase, diagnosis_label, owner_mode)
}

fn primary_view_from_mode(
    phase: SessionPhase,
    diagnosis_label: &str,
    owner_mode: RecoveryOwnerMode,
) -> RecoveryPrimaryView {
    let owner_state = match owner_mode {
        RecoveryOwnerMode::Healthy => {
            if phase == SessionPhase::Startup {
                "priming"
            } else {
                "stable-serving"
            }
        }
        RecoveryOwnerMode::StartupLowQuality
        | RecoveryOwnerMode::WaitingKeyframe
        | RecoveryOwnerMode::RecoveringReferenceChain => "rebuilding-supply",
        RecoveryOwnerMode::Stalled => "rebuilding-supply",
        RecoveryOwnerMode::ThinStream => "supply-starved",
    };
    RecoveryPrimaryView {
        owner_state: owner_state.to_string(),
        owner_reason: diagnosis_label.to_string(),
    }
}

fn project_phase_from_stats(stats: &XbxEngineMediaRuntimeStats) -> SessionPhase {
    match stats.session_phase.as_deref() {
        Some("startup" | "handshaking" | "priming") => SessionPhase::Startup,
        Some(
            "recovering" | "observing" | "local-self-healing" | "recovery-eligible"
            | "active-recovery" | "recovery-blocked",
        ) => SessionPhase::Recovering,
        _ => SessionPhase::Steady,
    }
}

fn recovery_stage_label(stats: &XbxEngineMediaRuntimeStats) -> &'static str {
    if stats.transport_recovery_episode_active
        && stats.transport_state != XbxEngineTransportStateDto::Connected
    {
        return "reconnecting";
    }
    if matches!(
        stats.session_phase.as_deref(),
        Some("startup" | "handshaking" | "priming")
    ) {
        return "priming";
    }
    if matches!(
        stats.session_phase.as_deref(),
        Some("observing" | "local-self-healing")
    ) {
        return "observe-anomaly";
    }
    if matches!(stats.session_phase.as_deref(), Some("recovery-eligible")) {
        return "recovery-eligible";
    }
    if matches!(stats.session_phase.as_deref(), Some("active-recovery")) {
        return "active-recovery";
    }
    if matches!(stats.session_phase.as_deref(), Some("recovery-blocked")) {
        return "recovery-blocked";
    }
    if stats.transport_recovery_episode_active
        && stats
            .video_anchor_clean_epoch
            .is_some_and(|epoch| epoch == stats.transport_recovery_epoch)
        && matches!(stats.video_owner_state.as_deref(), Some("stable-serving"))
    {
        return "ramp-up";
    }
    if owner_state_has_steady_output_semantics(stats)
        && has_fresh_media_output(stats, unix_now_ms())
        && !stats.video_decoder_stalled.unwrap_or(false)
        && !renderer_shadow_blocks_serviceability(stats, unix_now_ms())
    {
        return "steady";
    }
    if matches!(
        stats.video_owner_state.as_deref(),
        Some("rebuilding-supply" | "supply-starved")
    ) || matches!(
        escalation_structured_label(stats),
        Some(
            "waitKeyframe"
                | "receiverWaitingKeyframe"
                | "ingressWaitKeyframe"
                | "ingressFrameAbandoned"
                | "transportExpiredDeadline"
                | "transportSevereDeadline"
                | "transportSampleLoss"
                | "decoderBackendFailure"
                | "reconfigure"
                | "ingressReconfigure"
        )
    ) {
        return "rebuilding-supply";
    }
    "steady"
}

fn recovery_chain_value_label(reason: &str) -> &'static str {
    match reason {
        "waitKeyframe"
        | "receiverWaitingKeyframe"
        | "ingressWaitKeyframe"
        | "ingressFrameAbandoned" => "anchor",
        "reconfigure"
        | "decoderBackendFailure"
        | "transportSampleLoss"
        | "transportRecoveredLate" => "supply",
        "adapterThinStream" | "thinStream" => "disposable",
        "adapterIdleTimeout" | "transportExpiredDeadline" | "transportSevereDeadline" => "health",
        "lifecycleRecovering" => "connectivity",
        _ => "health",
    }
}

fn recovery_failure_cost_label(action: &str) -> &'static str {
    match action {
        "requestReconnectCandidate" => "high",
        "requestDecoderReset"
        | "requestPli+decoderReset"
        | "requestPli+decoderReset(startupLowQualityRetry)" => "medium",
        "requestPli"
        | "startupLowQualityRetry"
        | "coalesced:keyframeInFlight"
        | "coalesced:decoderResetInFlight" => "medium",
        "waitForBurst"
        | "waitForDecoderResetBurst"
        | "cooldownSuppressed"
        | "startupGraceSuppressed" => "low",
        _ => "medium",
    }
}

fn recovery_window_source_label(stats: &XbxEngineMediaRuntimeStats, reason: &str) -> &'static str {
    if stats.recovery_hard_fallback_timer_ms.is_some() {
        return "hard-fallback-window";
    }
    if matches!(
        reason,
        "waitKeyframe"
            | "receiverWaitingKeyframe"
            | "ingressWaitKeyframe"
            | "ingressFrameAbandoned"
    ) {
        return "transport-await-window";
    }
    if matches!(
        reason,
        "adapterIdleTimeout" | "decoderBackendFailure" | "reconfigure" | "ingressReconfigure"
    ) {
        return "local-maintenance-window";
    }
    if matches!(
        reason,
        "transportExpiredDeadline" | "transportSevereDeadline"
    ) {
        return "hard-stall-window";
    }
    if matches!(reason, "transportSampleLoss" | "transportRecoveredLate") {
        return "nack-window";
    }
    if matches!(reason, "lifecycleRecovering")
        || (stats.transport_recovery_episode_active
            && stats.transport_state != XbxEngineTransportStateDto::Connected)
    {
        return "reconnect-window";
    }
    if matches!(
        stats.session_phase.as_deref(),
        Some("startup" | "handshaking" | "priming")
    ) {
        return "startup-grace";
    }
    "session-phase-window"
}

fn resolve_effective_diagnosis_label_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    diagnosis_label: &str,
    now_ms: f64,
) -> String {
    if should_absorb_stale_recovery_diagnosis(stats, diagnosis_label, now_ms) {
        return "healthy".to_string();
    }
    if diagnosis_label == "adapterIdleTimeout"
        && is_audio_only_track(stats)
        && has_recent_recovery_action(stats, now_ms)
    {
        return "healthy".to_string();
    }
    if diagnosis_label == "adapterIdleTimeout"
        && has_fresh_media_output(stats, now_ms)
        && !stats.video_decoder_stalled.unwrap_or(false)
        && !renderer_shadow_blocks_serviceability(stats, now_ms)
    {
        return "healthy".to_string();
    }
    if !matches!(
        diagnosis_label,
        "transportExpiredDeadline"
            | "transportSevereDeadline"
            | "transportSampleLoss"
            | "adapterIdleTimeout"
            | "receiverWaitingKeyframe"
            | "ingressWaitKeyframe"
            | "ingressFrameAbandoned"
    ) {
        return diagnosis_label.to_string();
    }
    let profile = resolve_runtime_recovery_profile_at(stats, now_ms);
    if decoder_backend_failure_signal_is_active(stats, profile, now_ms) {
        "decoderBackendFailure".to_string()
    } else {
        diagnosis_label.to_string()
    }
}

fn current_owner_mode_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    phase: SessionPhase,
) -> RecoveryOwnerMode {
    let diagnosis = escalation_structured_label(stats).map(str::to_string);
    let effective_bitrate_kbps = extract_startup_recovery_bitrate_kbps(stats).unwrap_or(0.0);
    let baseline_profile = resolve_runtime_baseline_profile_kind(stats);
    let recovery_profile =
        ScenarioPolicyResolver::resolve_recovery_profile_by_kind(baseline_profile);
    let fresh_output = has_fresh_media_output(stats, unix_now_ms());
    let stable_output = effective_bitrate_kbps
        >= recovery_profile.startup_low_quality_recovered_kbps
        && fresh_output
        && has_serviceable_display_continuity(stats);
    let startup_low = phase == SessionPhase::Startup
        && stats.direct_gaming_bitrate_band.as_deref() == Some("startupLow");
    let decoder_stalled = stats.video_decoder_stalled.unwrap_or(false);
    let renderer_stalled = renderer_shadow_blocks_serviceability(stats, unix_now_ms());
    resolve_recovery_owner_mode_by_signals(
        diagnosis.as_deref(),
        phase,
        stable_output,
        fresh_output,
        startup_low,
        decoder_stalled,
        renderer_stalled,
    )
}

fn resolve_recovery_owner_mode_by_signals(
    diagnosis: Option<&str>,
    phase: SessionPhase,
    stable_output: bool,
    fresh_output: bool,
    startup_low: bool,
    decoder_stalled: bool,
    renderer_stalled: bool,
) -> RecoveryOwnerMode {
    let stale_idle_timeout = diagnosis == Some("adapterIdleTimeout")
        && fresh_output
        && !decoder_stalled
        && !renderer_stalled;

    if startup_low {
        return RecoveryOwnerMode::StartupLowQuality;
    }
    if phase == SessionPhase::Steady
        && (diagnosis.is_none() || stale_idle_timeout)
        && stable_output
        && !decoder_stalled
        && !renderer_stalled
    {
        return RecoveryOwnerMode::Healthy;
    }

    match diagnosis {
        Some(
            "waitKeyframe"
            | "receiverWaitingKeyframe"
            | "ingressWaitKeyframe"
            | "ingressFrameAbandoned",
        ) => RecoveryOwnerMode::WaitingKeyframe,
        Some("transportSampleLoss" | "reconfigure" | "ingressReconfigure") => {
            RecoveryOwnerMode::RecoveringReferenceChain
        }
        Some("adapterThinStream" | "thinStream") => RecoveryOwnerMode::ThinStream,
        Some("adapterIdleTimeout" | "decoderBackendFailure") if !stale_idle_timeout => {
            RecoveryOwnerMode::Stalled
        }
        Some("adapterIdleTimeout") if stale_idle_timeout => RecoveryOwnerMode::Healthy,
        Some("decoderBackendFailure") => RecoveryOwnerMode::Stalled,
        _ if phase == SessionPhase::Recovering && !stable_output => {
            RecoveryOwnerMode::RecoveringReferenceChain
        }
        _ => RecoveryOwnerMode::Healthy,
    }
}

pub(crate) fn unix_now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

pub(crate) fn has_fresh_media_output(stats: &XbxEngineMediaRuntimeStats, now_ms: f64) -> bool {
    const FRESH_MEDIA_OUTPUT_WINDOW_MS: f64 = 300.0;
    const FUTURE_TIMESTAMP_CLOCK_SKEW_GUARD_MS: f64 = 10_000.0;
    let runtime_now_ms = unix_now_ms();
    let effective_now_ms = [
        stats.latest_video_host_present_time_ms,
        stats.latest_video_decode_ok_time_ms,
    ]
    .into_iter()
    .flatten()
    .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal))
    .filter(|latest_at_ms| *latest_at_ms > now_ms + FUTURE_TIMESTAMP_CLOCK_SKEW_GUARD_MS)
    .map(|_| runtime_now_ms)
    .unwrap_or(now_ms);
    let present_fresh = stats
        .latest_video_host_present_time_ms
        .map(|at_ms| (effective_now_ms - at_ms).max(0.0) < FRESH_MEDIA_OUTPUT_WINDOW_MS)
        .unwrap_or(false);
    let decode_fresh = stats
        .latest_video_decode_ok_time_ms
        .map(|at_ms| (effective_now_ms - at_ms).max(0.0) < FRESH_MEDIA_OUTPUT_WINDOW_MS)
        .unwrap_or(false);
    // 只检查真实事件时间戳，不使用平滑 FPS 指标避免误判
    present_fresh || decode_fresh
}

pub(crate) fn host_presentation_serviceable(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    const HOST_PRESENT_SERVICEABLE_WINDOW_MS: f64 = 300.0;
    let pressure_hot = matches!(
        stats.host_no_pending_pressure_level.as_deref(),
        Some("high" | "critical")
    );
    if pressure_hot {
        return false;
    }
    stats
        .latest_video_host_present_time_ms
        .is_some_and(|at_ms| (now_ms - at_ms).max(0.0) < HOST_PRESENT_SERVICEABLE_WINDOW_MS)
}

pub(crate) fn renderer_shadow_blocks_serviceability(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    stats.video_renderer_stalled.unwrap_or(false) && !host_presentation_serviceable(stats, now_ms)
}

fn owner_state_has_steady_output_semantics(stats: &XbxEngineMediaRuntimeStats) -> bool {
    matches!(
        stats.video_owner_state.as_deref(),
        Some("stable-serving" | "degraded-serving")
    )
}

fn should_absorb_stale_recovery_diagnosis(
    stats: &XbxEngineMediaRuntimeStats,
    diagnosis_label: &str,
    now_ms: f64,
) -> bool {
    if !matches!(
        diagnosis_label,
        "receiverWaitingKeyframe"
            | "transportExpiredDeadline"
            | "transportSevereDeadline"
            | "transportSampleLoss"
    ) {
        return false;
    }
    owner_state_has_steady_output_semantics(stats)
        && has_fresh_media_output(stats, now_ms)
        && !stats.video_decoder_stalled.unwrap_or(false)
        && !renderer_shadow_blocks_serviceability(stats, now_ms)
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
        .latest_video_host_present_time_ms
        .map(|at_ms| (now_ms - at_ms).max(0.0))
        .unwrap_or(f64::INFINITY);
    let pipeline_not_advancing = renderer_shadow_blocks_serviceability(stats, now_ms)
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

pub(crate) fn resolve_runtime_recovery_profile(
    stats: &XbxEngineMediaRuntimeStats,
) -> RecoveryScenarioProfile {
    resolve_runtime_recovery_profile_at(stats, unix_now_ms())
}

fn resolve_runtime_recovery_profile_at(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> RecoveryScenarioProfile {
    let baseline_profile = classify_runtime_remote_profile(Some(stats), now_ms)
        .map(|profile| profile.baseline)
        .unwrap_or_else(|| resolve_runtime_baseline_profile_kind(stats));
    ScenarioPolicyResolver::resolve_recovery_profile_by_kind(baseline_profile)
}

#[cfg(test)]
mod tests {
    use super::{
        current_owner_mode_from_stats, recovery_stage_label,
        resolve_effective_diagnosis_label_from_stats, unix_now_ms, RecoveryOwnerMode,
    };
    use crate::transport::rtc::recovery::startup::SessionPhase;
    use crate::XbxEngineMediaRuntimeStats;

    #[test]
    fn degraded_owner_absorbs_stale_transport_expired_deadline_when_output_is_fresh() {
        let now_ms = unix_now_ms();
        let stats = XbxEngineMediaRuntimeStats {
            session_phase: Some("recovering".to_string()),
            recovery_active_escalation_reason: Some("transportExpiredDeadline".to_string()),
            video_owner_state: Some("degraded-serving".to_string()),
            latest_video_host_present_time_ms: Some(now_ms - 18.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 12.0),
            video_present_fps: 60.0,
            video_decoder_stalled: Some(false),
            video_renderer_stalled: Some(false),
            ..XbxEngineMediaRuntimeStats::default()
        };

        assert_eq!(
            resolve_effective_diagnosis_label_from_stats(
                &stats,
                "transportExpiredDeadline",
                now_ms
            ),
            "healthy"
        );
        assert_eq!(recovery_stage_label(&stats), "steady");
    }

    #[test]
    fn degraded_owner_does_not_absorb_transport_await_when_shadow_stall_blocks_host_service() {
        let now_ms = unix_now_ms();
        let stats = XbxEngineMediaRuntimeStats {
            session_phase: Some("recovering".to_string()),
            recovery_active_escalation_reason: Some("receiverWaitingKeyframe".to_string()),
            video_owner_state: Some("degraded-serving".to_string()),
            latest_video_host_present_time_ms: Some(now_ms - 18.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 12.0),
            host_no_pending_pressure_level: Some("critical".to_string()),
            video_present_fps: 60.0,
            video_decoder_stalled: Some(false),
            video_renderer_stalled: Some(true),
            ..XbxEngineMediaRuntimeStats::default()
        };

        assert_eq!(
            resolve_effective_diagnosis_label_from_stats(&stats, "receiverWaitingKeyframe", now_ms),
            "receiverWaitingKeyframe"
        );
        assert_eq!(recovery_stage_label(&stats), "rebuilding-supply");
    }

    #[test]
    fn degraded_owner_absorbs_transport_await_when_renderer_stall_is_only_shadow_signal() {
        let now_ms = unix_now_ms();
        let stats = XbxEngineMediaRuntimeStats {
            session_phase: Some("recovering".to_string()),
            recovery_active_escalation_reason: Some("receiverWaitingKeyframe".to_string()),
            video_owner_state: Some("degraded-serving".to_string()),
            latest_video_host_present_time_ms: Some(now_ms - 18.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 12.0),
            host_no_pending_pressure_level: Some("normal".to_string()),
            video_present_fps: 60.0,
            video_decoder_stalled: Some(false),
            video_renderer_stalled: Some(true),
            ..XbxEngineMediaRuntimeStats::default()
        };

        assert_eq!(
            resolve_effective_diagnosis_label_from_stats(&stats, "receiverWaitingKeyframe", now_ms),
            "healthy"
        );
        assert_eq!(recovery_stage_label(&stats), "steady");
    }

    #[test]
    fn recovering_phase_without_fresh_output_does_not_return_healthy_from_stale_fps() {
        let now_ms = unix_now_ms();
        let stats = XbxEngineMediaRuntimeStats {
            session_phase: Some("recovering".to_string()),
            inbound_video_bitrate_kbps: Some(20_000.0),
            video_present_fps: 60.0,
            latest_video_host_present_time_ms: Some(now_ms - 1_200.0),
            latest_video_decode_ok_time_ms: Some(now_ms - 1_100.0),
            video_decoder_stalled: Some(false),
            video_renderer_stalled: Some(false),
            ..XbxEngineMediaRuntimeStats::default()
        };

        assert_eq!(
            current_owner_mode_from_stats(&stats, SessionPhase::Recovering),
            RecoveryOwnerMode::RecoveringReferenceChain
        );
    }
}
const SERVICEABLE_PRESENT_FPS: f64 = 35.0;

fn has_serviceable_display_continuity(stats: &XbxEngineMediaRuntimeStats) -> bool {
    stats.video_present_fps >= SERVICEABLE_PRESENT_FPS
        || stats.recovery_playback_recovered_at_ms.is_some()
}
