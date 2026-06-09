use super::decode_sync::decoder_waiting_keyframe_control_active_from_stats;
use super::display::displayed_idr_serving_from_stats;
use super::insert::{derive_packet_recovery_action_stage_from_stats, PacketRecoveryActionStage};
use super::snapshot::DerivedDecoderHealth;
use crate::XbxEngineMediaRuntimeStats;

pub(crate) fn is_media_healthy_baseline(
    connected: bool,
    chain_healthy: bool,
    track_state: Option<&str>,
    track_video_bytes_total: Option<u64>,
    decode_age_ms: Option<f64>,
    present_age_ms: Option<f64>,
    decode_fresh_limit_ms: f64,
    present_fresh_limit_ms: f64,
    decoder_stalled: bool,
    renderer_stalled: bool,
) -> bool {
    if !connected || !chain_healthy || decoder_stalled || renderer_stalled {
        return false;
    }
    let track_attached = matches!(track_state, Some("remoteTrackAttached"));
    let has_video_bytes = track_video_bytes_total.is_some_and(|bytes| bytes > 0);
    let decode_fresh = decode_age_ms.is_some_and(|age| age <= decode_fresh_limit_ms);
    let present_fresh = present_age_ms.is_some_and(|age| age <= present_fresh_limit_ms);
    track_attached && has_video_bytes && decode_fresh && present_fresh
}

/// 对外单一恢复表面相位（收敛 transport-await / sustaining / waiting-keyframe 叙事）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RecoverySurfacePhase {
    #[default]
    Steady,
    Repairing,
    AwaitIdr,
    SupplyBreak,
}

impl RecoverySurfacePhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Steady => "steady",
            Self::Repairing => "repairing",
            Self::AwaitIdr => "await-idr",
            Self::SupplyBreak => "supply-break",
        }
    }
}

/// diagnostics / UI / trace 投影；Insert Hold/Emit 由 `ReferenceChainState` 裁决。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PresentationSupplyPhase {
    #[default]
    Priming,
    Steady,
    Repairing,
    SupplyBreak,
}

impl PresentationSupplyPhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Priming => "priming",
            Self::Steady => "steady",
            Self::Repairing => "repairing",
            Self::SupplyBreak => "supply-break",
        }
    }
}

/// 首显后 acquisition 窗：decode/submit 分段延迟阈值（与 Owner degraded 同量级，独立于 picture 叙事）。
pub(crate) const MEDIA_SUPPLY_PRIMING_MAX_DECODE_AGE_MS: f64 = 200.0;
pub(crate) const MEDIA_SUPPLY_PRIMING_MAX_SUBMIT_AGE_MS: f64 = 500.0;
/// 首显后固定 acquisition 窗（与 MEDIA_SUPPLY_GATE 起播 5s 对齐）；窗内恒为 `priming` 超相位。
pub(crate) const MEDIA_SUPPLY_PRIMING_ACQUISITION_WINDOW_MS: f64 = 5_000.0;

pub(crate) fn media_supply_acquisition_window_active_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    stats.host_frame_present_epoch > 0
        && stats
            .media_supply_host_first_present_at_ms
            .is_some_and(|at_ms| {
                (now_ms - at_ms).max(0.0) < MEDIA_SUPPLY_PRIMING_ACQUISITION_WINDOW_MS
            })
}

pub(crate) fn media_supply_decode_age_ms_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> Option<f64> {
    stats
        .latest_video_decode_ok_time_ms
        .map(|ts| (now_ms - ts).max(0.0))
}

/// Priming 完成：首显 + 当拍 decode/submit 均健康。picture `PlaybackRecovered`  alone 不算完成。
pub(crate) fn media_supply_priming_complete_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    if stats.host_frame_present_epoch == 0 {
        return false;
    }
    let decode_ok = media_supply_decode_age_ms_from_stats(stats, now_ms)
        .is_some_and(|age| age <= MEDIA_SUPPLY_PRIMING_MAX_DECODE_AGE_MS);
    let submit_ok = stats
        .submit_age_ms
        .is_some_and(|age| age <= MEDIA_SUPPLY_PRIMING_MAX_SUBMIT_AGE_MS);
    decode_ok && submit_ok
}

/// 起播 acquisition 超相位：无首显 / 首显后 5s 窗 / 分段延迟未健康 → 恒 `priming`，吞 repairing 子态。
pub(crate) fn media_supply_priming_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    if stats.host_frame_present_epoch == 0 {
        return true;
    }
    if media_supply_acquisition_window_active_from_stats(stats, now_ms) {
        return true;
    }
    !media_supply_priming_complete_from_stats(stats, now_ms)
}

pub(crate) fn recovery_effective_rtt_ms_from_stats(stats: &XbxEngineMediaRuntimeStats) -> f64 {
    stats
        .recovery_effective_rtt_ms
        .or(stats.recovery_smoothed_rtt_ms)
        .unwrap_or(80.0)
        .clamp(20.0, 400.0)
}

pub(crate) fn derive_presentation_supply_phase_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> PresentationSupplyPhase {
    if media_supply_submit_starved_from_stats(stats, now_ms) {
        return PresentationSupplyPhase::SupplyBreak;
    }
    if recovery_supply_break_active_from_stats(stats, now_ms) {
        return PresentationSupplyPhase::SupplyBreak;
    }
    if media_supply_priming_from_stats(stats, now_ms) {
        return PresentationSupplyPhase::Priming;
    }
    let timeline_repairing = stats
        .latest_video_timeline_observation
        .as_ref()
        .is_some_and(|timeline| {
            matches!(
                timeline.chain.state.as_str(),
                "repairing" | "waiting-keyframe"
            ) || timeline.gap.is_some()
        });
    let receiver_repairing = stats
        .latest_video_receiver_observation
        .as_ref()
        .is_some_and(|obs| obs.receiver_state == "repairing");
    if timeline_repairing || receiver_repairing {
        return PresentationSupplyPhase::Repairing;
    }
    PresentationSupplyPhase::Steady
}

