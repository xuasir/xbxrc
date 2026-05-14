//! 动态 RTT 感知恢复时序（RFC 2026-05-14）。
//! 单一入口解析 NACK / PLI / FIR / decoded pending / patience 等毫秒阈值。

use crate::api::backend::XbxEngineMediaRuntimeStats;
use crate::transport::rtc::recovery::policy::{
    RecoveryScenarioProfile, ScenarioPolicyProfileKind, ScenarioPolicyResolver,
};

#[inline]
fn clamp_ms(value: f64, floor: f64, ceiling: f64) -> f64 {
    value.clamp(floor, ceiling)
}

/// 场景默认 RTT（毫秒），在尚无 `video_rtt_ms` 时使用。
pub(crate) fn default_rtt_ms_for_kind(kind: ScenarioPolicyProfileKind) -> f64 {
    match kind {
        ScenarioPolicyProfileKind::HomeLanGaming => 10.0,
        ScenarioPolicyProfileKind::RelayGaming => 100.0,
        ScenarioPolicyProfileKind::CloudGaming => 200.0,
    }
}

fn nack_timeout_floor_ceiling(kind: ScenarioPolicyProfileKind) -> (f64, f64) {
    match kind {
        ScenarioPolicyProfileKind::HomeLanGaming => (45.0, 90.0),
        ScenarioPolicyProfileKind::RelayGaming => (120.0, 240.0),
        ScenarioPolicyProfileKind::CloudGaming => (240.0, 420.0),
    }
}

/// 每拍更新平滑 RTT（上升快、下降慢，单步尖峰限制），写入 `stats.recovery_smoothed_rtt_ms`。
pub(crate) fn advance_recovery_rtt_smoothing(stats: &mut XbxEngineMediaRuntimeStats) {
    let kind = ScenarioPolicyResolver::resolve_kind(
        stats.session_target_type.as_ref(),
        stats.transport_path.as_deref(),
    );
    let raw = stats
        .video_rtt_ms
        .unwrap_or_else(|| default_rtt_ms_for_kind(kind))
        .clamp(5.0, 800.0);
    let prev = stats.recovery_smoothed_rtt_ms;
    let new = match prev {
        None => raw,
        Some(p) => {
            let delta = raw - p;
            let capped = if delta.abs() > 100.0 {
                delta.signum() * 100.0
            } else {
                delta
            };
            let rate = if capped > 0.0 { 0.45 } else { 0.12 };
            (p + capped * rate).clamp(5.0, 800.0)
        }
    };
    stats.recovery_smoothed_rtt_ms = Some(new);
}

/// 当前拍用于时序解析的有效 RTT（优先平滑值）。
pub(crate) fn resolve_effective_rtt_ms(
    stats: &XbxEngineMediaRuntimeStats,
    kind: ScenarioPolicyProfileKind,
) -> f64 {
    stats
        .recovery_smoothed_rtt_ms
        .or(stats.video_rtt_ms)
        .unwrap_or_else(|| default_rtt_ms_for_kind(kind))
        .clamp(5.0, 800.0)
}

/// 由 RTT + 场景边界解析出的动态恢复时序（毫秒域）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RecoveryDynamicTiming {
    pub(crate) effective_rtt_ms: f64,
    pub(crate) nack_timeout_ms: f64,
    pub(crate) nack_retry_interval_ms: f64,
    pub(crate) pli_refresh_interval_ms: f64,
    pub(crate) fir_retry_interval_ms: f64,
    pub(crate) decoded_pending_commit_hold_ms: f64,
    pub(crate) continuation_patience_window_ms: f64,
    pub(crate) clean_anchor_commit_patience_window_ms: f64,
}

fn dynamic_nack_timeout_ms(effective_rtt_ms: f64, kind: ScenarioPolicyProfileKind) -> f64 {
    let (floor, ceiling) = nack_timeout_floor_ceiling(kind);
    clamp_ms(1.6 * effective_rtt_ms + 40.0, floor, ceiling)
}

fn dynamic_nack_retry_interval_ms(effective_rtt_ms: f64) -> f64 {
    clamp_ms(0.75 * effective_rtt_ms + 8.0, 12.0, 140.0)
}

/// NACK 重试间隔（毫秒，整数），供 `nack_policy` / scheduler 使用。
pub(crate) fn nack_retry_interval_u64_from_rtt_ms(rtt_ms: f64) -> u64 {
    dynamic_nack_retry_interval_ms(rtt_ms.max(0.0))
        .round()
        .clamp(4.0, 140.0) as u64
}

fn dynamic_pli_refresh_interval_ms(effective_rtt_ms: f64) -> f64 {
    clamp_ms(0.8 * effective_rtt_ms + 20.0, 40.0, 220.0)
}

fn dynamic_fir_retry_interval_ms(effective_rtt_ms: f64) -> f64 {
    clamp_ms(2.5 * effective_rtt_ms + 60.0, 180.0, 700.0)
}

fn dynamic_decoded_pending_commit_hold_ms(
    effective_rtt_ms: f64,
    kind: ScenarioPolicyProfileKind,
) -> f64 {
    let (floor, ceiling) = match kind {
        ScenarioPolicyProfileKind::HomeLanGaming => (120.0, 220.0),
        ScenarioPolicyProfileKind::RelayGaming => (160.0, 300.0),
        ScenarioPolicyProfileKind::CloudGaming => (220.0, 420.0),
    };
    clamp_ms(1.15 * effective_rtt_ms + 70.0, floor, ceiling)
}

