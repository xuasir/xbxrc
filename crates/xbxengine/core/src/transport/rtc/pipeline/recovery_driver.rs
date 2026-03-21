use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::{
    runtime_stats_sink::RuntimeStatsSink,
    transport::rtc::protocol::data_channel_state::XbxDataChannelState,
    transport::rtc::recovery::coordinator::{RecoveryCoordinator, RecoveryRuntimeState},
    transport::rtc::recovery::escalation::VideoEscalationController,
    transport::rtc::recovery::executor::apply_recovery_decision,
    XbxEngineMediaRuntimeStats, XbxEnginePendingRuntimeRecoveryAction,
};

use super::recovery_types::{RecoverySchedulerDispatch, RecoverySchedulerInput};

pub(super) struct RecoveryDriver {
    recovery_coordinator: RecoveryCoordinator,
    data_channel_state: Arc<Mutex<XbxDataChannelState>>,
    pending_runtime_action: Arc<Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>>,
    runtime_stats: RuntimeStatsSink,
    decode_handle: Arc<crate::media::video::decode::actor::DecodeActorHandle>,
}

impl RecoveryDriver {
    pub(super) fn new(
        escalation_controller: VideoEscalationController,
        data_channel_state: Arc<Mutex<XbxDataChannelState>>,
        pending_runtime_action: Arc<Mutex<Option<XbxEnginePendingRuntimeRecoveryAction>>>,
        runtime_stats: Arc<Mutex<XbxEngineMediaRuntimeStats>>,
        decode_handle: Arc<crate::media::video::decode::actor::DecodeActorHandle>,
        stream_started_at: std::time::Instant,
        startup_escalation_grace: Duration,
    ) -> Self {
        Self {
            recovery_coordinator: RecoveryCoordinator::new(
                escalation_controller,
                stream_started_at,
                startup_escalation_grace,
            ),
            data_channel_state,
            pending_runtime_action,
            runtime_stats: RuntimeStatsSink::new(runtime_stats),
            decode_handle,
        }
    }

    pub(super) fn schedule_input(
        &mut self,
        input: RecoverySchedulerInput,
    ) -> Vec<RecoverySchedulerDispatch> {
        match input {
            RecoverySchedulerInput::StartupRetryTick => self
                .recovery_coordinator
                .poll_startup_retry(self.runtime_stats.shared().as_ref())
                .map(|retry_decision| {
                    let runtime_state = self.recovery_coordinator.runtime_state_for_label(
                        self.runtime_stats.shared().as_ref(),
                        "startupLowQuality",
                    );
                    vec![
                        RecoverySchedulerDispatch::PublishRuntimeState(runtime_state),
                        RecoverySchedulerDispatch::ExecuteRecoveryAction {
                            decision: retry_decision,
                            reason_label: "startupLowQuality".to_string(),
                            observed_at_ms: now_ms_f64(),
                        },
                    ]
                })
                .unwrap_or_default(),
            RecoverySchedulerInput::TransportSignal(signal) => self
                .recovery_coordinator
                .on_transport_signal_with_runtime_stats(
                    signal,
                    self.runtime_stats.shared().as_ref(),
                )
                .into(),
            RecoverySchedulerInput::IngressSignal(signal) => self
                .recovery_coordinator
                .on_ingress_signal_with_runtime_stats(signal, self.runtime_stats.shared().as_ref())
                .into(),
        }
    }

    pub(super) async fn apply_dispatch(&self, dispatch: RecoverySchedulerDispatch) {
        match dispatch {
            RecoverySchedulerDispatch::PublishRuntimeState(state) => {
                self.publish_recovery_runtime_state(state);
            }
            RecoverySchedulerDispatch::ExecuteRecoveryAction {
                decision,
                reason_label,
                observed_at_ms,
            } => {
                apply_recovery_decision(
                    self.runtime_stats.shared(),
                    &self.pending_runtime_action,
                    &self.data_channel_state,
                    Some(&self.decode_handle),
                    decision,
                    reason_label.as_str(),
                    observed_at_ms,
                )
                .await;
            }
        }
    }

    fn publish_recovery_runtime_state(&self, state: RecoveryRuntimeState) {
        self.runtime_stats.record_recovery_runtime_state(
            state.phase.as_str().to_string(),
            state.recovery_policy_profile.to_string(),
            state.diagnosis_label,
            state.coupling.mode.as_str().to_string(),
            state.coupling.summary(),
        );
    }
}

fn now_ms_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}
