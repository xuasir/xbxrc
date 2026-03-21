use crate::transport::rtc::recovery::coordinator::{RecoveryDispatch, RecoveryRuntimeState};
use crate::transport::rtc::recovery::escalation::VideoEscalationDecision;
use crate::transport::rtc::recovery::signal::{VideoIngressSignal, VideoRecoverySignal};

// recovery scheduler 的显式输入面：
// - 启动重试 tick
// - transport recovery signal
// - ingress recovery signal
pub(super) enum RecoverySchedulerInput {
    StartupRetryTick,
    TransportSignal(VideoRecoverySignal),
    IngressSignal(VideoIngressSignal),
}

// recovery scheduler 的显式输出面：
// - 先发布 runtime state，再执行恢复动作
// - session 只消费 dispatch，不再依赖 driver 内部副作用顺序
pub(super) enum RecoverySchedulerDispatch {
    PublishRuntimeState(RecoveryRuntimeState),
    ExecuteRecoveryAction {
        decision: VideoEscalationDecision,
        reason_label: String,
        observed_at_ms: f64,
    },
}

impl From<RecoveryDispatch> for Vec<RecoverySchedulerDispatch> {
    fn from(dispatch: RecoveryDispatch) -> Self {
        let observed_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as f64)
            .unwrap_or(0.0);
        vec![
            RecoverySchedulerDispatch::PublishRuntimeState(dispatch.runtime_state.clone()),
            RecoverySchedulerDispatch::ExecuteRecoveryAction {
                reason_label: dispatch.runtime_state.diagnosis_label,
                decision: dispatch.decision,
                observed_at_ms,
            },
        ]
    }
}