fn dynamic_continuation_patience_ms(effective_rtt_ms: f64, kind: ScenarioPolicyProfileKind) -> f64 {
    let (floor, ceiling) = match kind {
        ScenarioPolicyProfileKind::HomeLanGaming => (100.0, 260.0),
        ScenarioPolicyProfileKind::RelayGaming => (140.0, 380.0),
        ScenarioPolicyProfileKind::CloudGaming => (180.0, 480.0),
    };
    clamp_ms(1.25 * effective_rtt_ms + 90.0, floor, ceiling)
}

fn dynamic_clean_anchor_patience_ms(_effective_rtt_ms: f64, hold_ms: f64) -> f64 {
    clamp_ms(hold_ms * 1.15, hold_ms + 20.0, 520.0)
}

/// 纯函数：给定有效 RTT 与 profile（用于 kind 边界），解析全部动态时序。
pub(crate) fn resolve_recovery_dynamic_timing_with_rtt(
    effective_rtt_ms: f64,
    profile: RecoveryScenarioProfile,
) -> RecoveryDynamicTiming {
    let kind = profile.kind;
    let effective_rtt_ms = effective_rtt_ms.clamp(5.0, 800.0);
    let nack_timeout_ms = dynamic_nack_timeout_ms(effective_rtt_ms, kind);
    let nack_retry_interval_ms = dynamic_nack_retry_interval_ms(effective_rtt_ms);
    let pli_refresh_interval_ms = dynamic_pli_refresh_interval_ms(effective_rtt_ms);
    let fir_retry_interval_ms = dynamic_fir_retry_interval_ms(effective_rtt_ms);
    let decoded_pending_commit_hold_ms =
        dynamic_decoded_pending_commit_hold_ms(effective_rtt_ms, kind);
    let continuation_patience_window_ms = dynamic_continuation_patience_ms(effective_rtt_ms, kind);
    let clean_anchor_commit_patience_window_ms =
        dynamic_clean_anchor_patience_ms(effective_rtt_ms, decoded_pending_commit_hold_ms);

    RecoveryDynamicTiming {
        effective_rtt_ms,
        nack_timeout_ms,
        nack_retry_interval_ms,
        pli_refresh_interval_ms,
        fir_retry_interval_ms,
        decoded_pending_commit_hold_ms,
        continuation_patience_window_ms,
        clean_anchor_commit_patience_window_ms,
    }
}

/// 从 runtime stats + 场景 profile 解析动态时序（使用平滑 RTT）。
pub(crate) fn resolve_recovery_dynamic_timing(
    stats: &XbxEngineMediaRuntimeStats,
    profile: RecoveryScenarioProfile,
) -> RecoveryDynamicTiming {
    let effective = resolve_effective_rtt_ms(stats, profile.kind);
    resolve_recovery_dynamic_timing_with_rtt(effective, profile)
}

/// 将解析结果写入 stats，供 diagnostics / trace 读取。
pub(crate) fn publish_recovery_timing_to_stats(
    stats: &mut XbxEngineMediaRuntimeStats,
    timing: &RecoveryDynamicTiming,
) {
    stats.recovery_effective_rtt_ms = Some(timing.effective_rtt_ms);
    stats.recovery_dynamic_nack_timeout_ms = Some(timing.nack_timeout_ms);
    stats.recovery_dynamic_nack_retry_interval_ms = Some(timing.nack_retry_interval_ms);
    stats.recovery_dynamic_pli_refresh_interval_ms = Some(timing.pli_refresh_interval_ms);
    stats.recovery_dynamic_fir_retry_interval_ms = Some(timing.fir_retry_interval_ms);
    stats.recovery_dynamic_decoded_pending_commit_hold_ms =
        Some(timing.decoded_pending_commit_hold_ms);
    stats.recovery_dynamic_continuation_patience_ms = Some(timing.continuation_patience_window_ms);
    stats.recovery_dynamic_clean_anchor_patience_ms =
        Some(timing.clean_anchor_commit_patience_window_ms);
}

pub(crate) fn transport_await_patience_window_ms(
    episode_status: &str,
    timing: &RecoveryDynamicTiming,
) -> f64 {
    if episode_status == "decoded" {
        timing.clean_anchor_commit_patience_window_ms
    } else {
        timing.continuation_patience_window_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::rtc::recovery::policy::ScenarioPolicyResolver;

    #[test]
    fn home_lan_10ms_rtt_nack_timeout_within_rfc_bounds() {
        let profile = ScenarioPolicyResolver::resolve_recovery_profile_by_kind(
            ScenarioPolicyProfileKind::HomeLanGaming,
        );
        let t = resolve_recovery_dynamic_timing_with_rtt(10.0, profile);
        assert!((t.nack_timeout_ms - 56.0).abs() < 0.01);
        assert!(t.nack_timeout_ms >= 45.0 && t.nack_timeout_ms <= 90.0);
        assert!((t.nack_retry_interval_ms - 15.5).abs() < 0.01);
        assert!((t.pli_refresh_interval_ms - 40.0).abs() < 0.01);
    }

    #[test]
    fn nack_retry_interval_u64_matches_rfc_formula() {
        assert_eq!(super::nack_retry_interval_u64_from_rtt_ms(10.0), 16);
        assert_eq!(super::nack_retry_interval_u64_from_rtt_ms(100.0), 83);
    }

    #[test]
    fn cloud_200ms_rtt_widens_pli_and_fir() {
        let profile = ScenarioPolicyResolver::resolve_recovery_profile_by_kind(
            ScenarioPolicyProfileKind::CloudGaming,
        );
        let t = resolve_recovery_dynamic_timing_with_rtt(200.0, profile);
        assert!(t.nack_timeout_ms >= 240.0 && t.nack_timeout_ms <= 420.0);
        assert!(t.pli_refresh_interval_ms > 150.0);
        assert!(t.fir_retry_interval_ms > 400.0);
    }
}
