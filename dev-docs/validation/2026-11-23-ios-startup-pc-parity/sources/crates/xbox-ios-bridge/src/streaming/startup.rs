use super::SessionFlowProjection;
use crate::{XboxBridgeError, XboxQueryFailure, XboxQueryFailureKind};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use xbox_streaming::{
    SessionProgressSnapshot, SessionStartupBoundedRetrySnapshot, SessionStartupObserver,
    SessionStartupPhase, SessionStartupPhaseStatus,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum XboxStartupFailureKind {
    SessionCreate,
    SessionReady,
    Runtime,
    Network,
    Auth,
    Target,
    HostRemotePlayUnavailable,
    HostRegistrationRetryExhausted,
    Unknown,
}

/// Swift 仅接收领域分类及固定诊断码，避免远端正文进入 UI/trace。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct XboxStartupFailure {
    pub kind: XboxStartupFailureKind,
    pub phase: String,
    pub retryable: bool,
    pub status_code: Option<u16>,
    pub failure_code: Option<String>,
    pub network_error_kind: Option<String>,
}

pub(super) fn map_startup_error(
    error: xbox_streaming::SessionFlowError,
    phase: SessionStartupPhase,
) -> super::XboxStreamingError {
    if error.message.starts_with("operationCancelled:") {
        return super::XboxStreamingError::Cancelled(0);
    }
    use xbox_streaming::session::startup_error::{
        classify_startup_error_kind_fallback, is_startup_error_retryable_fallback,
    };
    use xbox_streaming::SessionFlowStartupErrorKind as Kind;
    let kind = error
        .startup_hint
        .as_ref()
        .map(|hint| hint.kind.clone())
        .unwrap_or_else(|| classify_startup_error_kind_fallback(phase, &error));
    let retryable = error
        .startup_hint
        .as_ref()
        .map(|hint| hint.retryable)
        .unwrap_or_else(|| is_startup_error_retryable_fallback(&kind, &error));
    let kind = match kind {
        Kind::SessionCreate => XboxStartupFailureKind::SessionCreate,
        Kind::SessionReady => XboxStartupFailureKind::SessionReady,
        Kind::Runtime => XboxStartupFailureKind::Runtime,
        Kind::Network => XboxStartupFailureKind::Network,
        Kind::Auth => XboxStartupFailureKind::Auth,
        Kind::Target => XboxStartupFailureKind::Target,
        Kind::HostRemotePlayUnavailable => XboxStartupFailureKind::HostRemotePlayUnavailable,
        Kind::HostRegistrationRetryExhausted => {
            XboxStartupFailureKind::HostRegistrationRetryExhausted
        }
        Kind::Unknown => XboxStartupFailureKind::Unknown,
    };
    let codes = [
        "homeSessionBoundedRetryExhausted",
        "homeRemotePlayNotReady",
        "remoteConsoleWakeCircuitOpen",
        "remoteConsoleNotReady",
        "homeConsoleLookupTimeout",
        "homeTargetContextMissing",
        "homeCommandIdentityMissing",
        "homeDiscoveryCredentialRegionChanged",
        "homeRecreateReusedSession",
        "streamingStartTimeout",
        "createOutcomeUnresolved",
        "operationTimeout",
    ];
    let normalized = error.message.to_ascii_lowercase();
    let failure_code = codes
        .into_iter()
        .find(|code| normalized.contains(&code.to_ascii_lowercase()))
        .map(str::to_owned);
    let network_error_kind = error
        .startup_hint
        .as_ref()
        .and_then(|hint| serde_json::from_str::<serde_json::Value>(&hint.diagnostic_summary).ok())
        .and_then(|facts| facts["networkErrorKind"].as_str().map(str::to_owned))
        .filter(|kind| {
            [
                "timeout", "dns", "tls", "connect", "body", "request", "unknown",
            ]
            .contains(&kind.as_str())
        });
    super::XboxStreamingError::Startup {
        failure: XboxStartupFailure {
            kind,
            phase: phase_name(phase).into(),
            retryable,
            status_code: error.status,
            failure_code,
            network_error_kind,
        },
    }
}

