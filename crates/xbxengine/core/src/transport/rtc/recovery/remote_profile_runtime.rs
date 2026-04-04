use xbxengine_protocol::{
    compose_effective_remote_profile_label, XbxEngineRemoteProfileKindDto,
    XbxEngineRemoteSubprofileKindDto, XbxEngineTargetTypeDto,
};

use crate::XbxEngineMediaRuntimeStats;

const CLOUD_HIGH_RTT_MS: f64 = 95.0;
const FRESH_OUTPUT_WINDOW_MS: f64 = 450.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RuntimeRemoteProfileClassification {
    pub(crate) baseline: XbxEngineRemoteProfileKindDto,
    pub(crate) dynamic: XbxEngineRemoteSubprofileKindDto,
}

impl RuntimeRemoteProfileClassification {
    pub(crate) fn effective_label(&self) -> String {
        compose_effective_remote_profile_label(self.baseline, self.dynamic)
    }
}

pub(crate) fn classify_runtime_remote_profile(
    runtime_stats: Option<&XbxEngineMediaRuntimeStats>,
    now_ms: f64,
) -> Option<RuntimeRemoteProfileClassification> {
    let stats = runtime_stats?;
    let baseline = resolve_runtime_baseline_profile_kind(stats);
    let dynamic = classify_dynamic_subprofile(stats, baseline, now_ms);
    Some(RuntimeRemoteProfileClassification { baseline, dynamic })
}

pub(crate) fn resolve_runtime_baseline_profile_kind(
    stats: &XbxEngineMediaRuntimeStats,
) -> XbxEngineRemoteProfileKindDto {
    resolve_runtime_profile_kind(
        stats.baseline_remote_profile.as_deref(),
        stats.session_target_type.as_ref(),
        stats.transport_path.as_deref(),
    )
}

pub(crate) fn resolve_runtime_profile_kind(
    baseline_remote_profile: Option<&str>,
    session_target_type: Option<&XbxEngineTargetTypeDto>,
    transport_path: Option<&str>,
) -> XbxEngineRemoteProfileKindDto {
    baseline_remote_profile
        .and_then(XbxEngineRemoteProfileKindDto::from_str)
        .unwrap_or_else(|| {
            XbxEngineRemoteProfileKindDto::resolve(session_target_type, transport_path)
        })
}

pub(crate) fn persist_runtime_remote_profile_facts(
    runtime_stats: &mut XbxEngineMediaRuntimeStats,
    now_ms: f64,
) {
    let Some(classification) = classify_runtime_remote_profile(Some(runtime_stats), now_ms) else {
        runtime_stats.baseline_remote_profile = None;
        runtime_stats.dynamic_remote_subprofile = None;
        runtime_stats.effective_remote_profile_label = None;
        return;
    };
    runtime_stats.baseline_remote_profile = Some(classification.baseline.as_str().to_string());
    runtime_stats.dynamic_remote_subprofile = Some(classification.dynamic.as_str().to_string());
    runtime_stats.effective_remote_profile_label = Some(classification.effective_label());
}

fn classify_dynamic_subprofile(
    stats: &XbxEngineMediaRuntimeStats,
    baseline: XbxEngineRemoteProfileKindDto,
    now_ms: f64,
) -> XbxEngineRemoteSubprofileKindDto {
    if stats.video_decoder_stalled.unwrap_or(false)
        || stats.video_decoder_hardware_failure_streak > 0
    {
        return XbxEngineRemoteSubprofileKindDto::DecoderConstrained;
    }

    if is_display_constrained(stats, now_ms) {
        return XbxEngineRemoteSubprofileKindDto::DisplayConstrained;
    }

    if baseline.is_cloud() && is_cloud_startup(stats) {
        return XbxEngineRemoteSubprofileKindDto::CloudStartup;
    }

    if baseline.is_cloud()
        && resolve_video_rtt_ms(stats).is_some_and(|rtt_ms| rtt_ms >= CLOUD_HIGH_RTT_MS)
    {
        return XbxEngineRemoteSubprofileKindDto::CloudHighRtt;
    }

    XbxEngineRemoteSubprofileKindDto::Steady
}

