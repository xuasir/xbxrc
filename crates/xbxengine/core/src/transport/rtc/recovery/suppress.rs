//! Session 层恢复动作抑制：picture 关键帧仅 receive 执行。

use crate::transport::rtc::recovery::coordinator::RecoveryOwnerSignal;
use crate::transport::rtc::recovery::escalation::{RecoveryAction, VideoEscalationReason};
use crate::transport::rtc::session::control_model::{
    resolve_session_fault_domain, SessionFaultDomain,
};
use crate::XbxEngineMediaRuntimeStats;

/// receive 侧已在发 PLI/FIR 时，session 不应再 coalesce 为 keyframeInFlight。
pub(crate) fn receive_local_keyframe_request_recent(stats: &XbxEngineMediaRuntimeStats) -> bool {
    stats
        .latest_keyframe_request_source
        .as_ref()
        .is_some_and(|source| {
            source.starts_with("insert-gate")
                || source.starts_with("receiver-local")
                || source.starts_with("media-supply")
                || source.starts_with("chain-recovery")
        })
}

/// 记录 session 将图片级恢复委托给 receive（sticky authority + 可重复 trace 计数）。
pub(crate) fn record_picture_recovery_delegation(
    stats: &mut crate::XbxEngineMediaRuntimeStats,
    detail: &str,
) {
    stats.recovery_picture_recovery_authority = Some("receive".to_string());
    stats.recovery_picture_recovery_delegated_total = stats
        .recovery_picture_recovery_delegated_total
        .saturating_add(1);
    stats.latest_observation_label = Some("pictureRecoveryDelegated".to_string());
    stats.latest_observation_summary = Some(detail.to_string());
}

/// Session / recovery planner 不得再下发 PLI/FIR（receive-only 纪律）。
pub(crate) fn suppress_session_picture_recovery_action(action: RecoveryAction) -> RecoveryAction {
    match action {
        RecoveryAction::RequestPli | RecoveryAction::RequestFir => {
            RecoveryAction::DelegatedToReceive
        }
        other => other,
    }
}

/// 合并原 `should_suppress_display_picture_recovery_action`：DisplaySupply 误报 PLI 在进 planner 前即抑制。
pub(crate) fn should_suppress_display_supply_picture_recovery(
    action: RecoveryAction,
    owner_signal: &RecoveryOwnerSignal,
) -> bool {
    if !matches!(
        action,
        RecoveryAction::RequestPli | RecoveryAction::RequestFir
    ) {
        return false;
    }
    if matches!(
        owner_signal.reason_label.as_str(),
        "receiverWaitingKeyframe"
            | "waitKeyframe"
            | "ingressWaitKeyframe"
            | "idrRecoveryActive"
            | "supplyBreakKeyframeHint"
    ) {
        return false;
    }
    matches!(
        resolve_session_fault_domain(owner_signal.reason),
        SessionFaultDomain::DisplaySupply
    ) || matches!(
        owner_signal.reason,
        VideoEscalationReason::DisplaySupplyCritical
    )
}

/// session 出口唯一收口：先 DisplaySupply 门控，再 receive-only PLI/FIR 映射。
pub(crate) fn finalize_session_picture_recovery_action(
    action: RecoveryAction,
    owner_signal: &RecoveryOwnerSignal,
) -> RecoveryAction {
    if should_suppress_display_supply_picture_recovery(action, owner_signal) {
        return RecoveryAction::CooldownSuppressed;
    }
    suppress_session_picture_recovery_action(action)
}
