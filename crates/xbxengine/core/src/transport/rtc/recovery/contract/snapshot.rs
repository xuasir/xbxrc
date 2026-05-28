use super::decode_sync::{
    decoder_reference_synced_from_stats, displayed_idr_decoder_synced_from_stats,
};
use super::display::{
    displayed_idr_serving_allows_relaxed_controls_from_stats, displayed_idr_serving_from_stats,
};
use super::exit::{recovery_exit_path_from_stats, RecoveryExitPath, RecoveryExitThresholds};
use super::insert::{derive_packet_recovery_action_stage_from_stats, PacketRecoveryActionStage};
use super::supply::{
    derive_decoder_health_from_stats, derive_media_supply_phase_from_stats,
    idr_recovery_active_from_stats, recovery_effective_rtt_ms_from_stats,
    recovery_supply_break_active_from_stats, recovery_surface_phase_from_media_supply_phase,
    MediaSupplyPhase, RecoverySurfacePhase,
};
use super::transport_await::is_transport_await_unresolved_reason;
use crate::XbxEngineMediaRuntimeStats;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum DerivedDecoderHealth {
    #[default]
    Nominal,
    RepairingDecode,
    AwaitIdr,
    SupplyStalled,
}

impl DerivedDecoderHealth {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Nominal => "nominal",
            Self::RepairingDecode => "repairing-decode",
            Self::AwaitIdr => "await-idr",
            Self::SupplyStalled => "supply-stalled",
        }
    }
}

/// 单 tick 从 stats 派生的合同快照；Owner/policy/coordinator 应优先读此结构，避免重复调用 6+ 个 helper。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct RecoveryContractSnapshot {
    pub(crate) media_supply_phase: MediaSupplyPhase,
    pub(crate) surface_phase: RecoverySurfacePhase,
    pub(crate) derived_health: DerivedDecoderHealth,
    pub(crate) exit_path: RecoveryExitPath,
    pub(crate) serving_wide: bool,
    pub(crate) serving_relaxed: bool,
    pub(crate) supply_break_active: bool,
    pub(crate) decoder_reference_synced: bool,
    pub(crate) displayed_idr_decoder_synced: bool,
    pub(crate) action_stage: PacketRecoveryActionStage,
}

impl RecoveryContractSnapshot {
    pub(crate) fn from_stats(
        stats: &XbxEngineMediaRuntimeStats,
        now_ms: f64,
        exit_thresholds: RecoveryExitThresholds,
    ) -> Self {
        let serving_wide = displayed_idr_serving_from_stats(stats);
        let serving_relaxed =
            displayed_idr_serving_allows_relaxed_controls_from_stats(stats, now_ms);
        let supply_break_active = recovery_supply_break_active_from_stats(stats, now_ms);
        let media_supply_phase = derive_media_supply_phase_from_stats(stats, now_ms);
        let effective_rtt_ms = recovery_effective_rtt_ms_from_stats(stats);
        let decoder_reference_synced = decoder_reference_synced_from_stats(stats, now_ms);
        let displayed_idr_decoder_synced = displayed_idr_decoder_synced_from_stats(stats, now_ms);
        let action_stage =
            derive_packet_recovery_action_stage_from_stats(stats, now_ms, effective_rtt_ms);
        Self {
            media_supply_phase,
            surface_phase: recovery_surface_phase_from_media_supply_phase(media_supply_phase),
            derived_health: derive_decoder_health_from_stats(stats, now_ms),
            exit_path: recovery_exit_path_from_stats(stats, now_ms, exit_thresholds),
            serving_wide,
            serving_relaxed,
            supply_break_active,
            decoder_reference_synced,
            displayed_idr_decoder_synced,
            action_stage,
        }
    }
}

pub(crate) fn sync_derived_recovery_contract_fields(
    stats: &mut XbxEngineMediaRuntimeStats,
    now_ms: f64,
) {
    let media = derive_media_supply_phase_from_stats(stats, now_ms);
    let surface = recovery_surface_phase_from_media_supply_phase(media);
    let health = derive_decoder_health_from_stats(stats, now_ms);
    stats.media_supply_phase = Some(media.as_str().to_string());
    stats.recovery_surface_phase = Some(surface.as_str().to_string());
    stats.derived_decoder_health = Some(health.as_str().to_string());
    stats.recovery_transport_await_unresolved = Some(
        stats
            .recovery_diagnosis
            .as_deref()
            .is_some_and(is_transport_await_unresolved_reason)
            && surface != RecoverySurfacePhase::Steady
            && !recovery_supply_break_active_from_stats(stats, now_ms),
    );
    if idr_recovery_active_from_stats(stats, now_ms) {
        stats.session_phase = Some("active-recovery".to_string());
        if stats.recovery_diagnosis.is_none() {
            stats.recovery_diagnosis = Some("waitKeyframe".to_string());
        }
    }
}
