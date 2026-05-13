use crate::transport::rtc::recovery::escalation::VideoEscalationReason;
use crate::transport::rtc::recovery::startup::SessionPhase;

pub(crate) fn parse_session_phase(value: Option<&str>) -> SessionPhase {
    match value {
        Some("startup" | "connecting" | "handshaking" | "priming") => SessionPhase::Startup,
        Some("recovering" | "ramp-up" | "degraded") => SessionPhase::Recovering,
        _ => SessionPhase::Steady,
    }
}

pub(crate) fn map_label_to_escalation_reason(label: &str) -> Option<VideoEscalationReason> {
    if label.contains("waitKeyframeEntered:config_changed")
        || label.contains("waitKeyframeEntered:config_mismatch")
    {
        return Some(VideoEscalationReason::LocalSupplySuspect);
    }
    match label {
        "ingressWaitKeyframe" => Some(VideoEscalationReason::WaitKeyframe),
        "ingressFrameAbandoned" => Some(VideoEscalationReason::LocalSupplySuspect),
        "waitKeyframeEntered" => Some(VideoEscalationReason::WaitKeyframe),
        "frameAbandoned" => Some(VideoEscalationReason::LocalSupplySuspect),
        "transportAwaitRecoveryAnchor" => {
            Some(VideoEscalationReason::TransportAwaitRecoveryKeyframe)
        }
        "transportAwaitRecoverySuspect" | "localSupplySuspect" | "rebuildingSupplySuspect" => {
            Some(VideoEscalationReason::LocalSupplySuspect)
        }
        "bootstrapMissingSps" | "bootstrapMissingPps" | "inspectionRejectInvalidSliceHeader" => {
            Some(VideoEscalationReason::LocalSupplySuspect)
        }
        "displaySupplyCritical" => Some(VideoEscalationReason::DisplaySupplyCritical),
        "ingressReconfigure" => Some(VideoEscalationReason::Reconfigure),
        "decoderBackendFailure" => Some(VideoEscalationReason::DecoderBackendFailure),
        "adapterIdleTimeout" => Some(VideoEscalationReason::AdapterIdleTimeout),
        "adapterThinStream" => Some(VideoEscalationReason::AdapterThinStream),
        "transportLowValueDeadline" => Some(VideoEscalationReason::TransportLowValueDeadline),
        "transportRepairableDeadline" => Some(VideoEscalationReason::TransportRepairableDeadline),
        "transportExpiredDeadline" => Some(VideoEscalationReason::TransportExpiredDeadline),
        "transportSevereDeadline" => Some(VideoEscalationReason::TransportSevereDeadline),
        "transportRecoveredLate" => Some(VideoEscalationReason::TransportRecoveredLate),
        "transportSampleLoss" => Some(VideoEscalationReason::TransportSampleLoss),
        _ => None,
    }
}

pub(crate) fn resolve_connectivity_fallback_reason(label: &str) -> Option<VideoEscalationReason> {
    let reason = map_label_to_escalation_reason(label)?;
    match reason {
        VideoEscalationReason::TransportExpiredDeadline
        | VideoEscalationReason::TransportSevereDeadline
        | VideoEscalationReason::TransportRecoveredLate
        | VideoEscalationReason::TransportSampleLoss => Some(reason),
        _ => None,
    }
}

pub(crate) fn resolve_lifecycle_reconnect_reason_label(
    lifecycle_disconnected: bool,
    recovering_connectivity_failure: bool,
    force_lifecycle_reconnect: bool,
) -> &'static str {
    if lifecycle_disconnected {
        "rtcConnectionDisconnected"
    } else if recovering_connectivity_failure {
        "rtcConnectionRecovering"
    } else if force_lifecycle_reconnect {
        "livenessNoProgressTimeout"
    } else {
        "rtcConnectionRecovering"
    }
}