fn is_cloud_startup(stats: &XbxEngineMediaRuntimeStats) -> bool {
    if stats.direct_gaming_bitrate_band.as_deref() == Some("startupLow") {
        return true;
    }
    matches!(
        stats.session_phase.as_deref(),
        Some("startup" | "handshaking" | "priming")
    )
}

fn is_display_constrained(stats: &XbxEngineMediaRuntimeStats, now_ms: f64) -> bool {
    if stats.video_renderer_stalled.unwrap_or(false) {
        return true;
    }

    let pressure_level = stats
        .host_no_pending_pressure_level
        .as_deref()
        .unwrap_or_default();
    let pressure_hot = matches!(pressure_level, "high" | "critical");
    if !pressure_hot {
        return false;
    }

    !has_fresh_media_output(stats, now_ms)
}

fn has_fresh_media_output(stats: &XbxEngineMediaRuntimeStats, now_ms: f64) -> bool {
    let decode_fresh = stats
        .latest_video_decode_ok_time_ms
        .is_some_and(|at_ms| (now_ms - at_ms).max(0.0) <= FRESH_OUTPUT_WINDOW_MS);
    let present_fresh = stats
        .latest_video_host_present_time_ms
        .is_some_and(|at_ms| (now_ms - at_ms).max(0.0) <= FRESH_OUTPUT_WINDOW_MS);
    decode_fresh || present_fresh
}

fn resolve_video_rtt_ms(stats: &XbxEngineMediaRuntimeStats) -> Option<f64> {
    stats.video_rtt_ms.or_else(|| {
        stats
            .latest_video_bwe_observation
            .as_ref()
            .and_then(|obs| obs.rtt_ms)
    })
}

#[cfg(test)]
mod tests {
    use xbxengine_protocol::XbxEngineTargetTypeDto;

    use super::classify_runtime_remote_profile;
    use crate::XbxEngineMediaRuntimeStats;

    #[test]
    fn classify_cloud_startup_from_band() {
        let stats = XbxEngineMediaRuntimeStats {
            session_target_type: Some(XbxEngineTargetTypeDto::Cloud),
            direct_gaming_bitrate_band: Some("startupLow".to_string()),
            ..XbxEngineMediaRuntimeStats::default()
        };
        let profile = classify_runtime_remote_profile(Some(&stats), 10_000.0).expect("profile");
        assert_eq!(profile.baseline.as_str(), "cloudGaming");
        assert_eq!(profile.dynamic.as_str(), "cloudStartup");
        assert_eq!(profile.effective_label(), "cloudGaming+cloudStartup");
    }

    #[test]
    fn classify_cloud_high_rtt() {
        let stats = XbxEngineMediaRuntimeStats {
            session_target_type: Some(XbxEngineTargetTypeDto::Cloud),
            video_rtt_ms: Some(120.0),
            ..XbxEngineMediaRuntimeStats::default()
        };
        let profile = classify_runtime_remote_profile(Some(&stats), 10_000.0).expect("profile");
        assert_eq!(profile.dynamic.as_str(), "cloudHighRtt");
    }

    #[test]
    fn decoder_constrained_has_higher_priority() {
        let stats = XbxEngineMediaRuntimeStats {
            session_target_type: Some(XbxEngineTargetTypeDto::Cloud),
            video_rtt_ms: Some(180.0),
            video_decoder_stalled: Some(true),
            ..XbxEngineMediaRuntimeStats::default()
        };
        let profile = classify_runtime_remote_profile(Some(&stats), 10_000.0).expect("profile");
        assert_eq!(profile.dynamic.as_str(), "decoderConstrained");
    }

    #[test]
    fn display_constrained_uses_pressure_plus_stale_freshness() {
        let stats = XbxEngineMediaRuntimeStats {
            host_no_pending_pressure_level: Some("critical".to_string()),
            latest_video_decode_ok_time_ms: Some(1_000.0),
            latest_video_host_present_time_ms: Some(1_000.0),
            ..XbxEngineMediaRuntimeStats::default()
        };
        let profile = classify_runtime_remote_profile(Some(&stats), 2_000.0).expect("profile");
        assert_eq!(profile.dynamic.as_str(), "displayConstrained");
    }
}
