use crate::media::video::h264::inspection::{H264AccessUnitInspection, H264BootstrapRejectReason};
use crate::media::video::types::AssembledVideoFrame;
use crate::transport::rtc::receive::ReceiverState;
use crate::transport::rtc::recovery::contract::{
    decodable_to_feed, decoder_reference_synced_from_stats,
    displayed_idr_decoder_synced_from_stats, is_recovery_delta_continuation_ready,
    DecodableFeedContext,
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
    /// decode/displayed-idr 等输出事实已建立（比 first_frame_acquired 更严，不含仅 assembled）。
    pub prior_output_established: bool,
    /// 宿主已显示过 IDR（宽）：控制面放松；Insert 续播须配合 `displayed_idr_decoder_synced`。
    pub displayed_idr_serving: bool,
    /// displayed-idr 且解码器参考链已同步（窄，对齐 WebRTC decoder state）。
    pub displayed_idr_decoder_synced: bool,
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

pub fn steady_displayed_idr_delta_admits(
    inspection: &H264AccessUnitInspection,
    ctx: &ReceiverDecodeContext,
) -> bool {
    !inspection.is_idr
        && ctx.displayed_idr_decoder_synced
        && is_recovery_delta_continuation_ready(inspection)
}

pub(crate) fn decodable_feed_context_from_receiver(
    ctx: &ReceiverDecodeContext,
) -> DecodableFeedContext {
    DecodableFeedContext {
        decoder_reference_synced: ctx.decoder_reference_synced,
        displayed_idr_host_hint: ctx.displayed_idr_serving,
        first_frame_acquired: ctx.first_frame_acquired,
        hard_gap_blocks_delta: ctx.hard_gap_blocks_delta(),
        prior_output_established: ctx.prior_output_established,
        receiver_repairing: matches!(ctx.receiver_state, ReceiverState::Repairing),
        has_active_gap: ctx.has_active_gap,
    }
}

pub(crate) fn insert_decodable_to_feed(
    inspection: &H264AccessUnitInspection,
    ctx: &ReceiverDecodeContext,
    stage: crate::transport::rtc::recovery::contract::PacketRecoveryActionStage,
) -> bool {
    decodable_to_feed(
        inspection,
        &decodable_feed_context_from_receiver(ctx),
        stage,
    )
}

/// 单元测试专用；生产 Insert 单轨走 `resolve_insert_decision`。
#[cfg(test)]
pub fn resolve_inspection_admission(
    inspection: &H264AccessUnitInspection,
    ctx: &ReceiverDecodeContext,
    _policy: DecodeCorruptionPolicy,
) -> InspectionAdmission {
    if !inspection.slice_headers_valid {
        return InspectionAdmission::AwaitRecoveryKeyframe;
    }

    if steady_displayed_idr_delta_admits(inspection, ctx) {
        return InspectionAdmission::Accept;
    }

    if inspection.bootstrap_ready {
        return InspectionAdmission::Accept;
    }

    if !ctx.first_frame_acquired {
        return InspectionAdmission::AwaitRecoveryKeyframe;
    }

    if receiver_state_blocks_delta_continuation(ctx)
        || inspection_bootstrap_blocks_delta_continuation(inspection)
    {
        if ctx.prior_output_established
            && ctx.decoder_reference_synced
            && !ctx.hard_gap_blocks_delta()
            && is_recovery_delta_continuation_ready(inspection)
            && inspection.committed_sps_present()
            && inspection.committed_pps_present()
        {
            return InspectionAdmission::Accept;
        }
        return InspectionAdmission::AwaitRecoveryKeyframe;
    }

    if is_recovery_delta_continuation_ready(inspection)
        && inspection.committed_sps_present()
        && inspection.committed_pps_present()
    {
        if ctx.hard_gap_blocks_delta() || !ctx.decoder_reference_synced {
            return InspectionAdmission::AwaitRecoveryKeyframe;
        }
        return InspectionAdmission::Accept;
    }

    InspectionAdmission::AwaitRecoveryKeyframe
}

/// decode actor：由 runtime stats 重建 ingress 侧等价的 `ReceiverDecodeContext`。
pub(crate) fn receiver_decode_context_from_stats(
    stats: &crate::XbxEngineMediaRuntimeStats,
    now_ms: f64,
) -> ReceiverDecodeContext {
    use crate::transport::rtc::recovery::contract::{
        decoder_reference_synced_from_stats, displayed_idr_decoder_synced_from_stats,
        displayed_idr_serving_from_stats, has_current_clean_anchor_from_stats,
    };
    let receiver_state = stats
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
    let nack_exhausted = stats
        .latest_video_receiver_observation
        .as_ref()
        .is_some_and(|obs| {
            obs.nack_in_flight
                && has_active_gap
                && !matches!(receiver_state, ReceiverState::Repairing)
        });
    let first_frame_acquired = has_current_clean_anchor_from_stats(stats)
        || stats.recovery_playback_recovered_at_ms.is_some()
        || stats.latest_video_decode_ok_time_ms.is_some();
    let displayed_idr_serving = displayed_idr_serving_from_stats(stats);
    let displayed_idr_decoder_synced = displayed_idr_decoder_synced_from_stats(stats, now_ms);
    let decoder_reference_synced = decoder_reference_synced_from_stats(stats, now_ms);
    let prior_output_established = has_current_clean_anchor_from_stats(stats)
        || displayed_idr_serving
        || stats
            .latest_video_decode_ok_time_ms
            .is_some_and(|decode_ok_ms| (now_ms - decode_ok_ms).max(0.0) <= 2_000.0);
    ReceiverDecodeContext {
        receiver_state,
        has_active_gap,
        nack_exhausted,
        first_frame_acquired,
        prior_output_established,
        displayed_idr_serving,
        displayed_idr_decoder_synced,
        decoder_reference_synced,
    }
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
