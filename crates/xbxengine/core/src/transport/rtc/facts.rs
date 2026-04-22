use std::fmt;

use crate::media::video::ingress::scheduler::IngressDecision;
use crate::XbxEngineRecoveryReasonDomain;

/// 连接生命周期事实，尽量保持中性语义。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionLifecycleStateFact {
    New,
    Connecting,
    Connected,
    Disconnected,
    Failed,
    Closed,
    Recovering,
}

/// 数据通道标签事实，只保留当前主线使用的固定通道。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataChannelLabelFact {
    Control,
    Message,
    Input,
    Chat,
}

impl fmt::Display for DataChannelLabelFact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Control => write!(f, "control"),
            Self::Message => write!(f, "message"),
            Self::Input => write!(f, "input"),
            Self::Chat => write!(f, "chat"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PeerFact {
    ConnectionStateChanged {
        state: ConnectionLifecycleStateFact,
        observed_at_ms: f64,
    },
    DataChannelOpened {
        label: DataChannelLabelFact,
        observed_at_ms: f64,
    },
    DataChannelClosed {
        label: DataChannelLabelFact,
        observed_at_ms: f64,
    },
    DataChannelBufferedAmountHigh {
        label: DataChannelLabelFact,
        observed_at_ms: f64,
    },
    DataChannelBufferedAmountLow {
        label: DataChannelLabelFact,
        observed_at_ms: f64,
    },
    TransportMetricsSampled {
        video_rtt_ms: Option<f64>,
        loss_ratio_1s: f64,
        actual_video_bitrate_kbps: Option<f64>,
        observed_remb_kbps: Option<u32>,
        transport_path: Option<String>,
        observed_at_ms: f64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IngressDecisionFact {
    Submit,
    DropLate,
    DropBacklogIncoming,
    DropBacklogEvictQueued,
    DropUnrecoverable,
    WaitKeyframe,
    Reconfigure,
}

impl From<&IngressDecision> for IngressDecisionFact {
    fn from(decision: &IngressDecision) -> Self {
        match decision {
            IngressDecision::Submit => Self::Submit,
            IngressDecision::DropLate => Self::DropLate,
            IngressDecision::DropBacklogIncoming => Self::DropBacklogIncoming,
            IngressDecision::DropBacklogEvictQueued => Self::DropBacklogEvictQueued,
            IngressDecision::DropUnrecoverable => Self::DropUnrecoverable,
            IngressDecision::WaitKeyframe => Self::WaitKeyframe,
            IngressDecision::Reconfigure => Self::Reconfigure,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum MediaFact {
    FrameArrived {
        rtp_timestamp: u32,
        width: u32,
        height: u32,
        is_keyframe: bool,
        observed_at_ms: f64,
    },
    TransportObservationRaised {
        label: String,
        severity: u8,
        observed_at_ms: f64,
    },
    IngressDecisionObserved {
        decision: IngressDecisionFact,
        queue_depth: usize,
        observed_at_ms: f64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum TimerFact {
    MetricsSampleTick { observed_at_ms: f64 },
}

#[derive(Clone, Debug, PartialEq)]
pub enum TransportCommand {
    RequestPli {
        reason: String,
        observation_id: u64,
    },
    RequestFir {
        reason: String,
        observation_id: u64,
    },
    #[allow(dead_code)]
    RequestDecoderReset {
        reason: String,
        observation_id: u64,
    },
    RequestReconnectCandidate {
        reason: String,
        reason_domain: XbxEngineRecoveryReasonDomain,
        observation_id: u64,
    },
    SetTargetRembKbps {
        target_kbps: u32,
        reason: String,
        observation_id: u64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum SessionCommand {
    Transport(TransportCommand),
    LocalDecoderReset { reason: String, observation_id: u64 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandResultStatus {
    Succeeded,
    Deferred { reason: String },
    Failed { error: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct CommandResultFact {
    pub command: TransportCommand,
    pub status: CommandResultStatus,
    pub observed_at_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TransportFact {
    Peer(PeerFact),
    Media(MediaFact),
    Timer(TimerFact),
    CommandResult(CommandResultFact),
}
