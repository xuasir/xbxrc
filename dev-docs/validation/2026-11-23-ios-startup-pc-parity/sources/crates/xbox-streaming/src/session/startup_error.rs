//! PC 与原生宿主共用启动错误的兜底分类；显式领域 hint 保持优先。
use super::{SessionFlowError, SessionFlowStartupErrorKind, SessionStartupPhase};

pub fn classify_startup_error_kind_fallback(
    phase: SessionStartupPhase,
    error: &SessionFlowError,
) -> SessionFlowStartupErrorKind {
    use SessionFlowStartupErrorKind as Kind;
    let normalized = error.message.to_ascii_lowercase();
    if normalized.contains("homeremoteplaynotready") {
        return Kind::HostRemotePlayUnavailable;
    }
    if normalized.contains("homesessionboundedretryexhausted") {
        return Kind::HostRegistrationRetryExhausted;
    }
    if normalized.contains("target missing") {
        return Kind::Target;
    }
    if normalized.contains("auth")
        || normalized.contains("token missing")
        || normalized.contains("streaming token is unavailable")
    {
        return Kind::Auth;
    }
    if normalized.contains("network error") || normalized.starts_with("operationtimeout:") {
        return Kind::Network;
    }
    match phase {
        SessionStartupPhase::ResolvingContext | SessionStartupPhase::Failed => Kind::Unknown,
        SessionStartupPhase::CreatingSession => Kind::SessionCreate,
        SessionStartupPhase::WaitingSessionReady => Kind::SessionReady,
        SessionStartupPhase::StartingRuntime | SessionStartupPhase::Ready => Kind::Runtime,
    }
}

pub fn is_startup_error_retryable_fallback(
    kind: &SessionFlowStartupErrorKind,
    error: &SessionFlowError,
) -> bool {
    let normalized = error.message.to_ascii_lowercase();
    if normalized.contains("homeremoteplaynotready")
        || normalized.contains("remoteconsolewakecircuitopen")
    {
        return false;
    }
    matches!(
        kind,
        SessionFlowStartupErrorKind::SessionCreate
            | SessionFlowStartupErrorKind::SessionReady
            | SessionFlowStartupErrorKind::Network
    ) || error.status.is_some_and(|status| status >= 500)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preflight_and_circuit_failures_keep_pc_retry_policy() {
        for message in ["homeRemotePlayNotReady", "remoteConsoleWakeCircuitOpen"] {
            let error = SessionFlowError::http(503, message, None);
            let kind =
                classify_startup_error_kind_fallback(SessionStartupPhase::CreatingSession, &error);
            assert!(!is_startup_error_retryable_fallback(&kind, &error));
        }
        let error = SessionFlowError::message("homeSessionBoundedRetryExhausted");
        assert_eq!(
            classify_startup_error_kind_fallback(SessionStartupPhase::CreatingSession, &error),
            SessionFlowStartupErrorKind::HostRegistrationRetryExhausted
        );
    }
}
