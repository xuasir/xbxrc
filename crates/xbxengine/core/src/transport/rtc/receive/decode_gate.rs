use crate::media::video::h264::inspection::{H264AccessUnitInspection, H264BootstrapRejectReason};
use crate::media::video::types::AssembledVideoFrame;
use crate::transport::rtc::receive::ReceiverState;
use crate::transport::rtc::recovery::contract::is_recovery_delta_continuation_ready;

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
    /// 宿主已显示过 IDR：steady 非 IDR continuation 不应再被 bootstrapMissingIdr 打回 await-anchor。
    pub displayed_idr_serving: bool,
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
        && ctx.displayed_idr_serving
        && is_recovery_delta_continuation_ready(inspection)
}

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
        // 已有 decode/displayed-idr 输出事实后，codec 元数据齐备的 delta 应本地接纳；
        // receiver 仍标 WaitingKeyframe 时不应把整个 ingress 打进 await-anchor。
        if ctx.prior_output_established
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
        if ctx.hard_gap_blocks_delta() {
            return InspectionAdmission::AwaitRecoveryKeyframe;
        }
        return InspectionAdmission::Accept;
    }

    InspectionAdmission::AwaitRecoveryKeyframe
}

pub fn prior_output_continuation_allowed(
    first_frame_acquired: bool,
    is_blocking_non_keyframe_admission: bool,
) -> bool {
    first_frame_acquired && !is_blocking_non_keyframe_admission
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
