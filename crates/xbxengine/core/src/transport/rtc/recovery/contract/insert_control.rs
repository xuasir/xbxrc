//! Insert 控制面输入：仅 ledger / reference / 观测时序；不读诊断投影。

use super::gap::{
    parameter_sets_change_strict_window_ms, GapVsKeyframeMode, GAP_KEYFRAME_ONLY_MAX_AGE_MS,
};
use super::insert::PacketRecoveryActionStage;
use super::reference_chain::ReferenceChainObservation;

const FRESH_H264_IDR_ADMISSION_MS: f64 = 3_000.0;

/// Insert 裁决所需的观测时序（decode/ingress 写入 stats，非诊断投影派生）。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct InsertControlTiming {
    pub fresh_idr_inspection_accepted_at_ms: Option<f64>,
    pub parameter_sets_changed_at_ms: Option<f64>,
    pub gap_age_ms: Option<f64>,
    pub receive_display_stable: bool,
    pub decoder_waiting_keyframe: bool,
}

pub(crate) fn fresh_idr_admission_from_control(timing: &InsertControlTiming, now_ms: f64) -> bool {
    timing
        .fresh_idr_inspection_accepted_at_ms
        .is_some_and(|at| (now_ms - at).max(0.0) <= FRESH_H264_IDR_ADMISSION_MS)
}

pub(crate) fn parameter_sets_change_strict_from_control(
    timing: &InsertControlTiming,
    reference: ReferenceChainObservation,
    fresh_idr_admission: bool,
    now_ms: f64,
    effective_rtt_ms: f64,
) -> bool {
    if fresh_idr_admission {
        return false;
    }
    if !timing.receive_display_stable {
        return false;
    }
    if reference.decoder_reference_synced {
        return false;
    }
    let window_ms = parameter_sets_change_strict_window_ms(effective_rtt_ms);
    timing
        .parameter_sets_changed_at_ms
        .is_some_and(|at_ms| (now_ms - at_ms).max(0.0) < window_ms)
}

pub(crate) fn supply_break_continuation_from_control(
    reference: ReferenceChainObservation,
    action_stage: PacketRecoveryActionStage,
) -> bool {
    matches!(
        reference.state,
        super::reference_chain::ReferenceChainState::Continuous
            | super::reference_chain::ReferenceChainState::Repairing
    ) && matches!(
        action_stage,
        PacketRecoveryActionStage::Steady
            | PacketRecoveryActionStage::NackPending
            | PacketRecoveryActionStage::NackMissed
    )
}

const GAP_ABANDON_KEYFRAME_ONLY_MS: f64 = 5_000.0;

pub(crate) fn resolve_gap_mode_from_control(
    reference: ReferenceChainObservation,
    action_stage: PacketRecoveryActionStage,
    timing: &InsertControlTiming,
    post_ps_strict: bool,
    _now_ms: f64,
    effective_rtt_ms: f64,
) -> GapVsKeyframeMode {
    let gap_stale = timing
        .gap_age_ms
        .is_some_and(|age| age >= GAP_KEYFRAME_ONLY_MAX_AGE_MS.max(effective_rtt_ms * 2.0));
    let idr_pressure = post_ps_strict
        || (reference.state == super::reference_chain::ReferenceChainState::NeedKeyframe
            && !reference.bootstrap_ready);
    if timing.decoder_waiting_keyframe
        || gap_stale
        || idr_pressure
        || action_stage >= PacketRecoveryActionStage::WaitKeyframe
    {
        if timing
            .gap_age_ms
            .is_some_and(|age| age >= GAP_ABANDON_KEYFRAME_ONLY_MS)
        {
            return GapVsKeyframeMode::AbandonGap;
        }
        return GapVsKeyframeMode::KeyframeOnly;
    }
    GapVsKeyframeMode::RepairFirst
}
