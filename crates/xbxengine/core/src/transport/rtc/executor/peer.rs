use crate::XbxEnginePendingRuntimeRecoveryAction;

/// 统一 pending reconnect 的落地规则，避免多处直接改写相同状态。
pub(crate) fn stage_reconnect_candidate(
    pending_runtime_action: &mut Option<XbxEnginePendingRuntimeRecoveryAction>,
    observation_id: u64,
    reason: String,
) -> bool {
    if pending_runtime_action.is_some() {
        return false;
    }
    *pending_runtime_action = Some(
        XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id,
            reason,
        },
    );
    true
}

#[cfg(test)]
mod tests {
    use super::stage_reconnect_candidate;
    use crate::XbxEnginePendingRuntimeRecoveryAction;

    #[test]
    fn stage_reconnect_candidate_keeps_first_pending_request() {
        let mut pending = None;
        assert!(stage_reconnect_candidate(
            &mut pending,
            7,
            "peer-failed".to_string()
        ));
        assert!(!stage_reconnect_candidate(
            &mut pending,
            9,
            "peer-closed".to_string()
        ));
        assert!(matches!(
            pending,
            Some(
                XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
                    observation_id: 7,
                    ..
                }
            )
        ));
    }
}
