use super::decode_sync::decoder_reference_synced_from_stats;
use super::insert::{derive_packet_recovery_action_stage_from_stats, PacketRecoveryActionStage};
use super::supply::{
    derive_media_supply_phase_from_stats, recovery_effective_rtt_ms_from_stats, MediaSupplyPhase,
};
use crate::XbxEngineMediaRuntimeStats;

pub(crate) fn sparse_idr_pli_interval_ms(effective_rtt_ms: f64) -> f64 {
    (effective_rtt_ms.clamp(20.0, 400.0) * 0.5)
        .max(12.0)
        .min(40.0)
}

pub(crate) fn sparse_idr_pressure_active_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    sparse_idr_rhythm_from_stats(stats, now_ms).active
}

/// MustIdr 稀疏 IDR：receive-only PLI 节奏，与 session keyframe budget 解耦。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SparseIdrRhythm {
    pub(crate) active: bool,
    pub(crate) pli_due: bool,
    pub(crate) action_stage: PacketRecoveryActionStage,
    pub(crate) pli_interval_ms: f64,
}

impl SparseIdrRhythm {
    pub(crate) fn nack_escalation_immediate_eligible(self) -> bool {
        self.active
            && self.pli_due
            && matches!(
                self.action_stage,
                PacketRecoveryActionStage::WaitKeyframe | PacketRecoveryActionStage::RequestIdr
            )
    }
}

pub(crate) fn sparse_idr_rhythm_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> SparseIdrRhythm {
    let active = if decoder_reference_synced_from_stats(stats, now_ms) {
        false
    } else {
        matches!(
            derive_media_supply_phase_from_stats(stats, now_ms),
            MediaSupplyPhase::MustIdr
        )
    };
    let effective_rtt_ms = recovery_effective_rtt_ms_from_stats(stats);
    let pli_interval_ms = sparse_idr_pli_interval_ms(effective_rtt_ms);
    let action_stage =
        derive_packet_recovery_action_stage_from_stats(stats, now_ms, effective_rtt_ms);
    let pli_due = active
        && stats
            .receive_keyframe_last_sent_at_ms
            .map_or(true, |sent_at| now_ms - sent_at >= pli_interval_ms);
    SparseIdrRhythm {
        active,
        pli_due,
        action_stage,
        pli_interval_ms,
    }
}

const RECOVERY_KEYFRAME_RETRY_INTERVAL_MS: f64 = 450.0;

pub(crate) fn recovery_keyframe_retry_interval_ms_for_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> f64 {
    if sparse_idr_pressure_active_from_stats(stats, now_ms) {
        let rtt = recovery_effective_rtt_ms_from_stats(stats);
        return (2.0 * rtt)
            .max(120.0)
            .min(RECOVERY_KEYFRAME_RETRY_INTERVAL_MS);
    }
    RECOVERY_KEYFRAME_RETRY_INTERVAL_MS
}
