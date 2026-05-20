//! RFC 2026-05-20：pre-decode 关键帧请求仅由 `RtcReceiveCore` + `RtcTransportCapability` 执行。

use crate::transport::rtc::recovery::escalation::RecoveryAction;

pub(crate) const RECEIVER_WAITING_KEYFRAME_REASON: &str = "receiverWaitingKeyframe";
pub(crate) const RECEIVER_LOCAL_CONTINUATION_REASON: &str = "receiverLocalContinuation";

/// Session / recovery 层不得再下发 PLI/FIR。
pub(crate) fn suppress_session_picture_recovery_action(action: RecoveryAction) -> RecoveryAction {
    match action {
        RecoveryAction::RequestPli | RecoveryAction::RequestFir => {
            RecoveryAction::CooldownSuppressed
        }
        other => other,
    }
}

pub(crate) fn remap_legacy_picture_recovery_label(label: &str) -> &str {
    match label {
        "receiverWaitingKeyframe" | "awaitRecoveryAnchor" | "awaitingRecoveryAnchor" => {
            RECEIVER_WAITING_KEYFRAME_REASON
        }
        "receiverLocalContinuation" => RECEIVER_LOCAL_CONTINUATION_REASON,
        other => other,
    }
}
