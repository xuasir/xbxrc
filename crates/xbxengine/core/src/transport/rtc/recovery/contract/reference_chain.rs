//! 参考链状态：Insert/Decode 准入与 sparse IDR 的主输入。

use super::decode_sync::{
    decoder_no_output_request_idr_control_active_from_stats, decoder_reference_synced_from_stats,
    decoder_waiting_keyframe_control_active_from_stats, receiver_nack_exhausted_from_stats,
};
use super::display::{
    current_displayed_idr_at_ms_from_stats, current_playback_recovered_at_ms_from_stats,
};
use super::insert::{derive_packet_recovery_action_stage_from_stats, PacketRecoveryActionStage};
use crate::XbxEngineMediaRuntimeStats;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ReferenceChainState {
    #[default]
    Unknown,
    Continuous,
    Repairing,
    NeedKeyframe,
}

impl ReferenceChainState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Continuous => "continuous",
            Self::Repairing => "repairing",
            Self::NeedKeyframe => "need-keyframe",
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ReferenceChainObservation {
    pub state: ReferenceChainState,
    pub cause: &'static str,
    pub decoder_reference_synced: bool,
    pub bootstrap_ready: bool,
    pub has_active_gap: bool,
    pub nack_exhausted: bool,
    pub submit_age_ms: Option<f64>,
}

fn had_current_playback_output(stats: &XbxEngineMediaRuntimeStats, now_ms: f64) -> bool {
    decoder_reference_synced_from_stats(stats, now_ms)
        || current_playback_recovered_at_ms_from_stats(stats).is_some()
        || current_displayed_idr_at_ms_from_stats(stats).is_some()
}

pub(crate) fn derive_reference_chain_state_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
    effective_rtt_ms: f64,
) -> ReferenceChainObservation {
    let facts = reference_chain_diagnostic_facts_from_stats(stats, now_ms);
    let decoder_reference_synced = facts.decoder_reference_synced;
    let bootstrap_ready = facts.bootstrap_ready;
    let has_active_gap = facts.has_active_gap;
    let nack_exhausted = facts.nack_exhausted;
    let action_stage =
        derive_packet_recovery_action_stage_from_stats(stats, now_ms, effective_rtt_ms);
    let submit_age_ms = facts.submit_age_ms;
    let decoder_waiting = decoder_waiting_keyframe_control_active_from_stats(stats, now_ms);
    let decoder_no_output = decoder_no_output_request_idr_control_active_from_stats(stats, now_ms);

    let base = || ReferenceChainObservation {
        decoder_reference_synced,
        bootstrap_ready,
        has_active_gap,
        nack_exhausted,
        submit_age_ms,
        ..Default::default()
    };

    if decoder_waiting
        || decoder_no_output
        || matches!(
            action_stage,
            PacketRecoveryActionStage::WaitKeyframe | PacketRecoveryActionStage::RequestIdr
        )
    {
        return ReferenceChainObservation {
            state: ReferenceChainState::NeedKeyframe,
            cause: if decoder_waiting {
                "decoder-waiting-keyframe"
            } else if decoder_no_output {
                "decoder-no-output-streak"
            } else {
                "action-stage-wait-idr"
            },
            ..base()
        };
    }

    if has_active_gap {
        return ReferenceChainObservation {
            state: if nack_exhausted {
                ReferenceChainState::NeedKeyframe
            } else {
                ReferenceChainState::Repairing
            },
            cause: if nack_exhausted {
                "gap-nack-exhausted"
            } else {
                "gap-nack-pending"
            },
            ..base()
        };
    }

    if !decoder_reference_synced && !bootstrap_ready {
        let had_current_output = had_current_playback_output(stats, now_ms);
        if had_current_output && nack_exhausted {
            return ReferenceChainObservation {
                state: ReferenceChainState::NeedKeyframe,
                cause: "post-reset-gap-exhausted",
                ..base()
            };
        }
        if !had_current_output {
            return ReferenceChainObservation {
                state: ReferenceChainState::Unknown,
                cause: "bootstrap-missing-priming",
                ..base()
            };
        }
    }

    if !decoder_reference_synced {
        return ReferenceChainObservation {
            state: ReferenceChainState::NeedKeyframe,
            cause: if bootstrap_ready {
                "reference-not-synced"
            } else {
                "bootstrap-missing-priming"
            },
            ..base()
        };
    }

    ReferenceChainObservation {
        state: ReferenceChainState::Continuous,
        cause: "reference-continuous",
        ..base()
    }
}

/// 供 receive ledger 投影补充 trace/debug 字段；不承载 reference chain 状态裁决。
pub(crate) fn reference_chain_diagnostic_facts_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> ReferenceChainObservation {
    ReferenceChainObservation {
        decoder_reference_synced: decoder_reference_synced_from_stats(stats, now_ms),
        bootstrap_ready: stats
            .latest_h264_inspection_observation
            .as_ref()
            .map(|obs| obs.bootstrap_ready)
            .unwrap_or(false),
        has_active_gap: stats
            .latest_video_timeline_observation
            .as_ref()
            .and_then(|timeline| timeline.gap.as_ref())
            .is_some(),
        nack_exhausted: receiver_nack_exhausted_from_stats(stats),
        submit_age_ms: stats.submit_age_ms,
        ..Default::default()
    }
}
