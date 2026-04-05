use crate::{XbxEnginePendingRuntimeRecoveryAction, XbxEngineRecoveryReasonDomain};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StageReconnectCandidateOutcome {
    StagedNew,
    StagedUpdated,
    Unchanged,
}

/// 统一 pending reconnect 的落地规则，避免多处直接改写相同状态。
pub(crate) fn stage_reconnect_candidate(
    pending_runtime_action: &mut Option<XbxEnginePendingRuntimeRecoveryAction>,
    observation_id: u64,
    reason: String,
    reason_domain: XbxEngineRecoveryReasonDomain,
) -> StageReconnectCandidateOutcome {
    match pending_runtime_action.as_mut() {
        None => {
            *pending_runtime_action = Some(
                XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
                    observation_id,
                    reason,
                    reason_domain,
                },
            );
            StageReconnectCandidateOutcome::StagedNew
        }
        Some(XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
            observation_id: pending_observation_id,
            reason: pending_reason,
            reason_domain: pending_reason_domain,
        }) => {
            if *pending_observation_id == observation_id
                && *pending_reason == reason
                && *pending_reason_domain == reason_domain
            {
                return StageReconnectCandidateOutcome::Unchanged;
            }
            *pending_observation_id = observation_id;
            *pending_reason = reason;
            *pending_reason_domain = reason_domain;
            StageReconnectCandidateOutcome::StagedUpdated
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{stage_reconnect_candidate, StageReconnectCandidateOutcome};
    use crate::{XbxEnginePendingRuntimeRecoveryAction, XbxEngineRecoveryReasonDomain};

    #[test]
    fn stage_reconnect_candidate_updates_existing_pending_request() {
        let mut pending = None;
        assert_eq!(
            stage_reconnect_candidate(
                &mut pending,
                7,
                "peer-failed".to_string(),
                XbxEngineRecoveryReasonDomain::ConnectivityTransport,
            ),
            StageReconnectCandidateOutcome::StagedNew
        );
        assert_eq!(
            stage_reconnect_candidate(
                &mut pending,
                9,
                "peer-closed".to_string(),
                XbxEngineRecoveryReasonDomain::ConnectivityTransport,
            ),
            StageReconnectCandidateOutcome::StagedUpdated
        );
        assert!(matches!(
            pending,
            Some(
                XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
                    observation_id: 9,
                    reason,
                    reason_domain: XbxEngineRecoveryReasonDomain::ConnectivityTransport,
                }
            ) if reason == "peer-closed"
        ));
    }

    #[test]
    fn stage_reconnect_candidate_returns_unchanged_when_same_request_repeated() {
        let mut pending = Some(
            XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
                observation_id: 9,
                reason: "peer-closed".to_string(),
                reason_domain: XbxEngineRecoveryReasonDomain::ConnectivityTransport,
            },
        );
        assert_eq!(
            stage_reconnect_candidate(
                &mut pending,
                9,
                "peer-closed".to_string(),
                XbxEngineRecoveryReasonDomain::ConnectivityTransport,
            ),
            StageReconnectCandidateOutcome::Unchanged
        );
        assert!(matches!(
            pending,
            Some(
                XbxEnginePendingRuntimeRecoveryAction::RequestReconnectCandidate {
                    observation_id: 9,
                    reason,
                    reason_domain: XbxEngineRecoveryReasonDomain::ConnectivityTransport,
                }
            ) if reason == "peer-closed"
        ));
    }
}
