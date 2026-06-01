//! 参考链状态：Insert/Decode 准入与 sparse IDR 的主输入。

use super::decode_sync::{
    decoder_reference_synced_from_stats, receiver_nack_exhausted_from_stats,
    CONTINUATION_NO_OUTPUT_REQUEST_IDR_STREAK,
};
use super::insert::{derive_packet_recovery_action_stage_from_stats, PacketRecoveryActionStage};
use super::supply::{media_supply_submit_starved_from_stats, RECOVERY_SUPPLY_BREAK_SUBMIT_AGE_MS};
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

fn had_prior_playback_output(stats: &XbxEngineMediaRuntimeStats) -> bool {
    stats.recovery_decoder_reference_synced_at_ms.is_some()
        || stats.recovery_playback_recovered_at_ms.is_some()
        || stats.recovery_displayed_idr_at_ms.is_some()
}

pub(crate) fn derive_reference_chain_state_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
    effective_rtt_ms: f64,
) -> ReferenceChainObservation {
    let decoder_reference_synced = decoder_reference_synced_from_stats(stats, now_ms);
    let bootstrap_ready = stats
        .latest_h264_inspection_observation
        .as_ref()
        .map(|obs| obs.bootstrap_ready)
        .unwrap_or(false);
    let has_active_gap = stats
        .latest_video_timeline_observation
        .as_ref()
        .and_then(|timeline| timeline.gap.as_ref())
        .is_some();
    let nack_exhausted = receiver_nack_exhausted_from_stats(stats);
    let action_stage =
        derive_packet_recovery_action_stage_from_stats(stats, now_ms, effective_rtt_ms);
    let submit_starved = media_supply_submit_starved_from_stats(stats, now_ms);
    let submit_age_ms = stats.submit_age_ms;
    let decoder_waiting = stats.video_decoder_recovery_state.as_deref() == Some("waiting-keyframe");
    let no_output_streak = stats
        .latest_decode_output_path_observation
        .as_ref()
        .and_then(|obs| obs.backend_no_output_streak)
        .unwrap_or(0);

    let base = || ReferenceChainObservation {
        decoder_reference_synced,
        bootstrap_ready,
        has_active_gap,
        nack_exhausted,
        submit_age_ms,
        ..Default::default()
    };

    if decoder_waiting
        || no_output_streak >= CONTINUATION_NO_OUTPUT_REQUEST_IDR_STREAK
        || matches!(
            action_stage,
            PacketRecoveryActionStage::WaitKeyframe | PacketRecoveryActionStage::RequestIdr
        )
    {
        return ReferenceChainObservation {
            state: ReferenceChainState::NeedKeyframe,
            cause: if decoder_waiting {
                "decoder-waiting-keyframe"
            } else if no_output_streak >= CONTINUATION_NO_OUTPUT_REQUEST_IDR_STREAK {
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

    if submit_starved {
        return ReferenceChainObservation {
            state: ReferenceChainState::NeedKeyframe,
            cause: "supply-submit-starved",
            ..base()
        };
    }

    if !decoder_reference_synced && !bootstrap_ready {
        let had_prior_output = had_prior_playback_output(stats);
        let submit_stalled =
            submit_age_ms.is_some_and(|age| age >= RECOVERY_SUPPLY_BREAK_SUBMIT_AGE_MS);
        if had_prior_output && (nack_exhausted || submit_stalled) {
            return ReferenceChainObservation {
                state: ReferenceChainState::NeedKeyframe,
                cause: if nack_exhausted {
                    "post-reset-gap-exhausted"
                } else {
                    "post-reset-submit-starved"
                },
                ..base()
            };
        }
        if !had_prior_output {
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
