use crate::media::video::h264::inspection::{H264AccessUnitInspection, H264BootstrapRejectReason};
use crate::media::video::types::AssembledVideoFrame;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InspectionAdmission {
    Accept,
    AwaitRecoveryKeyframe,
}

pub fn resolve_inspection_admission(
    inspection: &H264AccessUnitInspection,
    prior_output_continuation_allowed: bool,
    decoder_bootstrap_no_output_continuation_allowed: bool,
    sustaining_recovery_continuation_allowed: bool,
) -> InspectionAdmission {
    if !inspection.slice_headers_valid {
        return InspectionAdmission::AwaitRecoveryKeyframe;
    }

    if inspection.bootstrap_ready {
        return InspectionAdmission::Accept;
    }

    if (prior_output_continuation_allowed
        || decoder_bootstrap_no_output_continuation_allowed
        || sustaining_recovery_continuation_allowed)
        && is_recovery_delta_continuation_ready(inspection)
    {
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
) -> &'static str {
    if !inspection.is_idr {
        if matches!(admission, InspectionAdmission::Accept)
            && inspection.delta_continuation_ready()
            && inspection.committed_sps_present()
            && inspection.committed_pps_present()
        {
            return "receiverLocalContinuation";
        }
        return "bootstrapMissingIdr";
    }
    match admission {
        InspectionAdmission::Accept => "firstKeyframeAccepted",
        InspectionAdmission::AwaitRecoveryKeyframe => inspection_bootstrap_reason(inspection),
    }
}
