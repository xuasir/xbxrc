/// RTCP 视频反馈目标能力状态（receiver-local，不进入全局 recovery 叙事）。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VideoFeedbackState {
    #[default]
    Warming,
    Ready,
    Unavailable,
}

impl VideoFeedbackState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warming => "warming",
            Self::Ready => "ready",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Clone, Debug)]
pub enum TransportCapabilityError {
    FeedbackUnavailable { detail: String },
    TransportNotReady { detail: String },
    SendFailed { detail: String },
}

impl std::fmt::Display for TransportCapabilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FeedbackUnavailable { detail } => write!(f, "feedback unavailable: {detail}"),
            Self::TransportNotReady { detail } => write!(f, "transport not ready: {detail}"),
            Self::SendFailed { detail } => write!(f, "send failed: {detail}"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyframeRequestKind {
    Pli,
    Fir,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyframeSendOutcome {
    Sent,
    FeedbackWarming,
    FeedbackUnavailable,
    TransportNotReady,
}