fn phase_name(phase: SessionStartupPhase) -> &'static str {
    match phase {
        SessionStartupPhase::ResolvingContext => "resolvingContext",
        SessionStartupPhase::CreatingSession => "creatingSession",
        SessionStartupPhase::WaitingSessionReady => "waitingSessionReady",
        SessionStartupPhase::StartingRuntime => "startingRuntime",
        SessionStartupPhase::Ready => "ready",
        SessionStartupPhase::Failed => "failed",
    }
}

/// 从授权开始到 Plan 解析贯穿同一取消作用域，预算取 PC 的公共常量。
#[derive(uniffi::Object)]
pub struct XboxStreamStartup {
    pub(super) scope: xbox_streaming::SessionStartupScope,
    pub(super) deadline: Instant,
    pub(super) target: xbox_streaming::Target,
}

#[uniffi::export]
pub fn create_stream_startup(
    target_type: String,
) -> Result<Arc<XboxStreamStartup>, super::XboxStreamingError> {
    let target = super::parse_target_type(&target_type)?;
    Ok(Arc::new(XboxStreamStartup {
        scope: xbox_streaming::SessionStartupScope::default(),
        deadline: Instant::now() + xbox_streaming::session::RESOLVING_CONTEXT_TIMEOUT,
        target,
    }))
}

#[uniffi::export]
impl XboxStreamStartup {
    pub fn cancel(&self) {
        self.scope.cancel();
    }
}

impl XboxStreamStartup {
    pub(super) fn context_timeout(&self) -> Result<Duration, xbox_streaming::SessionFlowError> {
        if self.scope.is_cancelled() {
            return Err(xbox_streaming::SessionFlowError::message(
                "operationCancelled:stage=resolvingContext",
            ));
        }
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            self.scope.cancel();
            return Err(xbox_streaming::SessionFlowError::message(
                "operationTimeout:stage=resolvingContext",
            ));
        }
        Ok(remaining)
    }

    pub(crate) async fn run_context<T>(
        &self,
        target: xbox_streaming::Target,
        operation: impl Future<Output = Result<T, XboxBridgeError>>,
    ) -> Result<T, XboxBridgeError> {
        if target != self.target {
            return Err(XboxBridgeError::InvalidData("startupTargetMismatch".into()));
        }
        let remaining = self.context_timeout().map_err(context_error)?;
        Box::pin(self.scope.run("resolvingContext", Some(remaining), async {
            Ok(Box::pin(operation).await)
        }))
        .await
        .map_err(context_error)?
    }
}

fn context_error(error: xbox_streaming::SessionFlowError) -> XboxBridgeError {
    if error.message.starts_with("operationTimeout:") {
        XboxBridgeError::Query {
            failure: XboxQueryFailure {
                kind: XboxQueryFailureKind::Timeout,
                status_code: None,
                retriable: true,
                retry_after_seconds: None,
            },
        }
    } else {
        XboxBridgeError::Cancelled
    }
}

/// 仅投影展示所需阶段与秒数，详情、身份、凭据和完整地址留在 Rust 会话层。
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct XboxStartupProgress {
    pub phase: String,
    pub status: String,
    pub stream_state: Option<String>,
    pub total_wait_seconds: Option<u64>,
    pub allocation_wait_seconds: Option<u64>,
    pub provisioning_wait_seconds: Option<u64>,
    pub retry_count: u8,
    pub retry_limit: u8,
}

impl XboxStartupProgress {
    pub(super) fn initial(home: bool) -> Self {
        Self {
            phase: if home {
                "preparingConsole"
            } else {
                "resolvingContext"
            }
            .into(),
            status: "entered".into(),
            stream_state: None,
            total_wait_seconds: None,
            allocation_wait_seconds: None,
            provisioning_wait_seconds: None,
            retry_count: 0,
            retry_limit: 0,
        }
    }

