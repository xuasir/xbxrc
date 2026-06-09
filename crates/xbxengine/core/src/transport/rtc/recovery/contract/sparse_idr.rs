use super::decode_sync::decoder_reference_synced_from_stats;
#[cfg(test)]
use super::insert::derive_packet_recovery_action_stage_from_stats;
use super::insert::PacketRecoveryActionStage;
use super::reference_chain::ReferenceChainState;
#[cfg(test)]
use super::supply::idr_recovery_active_from_stats;
use super::supply::{
    derive_recovery_surface_phase_from_stats, recovery_effective_rtt_ms_from_stats,
    RecoverySurfacePhase,
};
use crate::XbxEngineMediaRuntimeStats;

pub(crate) fn sparse_idr_pli_interval_ms(effective_rtt_ms: f64) -> f64 {
    (effective_rtt_ms.clamp(20.0, 400.0) * 0.5)
        .max(12.0)
        .min(40.0)
}

#[cfg(test)]
pub(crate) fn sparse_idr_pressure_active_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    sparse_idr_rhythm_from_stats(stats, now_ms).active
}

/// receive 主路径的 sparse pressure：只读 receive ledger 同步字段与 decoder sync 事实。
pub(crate) fn receive_ledger_sparse_pressure_active_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    if decoder_reference_synced_from_stats(stats, now_ms) {
        return false;
    }
    stats.receive_keyframe_required.unwrap_or(false)
        || stats.receive_keyframe_sent_count_unresolved > 0
        || matches!(
            stats.receive_keyframe_response_state.as_deref(),
            Some("non-idr-only" | "idr-unusable")
        )
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
    action_stage: PacketRecoveryActionStage,
) -> SparseIdrRhythm {
    let active = if decoder_reference_synced_from_stats(stats, now_ms) {
        false
    } else {
        ledger.keyframe_required || ledger.sparse_active()
    };
    let pli_interval_ms = sparse_idr_pli_interval_ms(effective_rtt_ms);
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

#[cfg(test)]
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

/// legacy sparse/must-idr 诊断字段：比较 receive reference 与 media recovery surface。
pub(crate) fn sparse_must_idr_projection_mismatch_from_reference(
    reference_state: ReferenceChainState,
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> bool {
    let await_idr_surface = matches!(
        derive_recovery_surface_phase_from_stats(stats, now_ms),
        RecoverySurfacePhase::AwaitIdr
    );
    let need_keyframe_ref = matches!(reference_state, ReferenceChainState::NeedKeyframe);
    need_keyframe_ref != await_idr_surface
}

const RECOVERY_KEYFRAME_RETRY_INTERVAL_MS: f64 = 450.0;

pub(crate) fn recovery_keyframe_retry_interval_ms_for_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> f64 {
    if receive_ledger_sparse_pressure_active_from_stats(stats, now_ms) {
        let rtt = recovery_effective_rtt_ms_from_stats(stats);
        return (2.0 * rtt)
            .max(120.0)
            .min(RECOVERY_KEYFRAME_RETRY_INTERVAL_MS);
    }
    RECOVERY_KEYFRAME_RETRY_INTERVAL_MS
}
