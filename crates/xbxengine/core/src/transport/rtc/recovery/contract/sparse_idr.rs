use super::decode_sync::decoder_reference_synced_from_stats;
use super::insert::{derive_packet_recovery_action_stage_from_stats, PacketRecoveryActionStage};
use super::reference_chain::{derive_reference_chain_state_from_stats, ReferenceChainState};
use super::supply::{
    derive_media_supply_phase_from_stats, idr_recovery_active_from_stats,
    recovery_effective_rtt_ms_from_stats, MediaSupplyPhase,
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

/// receive-only 稀疏 IDR：由 receive ledger / reference chain 驱动，与 media supply 投影解耦。
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

pub(crate) fn sparse_idr_rhythm_from_recovery_ledger(
    ledger: &crate::transport::rtc::receive::recovery_ledger::ReceiveRecoveryLedger,
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
    effective_rtt_ms: f64,
) -> SparseIdrRhythm {
    let active = if decoder_reference_synced_from_stats(stats, now_ms) {
        false
    } else {
        ledger.keyframe_required || ledger.sparse_active()
    };
    let pli_interval_ms = sparse_idr_pli_interval_ms(effective_rtt_ms);
    let action_stage =
        derive_packet_recovery_action_stage_from_stats(stats, now_ms, effective_rtt_ms);
    let last_sent = ledger
        .last_keyframe_request_sent_at_ms
        .or(stats.receive_keyframe_last_sent_at_ms);
    let pli_due = active && last_sent.map_or(true, |sent_at| now_ms - sent_at >= pli_interval_ms);
    SparseIdrRhythm {
        active,
        pli_due,
        action_stage,
        pli_interval_ms,
    }
}

pub(crate) fn sparse_idr_rhythm_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> SparseIdrRhythm {
    let effective_rtt_ms = recovery_effective_rtt_ms_from_stats(stats);
    let active = !decoder_reference_synced_from_stats(stats, now_ms)
        && idr_recovery_active_from_stats(stats, now_ms);
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

/// sparse active 与 MustIdr 投影不一致时由 receive 侧记入 stats。
pub(crate) fn sparse_must_idr_projection_mismatch(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    let effective_rtt_ms = recovery_effective_rtt_ms_from_stats(stats);
    let reference = derive_reference_chain_state_from_stats(stats, now_ms, effective_rtt_ms);
    let must_idr_supply = matches!(
        derive_media_supply_phase_from_stats(stats, now_ms),
        MediaSupplyPhase::MustIdr
    );
    let need_keyframe_ref = matches!(reference.state, ReferenceChainState::NeedKeyframe);
    need_keyframe_ref != must_idr_supply
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