    pub(super) fn apply_session(
        &mut self,
        progress: &SessionProgressSnapshot,
        stream_state: Option<&str>,
    ) {
        self.phase = "waitingSessionReady".into();
        self.stream_state = stream_state
            .filter(|state| {
                [
                    "Queued",
                    "Provisioning",
                    "ReadyToConnect",
                    "Provisioned",
                    "Failed",
                    "WaitingForResources",
                ]
                .contains(state)
            })
            .map(str::to_owned);
        self.total_wait_seconds = progress.queue_seconds.or_else(|| {
            progress
                .queue
                .as_ref()?
                .estimated_total_wait_time_in_seconds
        });
        self.allocation_wait_seconds = progress
            .queue
            .as_ref()
            .and_then(|queue| queue.estimated_allocation_time_in_seconds);
        self.provisioning_wait_seconds = progress
            .queue
            .as_ref()
            .and_then(|queue| queue.estimated_provisioning_time_in_seconds);
    }
}

pub(super) struct IosStartupObserver {
    projection: Arc<Mutex<SessionFlowProjection>>,
    generation: u64,
}

impl IosStartupObserver {
    pub(super) fn new(projection: Arc<Mutex<SessionFlowProjection>>, generation: u64) -> Self {
        Self {
            projection,
            generation,
        }
    }
}

impl SessionStartupObserver for IosStartupObserver {
    fn on_phase_event(
        &self,
        phase: SessionStartupPhase,
        status: SessionStartupPhaseStatus,
        _details: Option<&str>,
    ) {
        let Ok(mut projection) = self.projection.lock() else {
            return;
        };
        if projection.generation != self.generation {
            return;
        }
        let Some(progress) = &mut projection.startup_progress else {
            return;
        };
        progress.phase = match phase {
            SessionStartupPhase::ResolvingContext => "resolvingContext",
            SessionStartupPhase::CreatingSession => "creatingSession",
            SessionStartupPhase::WaitingSessionReady => "waitingSessionReady",
            SessionStartupPhase::StartingRuntime => "startingRuntime",
            SessionStartupPhase::Ready => "ready",
            SessionStartupPhase::Failed => "failed",
        }
        .into();
        progress.status = match status {
            SessionStartupPhaseStatus::Entered => "entered",
            SessionStartupPhaseStatus::Succeeded => "succeeded",
            SessionStartupPhaseStatus::Failed => "failed",
        }
        .into();
        if phase != SessionStartupPhase::Failed {
            projection.startup_phase = Some(phase);
        }
    }

