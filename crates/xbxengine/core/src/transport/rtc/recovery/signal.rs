use crate::media::video::ingress::scheduler::IngressDecision;

/**
 * Signal 层只表达“观测到了什么”，不直接掺入恢复策略语义。
 * 这样 adapter / ingress 可以稳定产出事实，后续由 diagnosis 映射成具体 reason。
 */
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoRecoverySignal {
    DisplaySupplyCritical,
    AdapterIdleTimeout,
    AdapterThinStream,
    TransportExpiredDeadline,
    TransportSevereDeadline,
    TransportRecoveredLate,
    TransportSampleLoss,
    TransportSampleLossBurst,
    TransportAwaitRecoveryKeyframe,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoIngressSignal {
    WaitKeyframe,
    FrameAbandoned,
    Reconfigure,
}

impl VideoIngressSignal {
    pub fn from_decision(decision: &IngressDecision) -> Self {
        match decision {
            IngressDecision::Reconfigure => Self::Reconfigure,
            IngressDecision::DropUnrecoverable => Self::FrameAbandoned,
            IngressDecision::WaitKeyframe
            | IngressDecision::DropLate
            | IngressDecision::DropBacklog => Self::WaitKeyframe,
            IngressDecision::Submit => unreachable!("submit 不应进入 recovery diagnosis"),
        }
    }
}
