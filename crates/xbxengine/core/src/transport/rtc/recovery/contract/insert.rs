use super::decode_sync::{
    receiver_nack_exhausted_from_stats, CONTINUATION_NO_OUTPUT_REQUEST_IDR_STREAK,
};
use super::gap::GAP_KEYFRAME_ONLY_MAX_AGE_MS;
use super::reference_chain::ReferenceChainState;
use super::transport_await::is_recovery_delta_continuation_ready;
use crate::media::video::h264::inspection::H264AccessUnitInspection;
use crate::XbxEngineMediaRuntimeStats;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum PacketRecoveryActionStage {
    Steady,
    NackPending,
    NackMissed,
    WaitKeyframe,
    RequestIdr,
}

impl Default for PacketRecoveryActionStage {
    fn default() -> Self {
        Self::Steady
    }
}

impl PacketRecoveryActionStage {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Steady => "steady",
            Self::NackPending => "nack_pending",
            Self::NackMissed => "nack_missed",
            Self::WaitKeyframe => "wait_keyframe",
            Self::RequestIdr => "request_idr",
        }
    }
}

/// Insert/decode 共用的可投喂谓词输入（避免 contract ↔ receive 循环依赖）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct DecodableFeedContext {
    pub decoder_reference_synced: bool,
    pub first_frame_acquired: bool,
    pub hard_gap_blocks_delta: bool,
    pub receiver_repairing: bool,
    pub has_active_gap: bool,
}

pub(crate) fn derive_packet_recovery_action_stage_from_stats(
    stats: &XbxEngineMediaRuntimeStats,
    now_ms: f64,
    effective_rtt_ms: f64,
) -> PacketRecoveryActionStage {
    let decoder_waiting = stats.video_decoder_recovery_state.as_deref() == Some("waiting-keyframe");
    let no_output_streak = stats
        .latest_decode_output_path_observation
        .as_ref()
        .and_then(|obs| obs.backend_no_output_streak)
        .unwrap_or(0);
    if decoder_waiting || no_output_streak >= CONTINUATION_NO_OUTPUT_REQUEST_IDR_STREAK {
        return PacketRecoveryActionStage::RequestIdr;
    }
    let gap_age_ms = stats
        .latest_video_timeline_observation
        .as_ref()
        .and_then(|timeline| timeline.gap.as_ref())
        .map(|gap| (now_ms - gap.observed_at_ms).max(0.0));
    let gap_open = gap_age_ms.is_some();
    let gap_stale = gap_age_ms
        .is_some_and(|age| age >= GAP_KEYFRAME_ONLY_MAX_AGE_MS.max(effective_rtt_ms * 2.0));
    if gap_stale {
        return PacketRecoveryActionStage::WaitKeyframe;
    }
    if gap_open {
        if stats
            .latest_video_receiver_observation
            .as_ref()
            .is_some_and(|obs| obs.nack_in_flight)
        {
            return PacketRecoveryActionStage::NackPending;
        }
        if receiver_nack_exhausted_from_stats(stats) {
            return PacketRecoveryActionStage::WaitKeyframe;
        }
        return PacketRecoveryActionStage::NackMissed;
    }
    PacketRecoveryActionStage::Steady
}

/// 是否允许将本 AU 送入解码器（与 libwebrtc reference-complete 纪律对齐）。
pub(crate) fn decodable_to_feed(
    inspection: &H264AccessUnitInspection,
    ctx: &DecodableFeedContext,
    stage: PacketRecoveryActionStage,
    reference_state: ReferenceChainState,
) -> bool {
    if !inspection.slice_headers_valid {
        return false;
    }
    if inspection.bootstrap_ready {
        return true;
    }
    if !is_recovery_delta_continuation_ready(inspection) {
        return false;
    }
    if ctx.hard_gap_blocks_delta {
        return false;
    }
    if !ctx.first_frame_acquired && !ctx.decoder_reference_synced {
        return false;
    }
    if !ctx.decoder_reference_synced {
        return false;
    }
    if matches!(reference_state, ReferenceChainState::NeedKeyframe)
        && !inspection.is_idr
        && !inspection.bootstrap_ready
    {
        return false;
    }
    match stage {
        PacketRecoveryActionStage::Steady | PacketRecoveryActionStage::NackPending => true,
        PacketRecoveryActionStage::NackMissed => ctx.receiver_repairing || !ctx.has_active_gap,
        PacketRecoveryActionStage::WaitKeyframe | PacketRecoveryActionStage::RequestIdr => false,
    }
}