    fn on_bounded_retry(
        &self,
        _phase: SessionStartupPhase,
        retry: &SessionStartupBoundedRetrySnapshot,
    ) {
        let Ok(mut projection) = self.projection.lock() else {
            return;
        };
        if projection.generation != self.generation {
            return;
        }
        if let Some(progress) = &mut projection.startup_progress {
            progress.retry_count = retry.retry_count;
            progress.retry_limit = retry.retry_limit;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xbox_streaming::{RuntimeLaunchState, SessionPhase};

    #[tokio::test]
    async fn authorization_and_context_share_deadline_and_cancellation() {
        let mut startup = XboxStreamStartup {
            scope: Default::default(),
            target: xbox_streaming::Target::Home,
            deadline: Instant::now() + xbox_streaming::session::RESOLVING_CONTEXT_TIMEOUT,
        };
        assert!(startup.context_timeout().unwrap() <= Duration::from_secs(90));
        startup
            .run_context(xbox_streaming::Target::Home, async { Ok(()) })
            .await
            .unwrap();
        assert!(matches!(
            startup
                .run_context(xbox_streaming::Target::Cloud, async { Ok(()) })
                .await,
            Err(XboxBridgeError::InvalidData(_))
        ));
        startup.deadline = Instant::now() - Duration::from_secs(1);
        let expired = startup
            .run_context(
                xbox_streaming::Target::Home,
                std::future::pending::<Result<(), XboxBridgeError>>(),
            )
            .await;
        assert!(
            matches!(expired, Err(XboxBridgeError::Query { failure }) if failure.kind == XboxQueryFailureKind::Timeout && failure.retriable)
        );
        assert!(startup.scope.is_cancelled());
        assert!(matches!(
            startup
                .run_context(xbox_streaming::Target::Home, async { Ok(()) })
                .await,
            Err(XboxBridgeError::Cancelled)
        ));
    }

    #[test]
    fn startup_errors_keep_shared_hint_status_and_safe_network_facts() {
        use xbox_streaming::SessionFlowError;
        for (error, kind, retryable, status) in [
            (
                SessionFlowError::from(xbox_webapi::WebApiError::http(401, "private-credential")),
                XboxStartupFailureKind::Auth,
                false,
                Some(401),
            ),
            (
                SessionFlowError::from(xbox_webapi::WebApiError::http(403, "private-account")),
                XboxStartupFailureKind::Target,
                false,
                Some(403),
            ),
            (
                SessionFlowError::from(xbox_webapi::WebApiError::network_kind(
                    xbox_webapi::NetworkErrorKind::Timeout,
                )),
                XboxStartupFailureKind::Network,
                true,
                None,
            ),
            (
                SessionFlowError::message("operationTimeout:stage=resolvingContext"),
                XboxStartupFailureKind::Network,
                true,
                None,
            ),
        ] {
            let projected = map_startup_error(error, SessionStartupPhase::ResolvingContext);
            let super::super::XboxStreamingError::Startup { failure } = projected else {
                panic!("startup projection required");
            };
            assert_eq!(
                (failure.kind, failure.retryable, failure.status_code),
                (kind, retryable, status)
            );
            assert_eq!(failure.phase, "resolvingContext");
            assert!(!format!("{failure:?}").contains("private-"));
        }
    }

    #[test]
    fn queue_projection_keeps_only_safe_state_and_timing() {
        let session = SessionProgressSnapshot {
            session_id: "fixture-session-id".into(),
            phase: SessionPhase::WaitingSessionReady,
            runtime_launch_state: RuntimeLaunchState::Blocked,
            status_text_key: "fixture-status".into(),
            queue_seconds: Some(90),
            queue: Some(xbox_streaming::session::monitor::QueueDetails {
                estimated_total_wait_time_in_seconds: Some(120),
                estimated_allocation_time_in_seconds: Some(20),
                estimated_provisioning_time_in_seconds: Some(10),
            }),
            error_code: Some("fixture-error".into()),
            error_message: Some("fixture-private-detail".into()),
            error_hint: None,
        };
        let mut progress = XboxStartupProgress::initial(false);
        progress.apply_session(&session, Some("Queued"));
        assert_eq!(progress.stream_state.as_deref(), Some("Queued"));
        assert_eq!(progress.total_wait_seconds, Some(90));
        assert_eq!(progress.allocation_wait_seconds, Some(20));
        assert_eq!(progress.provisioning_wait_seconds, Some(10));
        assert!(!format!("{progress:?}").contains("fixture-"));
        progress.apply_session(&session, Some("fixture-private-state"));
        assert_eq!(progress.stream_state, None);
    }

    #[test]
    fn observer_discards_old_generation_and_ignores_details() {
        let projection = Arc::new(Mutex::new(SessionFlowProjection {
            generation: 2,
            startup_progress: Some(XboxStartupProgress::initial(true)),
            ..Default::default()
        }));
        let old = IosStartupObserver::new(projection.clone(), 1);
        old.on_phase_event(
            SessionStartupPhase::Failed,
            SessionStartupPhaseStatus::Failed,
            Some("fixture-private-detail"),
        );
        assert_eq!(
            projection
                .lock()
                .unwrap()
                .startup_progress
                .as_ref()
                .unwrap()
                .phase,
            "preparingConsole"
        );
        let current = IosStartupObserver::new(projection.clone(), 2);
        current.on_phase_event(
            SessionStartupPhase::CreatingSession,
            SessionStartupPhaseStatus::Entered,
            Some("fixture-private-detail"),
        );
        let snapshot = projection.lock().unwrap().startup_progress.clone().unwrap();
        assert_eq!(snapshot.phase, "creatingSession");
        assert!(!format!("{snapshot:?}").contains("fixture-private-detail"));
        projection.lock().unwrap().startup_progress = None;
        current.on_phase_event(
            SessionStartupPhase::Ready,
            SessionStartupPhaseStatus::Succeeded,
            None,
        );
        assert!(projection.lock().unwrap().startup_progress.is_none());
    }
}
