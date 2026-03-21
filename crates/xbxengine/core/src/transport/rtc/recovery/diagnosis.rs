use crate::transport::rtc::recovery::escalation::VideoEscalationReason;
use crate::transport::rtc::recovery::signal::{VideoIngressSignal, VideoRecoverySignal};

/**
 * Diagnosis 层把事实信号映射成恢复域里的统一 reason/label。
 * Policy 只消费诊断结果，不再感知 adapter/ingress 的细节来源。
 */
pub struct VideoRecoveryDiagnosis {
    pub reason: VideoEscalationReason,
    pub label: &'static str,
}

impl VideoRecoverySignal {
    pub fn diagnose(self) -> VideoRecoveryDiagnosis {
        match self {
            VideoRecoverySignal::AdapterIdleTimeout => VideoRecoveryDiagnosis {
                reason: VideoEscalationReason::AdapterIdleTimeout,
                label: "adapterIdleTimeout",
            },
            VideoRecoverySignal::AdapterThinStream => VideoRecoveryDiagnosis {
                reason: VideoEscalationReason::AdapterThinStream,
                label: "adapterThinStream",
            },
            VideoRecoverySignal::TransportExpiredDeadline => VideoRecoveryDiagnosis {
                reason: VideoEscalationReason::TransportExpiredDeadline,
                label: "transportExpiredDeadline",
            },
            VideoRecoverySignal::TransportSevereDeadline => VideoRecoveryDiagnosis {
                reason: VideoEscalationReason::TransportSevereDeadline,
                label: "transportSevereDeadline",
            },
            VideoRecoverySignal::TransportRecoveredLate => VideoRecoveryDiagnosis {
                reason: VideoEscalationReason::TransportRecoveredLate,
                label: "transportRecoveredLate",
            },
            VideoRecoverySignal::TransportSampleLoss => VideoRecoveryDiagnosis {
                reason: VideoEscalationReason::TransportSampleLoss,
                label: "transportSampleLoss",
            },
            VideoRecoverySignal::TransportSampleLossBurst => VideoRecoveryDiagnosis {
                reason: VideoEscalationReason::WaitKeyframe,
                label: "transportSampleLoss",
            },
            VideoRecoverySignal::TransportAwaitRecoveryKeyframe => VideoRecoveryDiagnosis {
                reason: VideoEscalationReason::TransportAwaitRecoveryKeyframe,
                label: "transportAwaitRecoveryKeyframe",
            },
        }
    }
}

impl VideoIngressSignal {
    pub fn diagnose(self) -> VideoRecoveryDiagnosis {
        match self {
            VideoIngressSignal::WaitKeyframe => VideoRecoveryDiagnosis {
                reason: VideoEscalationReason::WaitKeyframe,
                label: "ingressWaitKeyframe",
            },
            VideoIngressSignal::Reconfigure => VideoRecoveryDiagnosis {
                reason: VideoEscalationReason::Reconfigure,
                label: "ingressReconfigure",
            },
        }
    }
}

pub fn diagnose_transport_signal(signal: VideoRecoverySignal) -> VideoRecoveryDiagnosis {
    signal.diagnose()
}

pub fn diagnose_ingress_signal(signal: VideoIngressSignal) -> VideoRecoveryDiagnosis {
    signal.diagnose()
}

#[cfg(test)]
mod tests {
    use super::{diagnose_ingress_signal, diagnose_transport_signal};
    use crate::transport::rtc::recovery::escalation::VideoEscalationReason;
    use crate::transport::rtc::recovery::signal::{VideoIngressSignal, VideoRecoverySignal};

    #[test]
    fn transport_sample_loss_burst_maps_to_wait_keyframe_reason() {
        let diagnosis = diagnose_transport_signal(VideoRecoverySignal::TransportSampleLossBurst);
        match diagnosis.reason {
            VideoEscalationReason::WaitKeyframe => {}
            _ => panic!("transport sample loss burst should diagnose to wait-keyframe"),
        }
        assert_eq!(diagnosis.label, "transportSampleLoss");
    }

    #[test]
    fn ingress_reconfigure_maps_to_reconfigure_reason() {
        let diagnosis = diagnose_ingress_signal(VideoIngressSignal::Reconfigure);
        match diagnosis.reason {
            VideoEscalationReason::Reconfigure => {}
            _ => panic!("ingress reconfigure should diagnose to reconfigure"),
        }
        assert_eq!(diagnosis.label, "ingressReconfigure");
    }
}
