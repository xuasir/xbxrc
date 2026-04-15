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
    /// 帧已入队（替换了低价值帧），语义上等同于 Submit，不触发 recovery 升级。
    FrameQueued,
    Reconfigure,
}

impl VideoIngressSignal {
    pub fn from_decision(decision: &IngressDecision) -> Self {
        match decision {
            IngressDecision::Reconfigure => Self::Reconfigure,
            IngressDecision::DropUnrecoverable => Self::FrameAbandoned,
            IngressDecision::WaitKeyframe
            | IngressDecision::DropLate
            // 新帧被丢弃（队列满且新帧价值不足），链路仍处于等待状态。
            | IngressDecision::DropBacklogIncoming => Self::WaitKeyframe,
            // 新帧已替换队内低价值帧并入队，语义上等同于 Submit（帧已被接受）。
            // 映射到 WaitKeyframe 会让 recovery 诊断误判为帧丢失，可能触发多余的 keyframe 请求。
            IngressDecision::DropBacklogEvictQueued => Self::FrameQueued,
            IngressDecision::Submit => unreachable!("submit 不应进入 recovery diagnosis"),
        }
    }
}
