use super::SessionFlowProjection;
use std::sync::{Arc, Mutex};
use xbox_streaming::{
    SessionProgressSnapshot, SessionStartupBoundedRetrySnapshot, SessionStartupObserver,
    SessionStartupPhase, SessionStartupPhaseStatus,
};

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
