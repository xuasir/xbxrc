use super::exit::{recovery_exit_path_from_stats, RecoveryExitPath, RecoveryExitThresholds};
use super::supply::recovery_supply_break_active_from_stats;
use super::transport_await::{
    current_clean_anchor_observed_at_ms, transport_await_has_hard_bootstrap_evidence_from_stats,
};
use crate::XbxEngineMediaRuntimeStats;

pub(crate) struct RecoveryDisplayFacts {
    pub displayed_idr_rtp: Option<u32>,
    pub displayed_idr_at_ms: Option<f64>,
    pub playback_recovered_at_ms: Option<f64>,
    pub fresh_anchor_recovered_at_ms: Option<f64>,
}

impl RecoveryDisplayFacts {
    pub(crate) fn from_stats(stats: &XbxEngineMediaRuntimeStats) -> Self {
        Self {
            displayed_idr_rtp: stats.recovery_displayed_idr_rtp,
            displayed_idr_at_ms: stats.recovery_displayed_idr_at_ms,
            playback_recovered_at_ms: stats.recovery_playback_recovered_at_ms,
            fresh_anchor_recovered_at_ms: stats.recovery_fresh_anchor_recovered_at_ms,
        }
    }

    pub(crate) fn has_established_displayed_idr(self) -> bool {
        self.displayed_idr_at_ms.is_some()
    }
}

pub(crate) fn current_clean_anchor_observed_at_ms_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
) -> Option<f64> {
    current_clean_anchor_observed_at_ms(
        stats.video_anchor_clean_epoch,
        stats.video_anchor_clean_observed_at_ms,
        stats.video_anchor_clean_source_event.as_deref(),
        stats.transport_recovery_epoch,
    )
}

pub(crate) fn has_current_clean_anchor_from_stats(stats: &XbxEngineMediaRuntimeStats) -> bool {
    let display = RecoveryDisplayFacts::from_stats(stats);
    display.fresh_anchor_recovered_at_ms.is_some() || display.has_established_displayed_idr()
}

/// latest-only mailbox 上屏帧常已是 IDR 之后的 delta；pending IDR + host 已 present 即视为 serving。
pub(crate) fn displayed_idr_serving_from_stats(stats: &XbxEngineMediaRuntimeStats) -> bool {
    stats.recovery_displayed_idr_at_ms.is_some()
        || (stats.recovery_pending_displayed_idr_rtp.is_some()
            && stats.host_frame_present_epoch > 0)
}

/// host present 提交 displayed-idr 事实时优先用 decode 侧 pending IDR，而非当前 displayed delta RTP。
pub(crate) fn resolve_host_display_idr_anchor_rtp(
    stats: &XbxEngineMediaRuntimeStats,
    last_displayed_rtp: Option<u32>,
) -> Option<u32> {
    stats
        .recovery_pending_displayed_idr_rtp
        .or(last_displayed_rtp)
}

const DISPLAYED_IDR_SERVING_DECODER_BOOTSTRAP_FRESH_MS: f64 = 1_500.0;
pub(crate) const DISPLAYED_IDR_SERVING_STALE_SUBMIT_BREAK_MS: f64 = 1_000.0;

/// steady continuation 的 codec 元数据拒因；displayed-idr 已 serving 时不应切断窄路径放松。
pub(crate) fn is_soft_missing_idr_bootstrap_reject_reason(reason: Option<&str>) -> bool {
    matches!(reason, Some("bootstrapMissingIdr" | "NonIdrVcl"))
}

/// TimedFallback：submit 已停滞但 TWCC 健康，允许 displayed-idr 续播窄路径（不等新 IDR AU）。
pub(crate) fn recovery_timed_fallback_active_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    recovery_exit_path_from_stats(stats, now_ms, RecoveryExitThresholds::default())
        == RecoveryExitPath::TimedFallback
}

/// decoder 要 IDR / bootstrap 硬拒 / submit 管线停滞时，禁用 P1 放松（collapse、强制 Submit、抑制 recovery-wait）。
pub(crate) fn displayed_idr_serving_relaxation_blocked_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    if transport_await_has_hard_bootstrap_evidence_from_stats(stats, now_ms) {
        return true;
    }
    if decoder_bootstrap_blocks_displayed_idr_relaxation(stats, now_ms) {
        return true;
    }
    // 已有 clean anchor 时不得因 decode FSM 焊死 waiting-keyframe 而切断 Insert 续播窄路径。
    if has_current_clean_anchor_from_stats(stats) && displayed_idr_serving_from_stats(stats) {
        return false;
    }
    if stats.video_decoder_recovery_state.as_deref() == Some("waiting-keyframe") {
        if recovery_timed_fallback_active_from_stats(stats, now_ms)
            && displayed_idr_serving_from_stats(stats)
        {
            return false;
        }
        if recovery_supply_break_active_from_stats(stats, now_ms) {
            return false;
        }
        return true;
    }
    stale_submit_pipeline_breaks_displayed_idr_relaxation(stats, now_ms)
}

/// displayed IDR 已上屏且允许 P1 放松控制（短脉冲抑制，不含供给断裂长尾）。
pub(crate) fn displayed_idr_serving_allows_relaxed_controls_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    displayed_idr_serving_from_stats(stats)
        && !displayed_idr_serving_relaxation_blocked_from_stats(stats, now_ms)
}

fn decoder_bootstrap_blocks_displayed_idr_relaxation(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    let Some(observation) = stats
        .latest_video_decoder_bootstrap_gate_observation
        .as_ref()
        .filter(|observation| {
            (now_ms - observation.observed_at_ms).max(0.0)
                <= DISPLAYED_IDR_SERVING_DECODER_BOOTSTRAP_FRESH_MS
        })
    else {
        return false;
    };
    if !observation.bootstrap_ready
        && is_soft_missing_idr_bootstrap_reject_reason(
            observation.bootstrap_reject_reason.as_deref(),
        )
        && displayed_idr_serving_from_stats(stats)
    {
        return false;
    }
    !observation.bootstrap_ready
        && is_soft_missing_idr_bootstrap_reject_reason(
            observation.bootstrap_reject_reason.as_deref(),
        )
}

fn stale_submit_pipeline_breaks_displayed_idr_relaxation(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    if recovery_timed_fallback_active_from_stats(stats, now_ms) {
        return false;
    }
    if recovery_supply_break_active_from_stats(stats, now_ms) {
        return false;
    }
    stats
        .submit_age_ms
        .is_some_and(|age_ms| age_ms >= DISPLAYED_IDR_SERVING_STALE_SUBMIT_BREAK_MS)
        && (stats.video_renderer_stalled.unwrap_or(false)
            || stats.video_decoder_stalled.unwrap_or(false))
}

/// displayed IDR 已上屏且仍在 gap repair：不把 receiver 投影成 waiting-keyframe，避免 supply 短脉冲。
pub(crate) fn should_collapse_receiver_waiting_keyframe_to_repairing(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
    has_active_gap: bool,
    assembled_frame_count: u64,
) -> bool {
    displayed_idr_serving_allows_relaxed_controls_from_stats(stats, now_ms)
        && has_active_gap
        && assembled_frame_count > 0
}