pub(crate) fn receive_keyframe_surface_await_idr_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    let effective_rtt_ms = recovery_effective_rtt_ms_from_stats(stats);
    let action_stage =
        derive_packet_recovery_action_stage_from_stats(stats, now_ms, effective_rtt_ms);
    stats.receive_keyframe_required.unwrap_or(false)
        || decoder_waiting_keyframe_control_active_from_stats(stats, now_ms)
        || matches!(
            action_stage,
            PacketRecoveryActionStage::WaitKeyframe | PacketRecoveryActionStage::RequestIdr
        )
}

/// 控制面应走 receive PLI / await-idr；这里只读 receive-local `keyframe_required`
/// 与 decoder waiting-keyframe 事实，`ReferenceChainState` 只保留给投影与 mismatch 诊断。
pub(crate) fn idr_recovery_active_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    stats.receive_keyframe_required.unwrap_or(false)
        || decoder_waiting_keyframe_control_active_from_stats(stats, now_ms)
}

/// Session / suspect 门控：只读 receive-local IDR 恢复事实。
pub(crate) fn receive_media_recovery_pressure_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    idr_recovery_active_from_stats(stats, now_ms)
}

/// SessionPhase::Steady 保持：host 可见供给仍可服务，即使 receive media recovery 仍 open。
pub(crate) fn receive_presentation_holds_steady_session_phase_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    if stats.receive_display_state.as_deref() == Some("display-stable") {
        return true;
    }
    let host_presenting =
        stats.host_frame_present_epoch > 0 && stats.latest_video_host_present_time_ms.is_some();
    if !host_presenting {
        return false;
    }
    if stats.recovery_playback_recovered_at_ms.is_some() {
        return true;
    }
    stats.receive_keyframe_required == Some(true)
        && stats
            .latest_video_host_present_time_ms
            .is_some_and(|at| (now_ms - at).max(0.0) < 600.0)
}

pub(crate) fn recovery_surface_phase_from_presentation_supply_phase(
    phase: PresentationSupplyPhase,
) -> RecoverySurfacePhase {
    match phase {
        PresentationSupplyPhase::SupplyBreak => RecoverySurfacePhase::SupplyBreak,
        PresentationSupplyPhase::Repairing => RecoverySurfacePhase::Repairing,
        PresentationSupplyPhase::Priming | PresentationSupplyPhase::Steady => {
            RecoverySurfacePhase::Steady
        }
    }
}

pub(crate) const RECOVERY_SUPPLY_BREAK_SUBMIT_AGE_MS: f64 = 1_500.0;

/// 曾上屏但 Host mailbox submit 停供：presentation supply 诊断，不能反灌 picture recovery。
pub(crate) fn media_supply_submit_starved_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    _now_ms: f64,
) -> bool {
    let had_host_output = stats.host_frame_present_epoch > 0
        || stats.recovery_playback_recovered_at_ms.is_some()
        || displayed_idr_serving_from_stats(stats);
    had_host_output
        && stats
            .submit_age_ms
            .is_some_and(|age| age >= RECOVERY_SUPPLY_BREAK_SUBMIT_AGE_MS)
}

pub(crate) fn recovery_supply_break_active_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    let _ = now_ms;
    let had_present = stats.host_frame_present_epoch > 0
        || stats.recovery_playback_recovered_at_ms.is_some()
        || displayed_idr_serving_from_stats(stats);
    if !had_present {
        return false;
    }
    let submit_stalled = stats
        .submit_age_ms
        .is_some_and(|age| age >= RECOVERY_SUPPLY_BREAK_SUBMIT_AGE_MS);
    let present_starved = stats.video_renderer_stalled.unwrap_or(false)
        && stats.display_age_ms.map(|age| age >= 500.0).unwrap_or(true);
    submit_stalled || present_starved
}

pub(crate) fn derive_recovery_surface_phase_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> RecoverySurfacePhase {
    if receive_keyframe_surface_await_idr_from_stats(stats, now_ms) {
        return RecoverySurfacePhase::AwaitIdr;
    }
    recovery_surface_phase_from_presentation_supply_phase(
        derive_presentation_supply_phase_from_stats(stats, now_ms),
    )
}

pub(crate) fn derive_decoder_health_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> DerivedDecoderHealth {
    if media_supply_submit_starved_from_stats(stats, now_ms)
        || recovery_supply_break_active_from_stats(stats, now_ms)
    {
        return DerivedDecoderHealth::SupplyStalled;
    }
    if decoder_waiting_keyframe_control_active_from_stats(stats, now_ms) {
        DerivedDecoderHealth::AwaitIdr
    } else {
        match stats.video_decoder_recovery_state.as_deref() {
            Some("recovering") => DerivedDecoderHealth::RepairingDecode,
            _ => DerivedDecoderHealth::Nominal,
        }
    }
}

/// owner / suspect_anchor / fast-path 门控：只读 receive-local IDR 恢复事实。
pub(crate) fn derived_decoder_health_indicates_await_idr(
    stats: &XbxEngineMediaRuntimeStats,
) -> bool {
    receive_media_recovery_pressure_from_stats(stats, 0.0)
}
