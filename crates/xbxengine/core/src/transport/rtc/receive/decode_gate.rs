use crate::media::video::h264::inspection::{H264AccessUnitInspection, H264BootstrapRejectReason};
use crate::media::video::types::AssembledVideoFrame;
use crate::transport::rtc::receive::ReceiverState;
use crate::transport::rtc::recovery::contract::{
    decodable_to_feed, DecodableFeedContext, ReferenceChainState,
};

/// pre-decode 对单个 RTP access unit 的裁决结果。
#[derive(Debug)]
pub enum DecodeGateDecision {
    Emit(AssembledVideoFrame),
    Continue,
}

/// RFC `RtcReceiveCore` decode gate：pre-decode AU 可解码性裁决。
#[derive(Debug, Default)]
pub struct DecodeGate;

impl DecodeGate {
    /// 接收主线 decode gate 入口（ingress 侧实现保留在 `RtcVideoFrameSource` 以承载 stats/trace 副作用）。
    pub(crate) async fn evaluate_for_ingress(
        &self,
        source: &mut crate::transport::rtc::receive::RtcVideoFrameSource,
        sample: crate::transport::rtc::receive::RtpAccessUnit,
    ) -> DecodeGateDecision {
        source.evaluate_decode_gate(sample).await
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DecodeCorruptionPolicy {
    /// 修洞/无 hard gap 时允许非 IDR delta（可接受花屏），对齐 libwebrtc Insert。
    #[default]
    StandardWebRtc,
}

/// receiver-local 解码上下文：不读 host present / mailbox。
#[derive(Clone, Copy, Debug)]
pub struct ReceiverDecodeContext {
    pub receiver_state: ReceiverState,
    pub has_active_gap: bool,
    pub nack_exhausted: bool,
    pub first_frame_acquired: bool,
    pub decoder_reference_synced: bool,
}

impl ReceiverDecodeContext {
    pub fn has_repair_progress(self) -> bool {
        matches!(
            self.receiver_state,
            ReceiverState::Receiving | ReceiverState::Repairing
        ) && !self.nack_exhausted
    }

    pub fn hard_gap_blocks_delta(self) -> bool {
        self.nack_exhausted
            && self.has_active_gap
            && !matches!(self.receiver_state, ReceiverState::Repairing)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InspectionAdmission {
    Accept,
    AwaitRecoveryKeyframe,
}

pub fn should_block_non_keyframe_admission(ctx: &ReceiverDecodeContext) -> bool {
    matches!(ctx.receiver_state, ReceiverState::WaitingKeyframe)
        && ctx.nack_exhausted
        && !ctx.has_repair_progress()
}

pub fn inspection_bootstrap_blocks_delta_continuation(
    inspection: &H264AccessUnitInspection,
) -> bool {
    matches!(
        inspection.bootstrap_reject_reason,
        Some(H264BootstrapRejectReason::BootstrapMissingIdr | H264BootstrapRejectReason::NonIdrVcl)
    )
}

pub fn receiver_state_blocks_delta_continuation(ctx: &ReceiverDecodeContext) -> bool {
    matches!(ctx.receiver_state, ReceiverState::WaitingKeyframe)
}

pub(crate) fn decodable_feed_context_from_receiver(
    ctx: &ReceiverDecodeContext,
) -> DecodableFeedContext {
    DecodableFeedContext {
        decoder_reference_synced: ctx.decoder_reference_synced,
        first_frame_acquired: ctx.first_frame_acquired,
        hard_gap_blocks_delta: ctx.hard_gap_blocks_delta(),
    }
}

pub(crate) fn insert_decodable_to_feed(
    inspection: &H264AccessUnitInspection,
    ctx: &ReceiverDecodeContext,
    stage: crate::transport::rtc::recovery::contract::PacketRecoveryActionStage,
    reference_state: ReferenceChainState,
) -> bool {
    decodable_to_feed(
        inspection,
        &decodable_feed_context_from_receiver(ctx),
        stage,
        reference_state,
    )
}

/// decode actor：由 runtime stats 重建 ingress 侧等价的 `ReceiverDecodeContext`。
pub(crate) fn receiver_decode_context_from_stats(
    stats: &crate::XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> ReceiverDecodeContext {
    use crate::transport::rtc::recovery::contract::{
        current_clean_anchor_observed_at_ms_from_stats,
        current_fresh_anchor_recovered_at_ms_from_stats, decoder_reference_synced_from_stats,
        has_current_clean_anchor_from_stats,
    };
    let decoder_reference_synced = decoder_reference_synced_from_stats(stats, now_ms);
    let current_media_anchor_committed = current_clean_anchor_observed_at_ms_from_stats(stats)
        .is_some()
        || current_fresh_anchor_recovered_at_ms_from_stats(stats).is_some();
    let first_frame_acquired = has_current_clean_anchor_from_stats(stats)
        || playback_recovered_current_for_active_recovery_episode(stats)
        || stats
            .latest_video_decode_ok_time_ms
            .is_some_and(|decode_ok_ms| {
                progress_time_current_for_active_recovery_episode(stats, decode_ok_ms)
            });
    let observed_receiver_state = stats
        .latest_video_receiver_observation
        .as_ref()
        .map(|obs| match obs.receiver_state.as_str() {
            "waiting-keyframe" => ReceiverState::WaitingKeyframe,
            "repairing" => ReceiverState::Repairing,
            _ => ReceiverState::Receiving,
        })
        .unwrap_or(ReceiverState::Receiving);
    let has_active_gap = stats
        .latest_video_timeline_observation
        .as_ref()
        .and_then(|timeline| timeline.gap.as_ref())
        .is_some();
    let receiver_state = if matches!(observed_receiver_state, ReceiverState::WaitingKeyframe)
        && first_frame_acquired
        && (current_media_anchor_committed || decoder_reference_synced)
    {
        if has_active_gap {
            ReceiverState::Repairing
        } else {
            ReceiverState::Receiving
        }
    } else {
        observed_receiver_state
    };
    let nack_exhausted = stats
        .latest_video_receiver_observation
        .as_ref()
        .is_some_and(|obs| {
            obs.nack_in_flight
                && has_active_gap
                && !matches!(receiver_state, ReceiverState::Repairing)
        });
    ReceiverDecodeContext {
        receiver_state,
        has_active_gap,
        nack_exhausted,
        first_frame_acquired,
        decoder_reference_synced,
    }
}

fn playback_recovered_current_for_active_recovery_episode(
    stats: &crate::XbxEngineMediaRuntimeStats,
) -> bool {
    stats
        .recovery_playback_recovered_at_ms
        .is_some_and(|recovered_at_ms| {
            progress_time_current_for_active_recovery_episode(stats, recovered_at_ms)
        })
}

fn progress_time_current_for_active_recovery_episode(
    stats: &crate::XbxEngineMediaRuntimeStats,
    progress_ms: f64,
) -> bool {
    if !stats.transport_recovery_episode_active {
        return true;
    }
    stats
        .transport_recovery_episode_opened_at_ms
        .is_none_or(|opened_at_ms| progress_ms >= opened_at_ms)
}

pub fn inspection_bootstrap_reason(inspection: &H264AccessUnitInspection) -> &'static str {
    match inspection.bootstrap_reject_reason {
        Some(H264BootstrapRejectReason::NoVcl) => "inspectionRejectNoVcl",
        Some(H264BootstrapRejectReason::MissingSps) => "bootstrapMissingSps",
        Some(H264BootstrapRejectReason::MissingPps) => "bootstrapMissingPps",
        Some(H264BootstrapRejectReason::BootstrapMissingIdr)
        | Some(H264BootstrapRejectReason::NonIdrVcl) => "bootstrapMissingIdr",
        Some(H264BootstrapRejectReason::MixedIdrWithTrailingDelta) => "mixedIdrWithTrailingDelta",
        Some(H264BootstrapRejectReason::InvalidSliceHeader) => "inspectionRejectInvalidSliceHeader",
        None if !inspection.slice_headers_valid => "inspectionRejectInvalidSliceHeader",
        None => "inspectionRejectUnknown",
    }
}

pub fn keyframe_episode_response_detail(
    inspection: &H264AccessUnitInspection,
    admission: InspectionAdmission,
    ctx: Option<&ReceiverDecodeContext>,
) -> &'static str {
    if !inspection.is_idr {
        if matches!(admission, InspectionAdmission::Accept)
            && inspection.delta_continuation_ready()
            && inspection.committed_sps_present()
            && inspection.committed_pps_present()
        {
            if ctx.is_some_and(|c| matches!(c.receiver_state, ReceiverState::Repairing)) {
                return "repairingContinuation";
            }
            return "receiverLocalContinuation";
        }
        return "bootstrapMissingIdr";
    }
    match admission {
        InspectionAdmission::Accept => "firstKeyframeAccepted",
        InspectionAdmission::AwaitRecoveryKeyframe => inspection_bootstrap_reason(inspection),
    }
}

#[cfg(test)]
mod tests {
    use super::receiver_decode_context_from_stats;
    use crate::XbxEngineMediaRuntimeStats;

    #[test]
    fn receiver_decode_context_ignores_stale_playback_recovered_for_active_episode() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_recovery_episode_active: true,
            transport_recovery_episode_opened_at_ms: Some(2_000.0),
            recovery_playback_recovered_at_ms: Some(1_000.0),
            latest_video_decode_ok_time_ms: Some(1_500.0),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let context = receiver_decode_context_from_stats(&stats, 2_100.0);

        assert!(!context.first_frame_acquired);
    }

    #[test]
    fn receiver_decode_context_accepts_current_episode_playback_recovered() {
        let stats = XbxEngineMediaRuntimeStats {
            transport_recovery_episode_active: true,
            transport_recovery_episode_opened_at_ms: Some(2_000.0),
            recovery_playback_recovered_at_ms: Some(2_050.0),
            ..XbxEngineMediaRuntimeStats::default()
        };

        let context = receiver_decode_context_from_stats(&stats, 2_100.0);

        assert!(context.first_frame_acquired);
    }
}
