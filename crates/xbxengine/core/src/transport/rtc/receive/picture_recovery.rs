//! RFC 2026-05-20：pre-decode 关键帧请求仅由 `RtcReceiveCore` + `RtcTransportCapability` 执行。

use crate::transport::rtc::recovery::escalation::RecoveryAction;

/// Session / recovery 层不得再下发 PLI/FIR。
pub(crate) fn suppress_session_picture_recovery_action(action: RecoveryAction) -> RecoveryAction {
    match action {
        RecoveryAction::RequestPli | RecoveryAction::RequestFir => {
            RecoveryAction::CooldownSuppressed
        }
        other => other,
    }
}
